// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Recovering an account from its recovery phrase (born-v3): the recovery
//! screens of the entry page.
//!
//! ```text
//! intro -> check the fingerprint, type the 24 words and the passphrase -> done
//!          (cancel is always there; five minutes in all)
//! ```
//!
//! A recovery opens only from the user's explicit action on an account of
//! the entry page ("Recover with recovery phrase…"), and sends nothing
//! secret until the user has typed the fingerprint the daemon named. Nothing
//! the network says can open it: the panel starts no request by itself, and
//! the store's recovery record is read only after the panel was opened.
//!
//! What the user types never stays:
//!
//! - The phrase and the passphrase live in masked inputs. On the one submit
//!   they leave as [`SecretInput`]s (the phrase resolved to list words and
//!   joined by single spaces), the inputs are cleared at once, and the
//!   secrets are zeroized when the source is done with them.
//! - Cancel, expiry, failure, close and drop clear every input.
//!
//! Limits (see the delivery notes): the input widget hands out its text as a
//! copy once per frame while the user types (the status line under the
//! phrase counts the words), and those copies are not zeroized; they stop
//! when the inputs are cleared. The platform accessibility tree can expose a
//! masked input's value to assistive technology the user has enabled.
//!
//! - `render`: the screens.

mod render;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable as _, Subscription, Window,
};
use ramflux_desktop_core::{
    AccountId, IntentKind, RecoveryCeremony, RecoveryPhase, SecretInput, Timestamp, check_phrase,
    methods, normalize_phrase,
};

use crate::actions::CeremonyEscape;
use crate::app::{StoreEvent, StoreHandle};
use crate::entry::abandon::{Abandon, Arrival, Next};
use crate::i18n;
use crate::root::IntentRoot;

/// What the panel tells the entry page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryEvent {
    /// The user left the recovery (done, closed or backed out).
    Closed,
}

/// A request sent to the daemon whose answer has not arrived. While one is
/// pending every control that sends a request is disabled and says so, and
/// the methods refuse a second send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryPending {
    /// `account.recover.begin`.
    Begin,
    /// `account.recover.confirm`.
    Confirm,
    /// `account.recover.cancel`.
    Cancel,
}

impl RecoveryPending {
    /// The contract method whose answer is awaited: only its failure ends
    /// the wait.
    pub fn method(self) -> &'static str {
        match self {
            Self::Begin => methods::RECOVER_BEGIN,
            Self::Confirm => methods::RECOVER_CONFIRM,
            Self::Cancel => methods::RECOVER_CANCEL,
        }
    }
}

/// The screens, for moving focus when the screen changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Screen {
    Intro,
    Check,
    Checking,
    Done,
    Ended,
}

/// The recovery screens of the entry page.
pub struct RecoveryPanel {
    store: Entity<StoreHandle>,
    root: Entity<IntentRoot>,
    /// The account being recovered; set when the user asked for it.
    account: Option<AccountId>,
    fingerprint: Entity<InputState>,
    phrase: Entity<InputState>,
    passphrase: Entity<InputState>,
    /// The store's recovery revision when the panel was opened: older
    /// records are an earlier recovery's, not this one's.
    baseline: u64,
    seen_revision: u64,
    pending: Option<RecoveryPending>,
    /// The failed-effect count when the request was sent.
    failures_mark: u64,
    /// The five minutes passed while the recovery ran.
    expired: bool,
    /// A begin the user backed out of while it was in flight (see
    /// [`Abandon`]).
    abandon: Option<Abandon>,
    /// The id of a recovery this panel started, tried to let go of, and
    /// could not (its cancel failed): the panel's own to show and cancel
    /// again when it opens next. No other running recovery is.
    left_running: Option<String>,
    seen_second: Option<u64>,
    focus: FocusHandle,
    heading_focus: FocusHandle,
    status_focus: FocusHandle,
    alert_focus: FocusHandle,
    focused_screen: Option<Screen>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<RecoveryEvent> for RecoveryPanel {}

impl RecoveryPanel {
    /// A panel over `store`, submitting through `root`.
    pub fn new(
        store: Entity<StoreHandle>,
        root: Entity<IntentRoot>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let fingerprint =
            i18n::localized_input("recovery.fingerprint.placeholder", window, cx, |input| input);
        let phrase = i18n::localized_input("recovery.phrase.placeholder", window, cx, |input| {
            input.masked(true)
        });
        let passphrase =
            i18n::localized_input("recovery.passphrase.placeholder", window, cx, |input| {
                input.masked(true)
            });
        let mut subscriptions = vec![
            cx.subscribe_in(&store, window, |this, _, event: &StoreEvent, window, cx| {
                if matches!(event, StoreEvent::Changed(_)) {
                    this.sync(window, cx);
                }
            }),
            cx.observe(&root, |_, _, cx| cx.notify()),
            cx.subscribe_in(&passphrase, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.recover(window, cx);
                }
            }),
        ];
        for input in [&fingerprint, &phrase, &passphrase] {
            subscriptions.push(cx.subscribe(input, |_, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }));
        }
        let revision = store.read(cx).entry_store().recovery_revision();
        Self {
            store,
            root,
            account: None,
            fingerprint,
            phrase,
            passphrase,
            baseline: revision,
            seen_revision: revision,
            pending: None,
            failures_mark: 0,
            expired: false,
            abandon: None,
            left_running: None,
            seen_second: None,
            focus: cx.focus_handle(),
            heading_focus: cx.focus_handle(),
            status_focus: cx.focus_handle(),
            alert_focus: cx.focus_handle(),
            focused_screen: None,
            _subscriptions: subscriptions,
        }
    }

    /// The account being recovered, once the user asked for it.
    pub fn account(&self) -> Option<&AccountId> {
        self.account.as_ref()
    }

    /// The request whose answer is awaited.
    pub fn pending(&self) -> Option<RecoveryPending> {
        self.pending
    }

    /// Whether the five minutes passed while the recovery ran.
    pub fn expired_seen(&self) -> bool {
        self.expired
    }

    /// Whether an abandoned begin is still being let go of.
    pub fn is_abandoned(&self) -> bool {
        self.abandon.is_some()
    }

    /// The fingerprint, phrase and passphrase inputs.
    pub fn inputs(&self) -> [&Entity<InputState>; 3] {
        [&self.fingerprint, &self.phrase, &self.passphrase]
    }

    /// Whether any input holds text (read from the inputs' own buffers).
    pub fn holds_typed(&self, cx: &App) -> bool {
        self.inputs().into_iter().any(|input| input.read(cx).text().len() > 0)
    }

    /// The id of the element that has keyboard focus among the panel's own
    /// focus targets (inputs and the containers a reader lands on), if any.
    /// A target draws the theme's focus ring while it has focus.
    pub fn focus_target(&self, window: &Window, cx: &App) -> Option<&'static str> {
        let inputs = [
            (&self.fingerprint, "recovery-fingerprint-input"),
            (&self.phrase, "recovery-phrase-input"),
            (&self.passphrase, "recovery-passphrase-input"),
        ];
        if let Some((_, id)) =
            inputs.into_iter().find(|(input, _)| input.read(cx).focus_handle(cx).is_focused(window))
        {
            return Some(id);
        }
        [
            (&self.heading_focus, "recovery-heading"),
            (&self.status_focus, "recovery-checking-text"),
            (&self.alert_focus, "recovery-ended-text"),
        ]
        .into_iter()
        .find(|(handle, _)| handle.is_focused(window))
        .map(|(_, id)| id)
    }

    /// Whether keyboard focus is inside the panel.
    pub fn has_focus(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx)
    }

    /// Opens the recovery of `account` at its first screen. Only the user's
    /// own action calls this; nothing is sent yet.
    pub fn open(&mut self, account: AccountId, window: &mut Window, cx: &mut Context<Self>) {
        self.account = Some(account);
        self.expired = false;
        self.pending = None;
        self.seen_second = None;
        self.focused_screen = None;
        let store = self.store.read(cx).entry_store();
        // A recovery still running (a cancel that failed left it) is this
        // panel's to show and cancel again, if it is this account's.
        let running = store.recovery().is_some_and(|record| {
            record.is_active()
                && Some(&record.account_id) == self.account.as_ref()
                && self.left_running.as_deref() == Some(record.ceremony_id.as_str())
        });
        self.left_running = None;
        self.baseline = store.recovery_revision().saturating_sub(u64::from(running));
        self.seen_revision = store.recovery_revision();
        self.clear_inputs(window, cx);
        cx.notify();
    }

    /// The recovery of this panel, if one was reported since it opened.
    fn record(&self, cx: &App) -> Option<RecoveryCeremony> {
        if self.abandon.is_some() {
            return None;
        }
        let store = self.store.read(cx).entry_store();
        store
            .recovery()
            .filter(|_| store.recovery_revision() > self.baseline)
            .filter(|record| Some(&record.account_id) == self.account.as_ref())
            .cloned()
    }

    fn now(&self, cx: &App) -> Timestamp {
        self.store.read(cx).session().map(|session| session.now()).unwrap_or_default()
    }

    fn clear_inputs(&self, window: &mut Window, cx: &mut Context<Self>) {
        for input in self.inputs() {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    /// Sends `kind` and awaits its answer as `pending`. Refused while an
    /// answer is awaited (only cancel may follow, unless cancel is awaited).
    fn submit(
        &mut self,
        kind: IntentKind,
        pending: RecoveryPending,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let blocked = match self.pending {
            None => false,
            Some(awaited) => {
                pending != RecoveryPending::Cancel || awaited == RecoveryPending::Cancel
            }
        };
        if blocked {
            return;
        }
        self.pending = Some(pending);
        self.failures_mark = self.store.read(cx).entry_store().failures_total();
        let root = self.root.clone();
        let result = root.update(cx, |root, cx| root.submit_secret(kind, window, cx));
        // Policy refused it: no answer is coming.
        if !result.starts_with("submitted") {
            self.pending = None;
        }
        cx.notify();
    }

    /// Follows the store: notes the daemon's answers, clears what was typed
    /// once the recovery is not waiting for it, and ends a recovery whose
    /// five minutes are over.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let store = self.store.read(cx).entry_store();
        let revision = store.recovery_revision();
        let answered = revision != self.seen_revision;
        let own_failed = self.pending.is_some_and(|pending| {
            store.failures_since(self.failures_mark).iter().any(|f| f.method == pending.method())
        });
        if answered || own_failed {
            self.pending = None;
        }
        self.seen_revision = revision;
        if self.abandon.is_some() {
            self.settle_abandoned(answered, window, cx);
            cx.notify();
            return;
        }
        match self.record(cx) {
            Some(record) if record.is_active() => {
                let now = self.now(cx);
                if record.expired_at(now) && !self.expired {
                    self.expire(window, cx);
                }
                let second = record.remaining_ms(now) / 1_000;
                if self.seen_second != Some(second) {
                    self.seen_second = Some(second);
                    cx.notify();
                }
            }
            // Not waiting for the phrase any more (or never was): nothing
            // typed is kept.
            _ if self.holds_typed(cx) => self.clear_inputs(window, cx),
            _ => {}
        }
        cx.notify();
    }

    /// The recovery a Back or Escape left behind while its begin was in
    /// flight (see [`Abandon`]).
    fn settle_abandoned(&mut self, answered: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(abandon) = self.abandon else { return };
        let store = self.store.read(cx).entry_store();
        let revision = store.recovery_revision();
        let failed_since = |mark: u64, method: &str| {
            store.failures_since(mark).iter().any(|failure| failure.method == method)
        };
        let arrival = Arrival {
            running: store.recovery().is_some_and(RecoveryCeremony::is_active),
            answered,
            idle: self.pending.is_none(),
            begin_failed: failed_since(abandon.begin_mark, methods::RECOVER_BEGIN),
            cancel_failed: abandon
                .cancel_mark
                .is_some_and(|mark| failed_since(mark, methods::RECOVER_CANCEL)),
        };
        match abandon.next(arrival) {
            Next::Wait => {}
            Next::Cancel => {
                self.submit(IntentKind::CancelRecovery, RecoveryPending::Cancel, window, cx);
                let mark = self.failures_mark;
                if let Some(abandon) = &mut self.abandon {
                    abandon.cancel_mark = Some(mark);
                }
            }
            Next::Release => {
                self.abandon = None;
                self.baseline = revision;
                self.pending = None;
            }
            Next::ReleaseShowing => {
                self.left_running = self
                    .store
                    .read(cx)
                    .store()
                    .recovery()
                    .filter(|record| record.is_active())
                    .map(|record| record.ceremony_id.clone());
                self.abandon = None;
                self.pending = None;
            }
        }
    }

    /// The five minutes are over: nothing is kept, the screen says so, and
    /// the daemon is told to let go.
    fn expire(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.expired = true;
        self.clear_inputs(window, cx);
        self.pending = None;
        self.submit(IntentKind::CancelRecovery, RecoveryPending::Cancel, window, cx);
    }

    /// "Continue" on the first screen: asks the daemon for the account's
    /// fingerprint. Sends no secret.
    pub fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(account_id) = self.account.clone() else { return };
        if self.pending.is_some() || self.abandon.is_some() {
            return;
        }
        self.expired = false;
        self.seen_second = None;
        self.submit(IntentKind::BeginRecovery { account_id }, RecoveryPending::Begin, window, cx);
    }

    /// Whether the typed text is the fingerprint the daemon named (white
    /// space between its groups does not matter).
    pub fn fingerprint_matches(&self, cx: &App) -> bool {
        let Some(record) = self.record(cx) else { return false };
        let typed = self.fingerprint.read(cx).value();
        !record.anchor.is_empty()
            && typed.split_whitespace().flat_map(str::chars).eq(record.anchor.chars())
    }

    /// Whether everything is typed: the fingerprint, 24 recovery words and a
    /// passphrase (the one attempt is not spent on what cannot be right).
    pub fn can_recover(&self, cx: &App) -> bool {
        self.pending.is_none()
            && self.fingerprint_matches(cx)
            && check_phrase(self.phrase.read(cx).value().as_ref()).complete()
            && self.passphrase.read(cx).text().len() > 0
    }

    /// Sends the fingerprint, the words and the passphrase (the one
    /// attempt) and clears everything typed.
    pub fn recover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_recover(cx) {
            return;
        }
        let Some(record) = self.record(cx).filter(|r| matches!(r.phase, RecoveryPhase::Prepared))
        else {
            return;
        };
        let Ok(words) = normalize_phrase(self.phrase.read(cx).value().as_ref()) else { return };
        let phrase = SecretInput::new(words.to_string());
        drop(words);
        let passphrase = SecretInput::new(self.passphrase.read(cx).value().to_string());
        self.clear_inputs(window, cx);
        self.submit(
            IntentKind::ConfirmRecovery { anchor: record.anchor, phrase, passphrase },
            RecoveryPending::Confirm,
            window,
            cx,
        );
    }

    /// Cancels the recovery (or backs out of the first screen): nothing
    /// typed is kept, and the daemon erases its copy.
    pub fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_inputs(window, cx);
        if self.pending == Some(RecoveryPending::Begin) {
            // The daemon has not said whether a recovery exists yet: let go
            // of it when it does.
            self.abandon = Some(Abandon { begin_mark: self.failures_mark, cancel_mark: None });
            self.close(window, cx);
        } else if self.record(cx).is_some_and(|record| record.is_active()) {
            self.submit(IntentKind::CancelRecovery, RecoveryPending::Cancel, window, cx);
        } else {
            self.close(window, cx);
        }
    }

    /// The window is closing: the daemon is told to let go, best effort
    /// (it also lets go when the connection drops).
    pub fn cancel_for_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_inputs(window, cx);
        if self.record(cx).is_some_and(|record| record.is_active()) {
            self.pending = None;
            self.submit(IntentKind::CancelRecovery, RecoveryPending::Cancel, window, cx);
        }
    }

    /// Escape: leave a finished recovery, cancel a running one.
    fn on_escape(&mut self, _: &CeremonyEscape, window: &mut Window, cx: &mut Context<Self>) {
        match self.record(cx) {
            Some(record) if !record.is_active() => self.close(window, cx),
            _ if self.pending == Some(RecoveryPending::Confirm) => {}
            _ => self.cancel(window, cx),
        }
    }

    /// Leaves the recovery screens.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_inputs(window, cx);
        cx.emit(RecoveryEvent::Closed);
        cx.notify();
    }

    /// Leaves an ended recovery and starts at the first screen again.
    pub fn start_over(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(account) = self.account.clone() {
            self.open(account, window, cx);
        }
    }

    /// Whether the recovered account is on screen, so the entry page keeps
    /// it in view even once the account is unlocked.
    pub fn shows_result(&self, cx: &App) -> bool {
        self.record(cx).is_some_and(|r| matches!(r.phase, RecoveryPhase::Recovered(_)))
    }
}

impl Drop for RecoveryPanel {
    fn drop(&mut self) {
        // The inputs drop with the panel; nothing else holds a secret.
        self.abandon = None;
    }
}
