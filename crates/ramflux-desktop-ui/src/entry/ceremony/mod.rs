// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Account creation with a recovery phrase (born-v3): the ceremony screens
//! of the entry page.
//!
//! ```text
//! name -> get ready -> the 24 words, once -> type three words back -> done
//!                      (cancel is always there; five minutes in all)
//! ```
//!
//! The ceremony itself (phase, expiry, which words are asked) is in the
//! store. This panel holds only what the user is looking at (view-local,
//! R4) and the one secret that travels to the screen:
//!
//! - The recovery phrase arrives through [`StoreHandle::take_recovery_phrase`]
//!   when the store reports `PhraseReady`, and is held in [`Held`]. It is
//!   erased the moment the user hides it (before the daemon is asked for the
//!   words), when the ceremony ends or is cancelled, when it expires, and
//!   when the panel is dropped. No other field, event or store row has the
//!   words; [`Held`] is the only place to look.
//! - While the window is not active the words are covered: not drawn at
//!   all, and shown again only on request while the window is active.
//! - The words typed back leave their inputs as [`SecretInput`]s, resolved
//!   to list words, and the inputs are cleared at once.
//!
//! Limits (see the delivery notes): GPUI Kit 0.8.0 has no way to exclude a
//! window from screenshots or screen sharing, so the screen asks the user
//! to stop them first and cannot enforce it. Words drawn on screen are
//! copied into GPUI text elements for the frames they are visible; those
//! copies, and GPUI's text layout caches, are outside this panel's control
//! and are not zeroized. They stop being refreshed the frame the words stop
//! being drawn.
//!
//! - `render`: the screens.

mod render;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{App, Context, Entity, EventEmitter, FocusHandle, Subscription, Window};
use ramflux_desktop_core::{
    Ceremony, CeremonyPhase, IntentKind, RecoveryPhrase, SecretInput, Timestamp, WordInput,
    resolve_word,
};

use crate::actions::CeremonyEscape;
use crate::app::{StoreEvent, StoreHandle};
use crate::i18n;
use crate::root::IntentRoot;
use crate::settings::backup::text::Strength;

/// What the panel tells the entry page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CeremonyEvent {
    /// The user left the ceremony (done, closed or backed out).
    Closed,
}

/// What the screen holds of the recovery phrase.
///
/// The only place in the UI where the words live. Not `Clone`, no `Debug`
/// of the words; the phrase's buffer is zeroized when it is dropped.
pub struct Held {
    phrase: Option<RecoveryPhrase>,
    /// The words are not drawn (the window lost focus).
    covered: bool,
    /// The phrase was taken from the source for this ceremony: the source
    /// gives it out once, so a missing phrase is not asked for again.
    taken: bool,
}

impl Held {
    fn new() -> Self {
        Self { phrase: None, covered: false, taken: false }
    }

    /// Whether the screen holds the words.
    pub fn holds_phrase(&self) -> bool {
        self.phrase.is_some()
    }

    /// How many words the screen holds (0 once hidden).
    pub fn words(&self) -> usize {
        self.phrase.as_ref().map_or(0, |phrase| phrase.words().count())
    }

    /// Whether the held words are covered (not drawn).
    pub fn is_covered(&self) -> bool {
        self.covered
    }

    /// Drops the phrase, which zeroizes it.
    fn erase(&mut self) {
        self.phrase = None;
        self.covered = false;
    }
}

impl std::fmt::Debug for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Held")
            .field("words", &self.words())
            .field("covered", &self.covered)
            .field("taken", &self.taken)
            .finish()
    }
}

/// A request sent to the daemon whose answer has not arrived. While one is
/// pending every control that sends a request is disabled and says so, and
/// the methods refuse a second send (a second press of "Show recovery
/// phrase" would end the ceremony).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    /// `account.create`.
    Begin,
    /// `show_once`.
    Show,
    /// `hidden_challenge`.
    Hide,
    /// `confirm`.
    Confirm,
    /// `cancel`.
    Cancel,
}

/// The screens, for moving focus when the screen changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Screen {
    Name,
    Ready,
    Phrase,
    Hiding,
    Words,
    Passphrase,
    Done,
    Ended,
}

/// The ceremony screens of the entry page.
pub struct CeremonyPanel {
    store: Entity<StoreHandle>,
    root: Entity<IntentRoot>,
    name: Entity<InputState>,
    words: [Entity<InputState>; 3],
    passphrase: Entity<InputState>,
    repeat: Entity<InputState>,
    held: Held,
    /// The three words, once typed and before the passphrase step ends in
    /// the one confirm. Secrets: zeroized on drop, dropped on cancel,
    /// expiry, failure and close.
    typed: Option<Vec<SecretInput>>,
    /// The store's ceremony revision when the panel was opened: older
    /// records are an earlier ceremony's, not this one's.
    baseline: u64,
    /// The last revision seen: a newer one is the daemon's answer.
    seen_revision: u64,
    /// The request whose answer is awaited.
    pending: Option<Pending>,
    /// The failed-effect count when it was sent: a newer failure is its
    /// answer too.
    failures_mark: u64,
    /// The five minutes passed while the ceremony ran: the expiry stays in
    /// view even after the ceremony was cancelled to free the daemon.
    expired: bool,
    /// Escape was pressed: the screen asks whether to cancel.
    confirming_cancel: bool,
    /// The whole second last drawn in the countdown.
    seen_second: Option<u64>,
    focus: FocusHandle,
    focused_screen: Option<Screen>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<CeremonyEvent> for CeremonyPanel {}

impl CeremonyPanel {
    /// A panel over `store`, submitting through `root`.
    pub fn new(
        store: Entity<StoreHandle>,
        root: Entity<IntentRoot>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = i18n::localized_input("ceremony.name.placeholder", window, cx, |input| input);
        let word = |window: &mut Window, cx: &mut App| {
            i18n::localized_input("ceremony.confirm.placeholder", window, cx, |input| input)
        };
        let words = [word(window, cx), word(window, cx), word(window, cx)];
        let secret = |key: &'static str, window: &mut Window, cx: &mut App| {
            i18n::localized_input(key, window, cx, |input| input.masked(true))
        };
        let passphrase = secret("ceremony.passphrase.label", window, cx);
        let repeat = secret("ceremony.passphrase.repeat", window, cx);
        let panel = cx.entity().downgrade();
        let mut subscriptions = vec![
            cx.subscribe_in(&store, window, |this, _, event: &StoreEvent, window, cx| {
                if matches!(event, StoreEvent::Changed(_)) {
                    this.sync(window, cx);
                }
            }),
            cx.observe(&root, |_, _, cx| cx.notify()),
            // Cover the words the moment the window loses focus.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() && this.held.phrase.is_some() && !this.held.covered {
                    this.held.covered = true;
                    cx.notify();
                }
            }),
            cx.subscribe_in(&name, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.begin(window, cx);
                }
            }),
            cx.subscribe_in(&words[2], window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.next(window, cx);
                }
            }),
            cx.subscribe_in(&repeat, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.confirm(window, cx);
                }
            }),
        ];
        for input in words.iter().chain([&passphrase, &repeat]) {
            subscriptions.push(cx.subscribe(input, |_, _, event, cx| redraw_on_change(event, cx)));
        }
        // Closing the window lets the daemon go of the ceremony at once
        // (it also does when the connection drops).
        window.on_window_should_close(cx, move |window, cx| {
            panel.update(cx, |panel, cx| panel.cancel_for_close(window, cx)).ok();
            true
        });
        let revision = store.read(cx).store().ceremony_revision();
        Self {
            store,
            root,
            name,
            words,
            passphrase,
            repeat,
            held: Held::new(),
            typed: None,
            baseline: revision,
            seen_revision: revision,
            pending: None,
            failures_mark: 0,
            expired: false,
            confirming_cancel: false,
            seen_second: None,
            focus: cx.focus_handle(),
            focused_screen: None,
            _subscriptions: subscriptions,
        }
    }

    /// What the screen holds of the recovery phrase.
    pub fn held(&self) -> &Held {
        &self.held
    }

    /// Whether the three words are held, waiting for the passphrase.
    pub fn holds_typed_words(&self) -> bool {
        self.typed.is_some()
    }

    /// The request whose answer is awaited.
    pub fn pending(&self) -> Option<Pending> {
        self.pending
    }

    /// Whether the five minutes passed while the ceremony ran.
    pub fn expired_seen(&self) -> bool {
        self.expired
    }

    /// The account name input.
    pub fn name_input(&self) -> &Entity<InputState> {
        &self.name
    }

    /// The three word inputs of the confirmation.
    pub fn word_inputs(&self) -> &[Entity<InputState>; 3] {
        &self.words
    }

    /// The passphrase and repeat inputs.
    pub fn passphrase_inputs(&self) -> [&Entity<InputState>; 2] {
        [&self.passphrase, &self.repeat]
    }

    /// Whether keyboard focus is inside the panel.
    pub fn has_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx)
    }

    /// Starts afresh: the store's record of an earlier ceremony is not this
    /// one's, and nothing from before is held.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.held = Held::new();
        self.typed = None;
        self.expired = false;
        self.pending = None;
        self.confirming_cancel = false;
        self.seen_second = None;
        self.focused_screen = None;
        self.baseline = self.store.read(cx).store().ceremony_revision();
        self.seen_revision = self.baseline;
        self.clear_inputs(window, cx);
        cx.notify();
    }

    /// The ceremony of this panel, if one was reported since it opened.
    fn ceremony(&self, cx: &App) -> Option<Ceremony> {
        let store = self.store.read(cx).store();
        store.ceremony().filter(|_| store.ceremony_revision() > self.baseline).cloned()
    }

    /// The time the ceremony is measured against.
    fn now(&self, cx: &App) -> Timestamp {
        self.store.read(cx).session().map(|session| session.now()).unwrap_or_default()
    }

    fn clear_inputs(&self, window: &mut Window, cx: &mut Context<Self>) {
        let all =
            std::iter::once(&self.name).chain(&self.words).chain([&self.passphrase, &self.repeat]);
        for input in all {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    /// Drops every secret the panel holds: the phrase, the typed words and
    /// what is in the inputs.
    fn wipe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.held.erase();
        self.typed = None;
        self.clear_inputs(window, cx);
    }

    /// Sends `kind` and awaits its answer as `pending`. Refused while an
    /// answer is awaited (only cancel may follow, unless cancel is awaited).
    fn submit(
        &mut self,
        kind: IntentKind,
        pending: Pending,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let blocked = match self.pending {
            None => false,
            Some(awaited) => pending != Pending::Cancel || awaited == Pending::Cancel,
        };
        if blocked {
            return;
        }
        self.pending = Some(pending);
        self.failures_mark = self.store.read(cx).store().failures_total();
        let root = self.root.clone();
        let result = root.update(cx, |root, cx| root.submit_secret(kind, window, cx));
        // Policy refused it: no answer is coming.
        if !result.starts_with("submitted") {
            self.pending = None;
        }
        cx.notify();
    }

    /// Follows the store: takes the phrase when the daemon hands it out,
    /// drops it as soon as the ceremony is not showing it, ends a ceremony
    /// whose five minutes are over, and notes the daemon's answers.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let store = self.store.read(cx).store();
        let revision = store.ceremony_revision();
        let failures = store.failures_total();
        if revision != self.seen_revision || failures != self.failures_mark {
            self.pending = None;
        }
        self.seen_revision = revision;
        let ceremony = self.ceremony(cx);
        match ceremony.as_ref().map(|c| &c.phase) {
            Some(CeremonyPhase::PhraseReady) => {
                if !self.held.taken {
                    self.held.taken = true;
                    self.held.phrase =
                        self.store.update(cx, |handle, _| handle.take_recovery_phrase());
                    self.held.covered = !window.is_window_active();
                }
            }
            // Whatever else the ceremony is, the words are not on screen.
            _ => self.held.erase(),
        }
        // The typed words and passphrase live only while the confirmation
        // is being typed.
        let confirming =
            matches!(ceremony.as_ref().map(|c| &c.phase), Some(CeremonyPhase::Challenged { .. }));
        if !confirming && (self.typed.is_some() || self.secrets_typed(cx)) {
            self.typed = None;
            self.clear_inputs(window, cx);
        }
        if let Some(ceremony) = ceremony.filter(Ceremony::is_active) {
            let now = self.now(cx);
            if ceremony.expired_at(now) && !self.expired {
                self.expire(window, cx);
            }
            let second = ceremony.remaining_ms(now) / 1_000;
            if self.seen_second != Some(second) {
                self.seen_second = Some(second);
                cx.notify();
            }
        } else {
            self.confirming_cancel = false;
        }
        cx.notify();
    }

    /// Whether a word or passphrase input has text.
    fn secrets_typed(&self, cx: &App) -> bool {
        self.words
            .iter()
            .chain([&self.passphrase, &self.repeat])
            .any(|input| !input.read(cx).value().is_empty())
    }

    /// The five minutes are over: nothing is held, the screen says so, and
    /// the daemon is told to let go of the ceremony.
    fn expire(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.expired = true;
        self.wipe(window, cx);
        self.pending = None;
        self.submit(IntentKind::CancelCeremony, Pending::Cancel, window, cx);
    }

    /// Asks the daemon to prepare an account called what the user typed.
    pub fn begin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let label = self.name.read(cx).value().trim().to_owned();
        if label.is_empty() || self.pending.is_some() {
            return;
        }
        self.held = Held::new();
        self.expired = false;
        self.seen_second = None;
        self.submit(IntentKind::BeginCeremony { label }, Pending::Begin, window, cx);
    }

    /// Asks for the recovery phrase, once.
    pub fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.submit(IntentKind::ShowRecoveryPhrase, Pending::Show, window, cx);
    }

    /// Shows the words again after they were covered. Only while the
    /// window is active.
    pub fn reveal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.is_window_active() && self.held.covered {
            self.held.covered = false;
            cx.notify();
        }
    }

    /// "I wrote them down": the words are erased from the screen first,
    /// and only then does the daemon choose the words to ask.
    pub fn hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        self.held.erase();
        self.submit(IntentKind::HideRecoveryPhrase, Pending::Hide, window, cx);
    }

    /// The typed words as list words, once all three name one.
    fn resolved_words(&self, cx: &App) -> Option<[&'static str; 3]> {
        let mut resolved = [""; 3];
        for (slot, input) in resolved.iter_mut().zip(&self.words) {
            *slot = resolve_word(input.read(cx).value().as_ref()).resolved()?;
        }
        Some(resolved)
    }

    /// Whether all three typed words name a list word (the one attempt is
    /// not spent on a word that cannot be right).
    pub fn can_continue(&self, cx: &App) -> bool {
        self.resolved_words(cx).is_some()
    }

    /// The three words are typed: keep them as secrets and ask for the
    /// passphrase. Nothing is sent yet.
    pub fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(resolved) = self.resolved_words(cx) else { return };
        if self.typed.is_some() || self.pending.is_some() {
            return;
        }
        self.typed = Some(resolved.iter().map(|word| SecretInput::new(*word)).collect());
        for input in &self.words {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        cx.notify();
    }

    /// The passphrase is acceptable: strong enough and typed twice.
    pub fn can_confirm(&self, cx: &App) -> bool {
        let typed = self.passphrase.read(cx).value();
        Strength::of(&typed) != Strength::TooShort && typed == self.repeat.read(cx).value()
    }

    /// Sends the words and the passphrase (the one attempt) and clears
    /// everything typed.
    pub fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_confirm(cx) || self.pending.is_some() {
            return;
        }
        let Some(words) = self.typed.take() else { return };
        let passphrase = SecretInput::new(self.passphrase.read(cx).value().to_string());
        self.clear_inputs(window, cx);
        self.submit(
            IntentKind::ConfirmCeremony { words, passphrase },
            Pending::Confirm,
            window,
            cx,
        );
    }

    /// Cancels the ceremony (or backs out of the name step): nothing is
    /// held afterwards, and the daemon erases its copy.
    pub fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirming_cancel = false;
        self.wipe(window, cx);
        if self.ceremony(cx).is_some_and(|c| c.is_active()) {
            self.submit(IntentKind::CancelCeremony, Pending::Cancel, window, cx);
        } else {
            self.close(window, cx);
        }
    }

    /// The window is closing: the daemon is told to let go, best effort.
    fn cancel_for_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.wipe(window, cx);
        if self.ceremony(cx).is_some_and(|c| c.is_active()) {
            self.pending = None;
            self.submit(IntentKind::CancelCeremony, Pending::Cancel, window, cx);
        }
    }

    /// Escape: ask whether to cancel a running setup; leave a finished one.
    fn on_escape(&mut self, _: &CeremonyEscape, window: &mut Window, cx: &mut Context<Self>) {
        match self.ceremony(cx) {
            Some(ceremony) if ceremony.is_active() && !self.expired => {
                self.confirming_cancel = !self.confirming_cancel;
                cx.notify();
            }
            Some(_) => self.close(window, cx),
            None => self.cancel(window, cx),
        }
    }

    /// Dismisses the question without cancelling.
    pub fn keep_going(&mut self, cx: &mut Context<Self>) {
        self.confirming_cancel = false;
        cx.notify();
    }

    /// Leaves the ceremony screens.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirming_cancel = false;
        self.wipe(window, cx);
        cx.emit(CeremonyEvent::Closed);
        cx.notify();
    }

    /// Leaves an ended ceremony and starts the name step again.
    pub fn start_over(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open(window, cx);
    }

    /// Whether the account the ceremony made is on screen, so the entry
    /// page keeps it in view even once the account is listed.
    pub fn shows_result(&self, cx: &App) -> bool {
        self.ceremony(cx).is_some_and(|c| matches!(c.phase, CeremonyPhase::Created(_)))
    }
}

/// A typed word changes what the hint under it says.
fn redraw_on_change<T: 'static>(event: &InputEvent, cx: &mut Context<T>) {
    if matches!(event, InputEvent::Change) {
        cx.notify();
    }
}

impl Drop for CeremonyPanel {
    fn drop(&mut self) {
        self.held.erase();
        self.typed = None;
    }
}

/// What a typed word says, for the hint under its input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Hint {
    /// Nothing typed.
    Nothing,
    /// The word it stands for.
    Reads(&'static str),
    /// Candidates to pick from while it is being typed.
    Suggest(Vec<&'static str>),
    /// Not a recovery word.
    Unknown,
    /// More than one word.
    NotOneWord,
}

impl Hint {
    /// The hint for what is typed.
    pub(crate) fn of(typed: &str) -> Self {
        match resolve_word(typed) {
            WordInput::Empty => Self::Nothing,
            WordInput::Word(word) | WordInput::Prefix(word) => Self::Reads(word),
            WordInput::Incomplete => {
                let candidates = ramflux_desktop_core::suggestions(typed);
                if candidates.is_empty() { Self::Nothing } else { Self::Suggest(candidates) }
            }
            WordInput::Unknown => Self::Unknown,
            WordInput::NotOneWord => Self::NotOneWord,
        }
    }
}
