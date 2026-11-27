// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! One account's store and source, with a scenario clock.
//!
//! This is the only code in the application that calls [`Store::apply`] and
//! [`Source::submit`] (rules R1 and R2). The GPUI [`super::StoreHandle`] owns
//! one session per account and drives it; views never reach a session.
//!
//! A session is backed either by a scenario-playing mock (with the
//! developer controls: pause, step, seek) or by a live source (the SDK
//! source over a real daemon), which follows the wall clock and ignores the
//! scenario controls.
//!
//! Card runtime facts are applied to the store like any other fact; the
//! bus steps the store asks for (a re-read, a submit, a pool suspend or
//! dispose) are dispatched here as intents, and a link the host confirmed
//! names the URL the app opens.

use std::collections::BTreeSet;

use ramflux_desktop_core::{
    CARD_CHANGED_REASON, CardDue, CardEvent, CardInput, CardSeen, ContextId, DomainEvent, Effect,
    EventEnvelope, Intent, IntentError, IntentKind, MomentId, Source, Store, SurfaceId, Timestamp,
    handle_intent, methods,
};
#[cfg(feature = "demo")]
use ramflux_desktop_test::{MockSource, Scenario};

use super::changes::ChangeSet;
use super::inspector::InspectorEntry;
use super::names::{effect_label, event_name, intent_name};

/// Card intents accepted and not yet answered.
#[derive(Debug, Default)]
pub struct CardMarks {
    /// Surfaces whose session open was asked.
    opening: BTreeSet<SurfaceId>,
    /// The locale and theme each card was last presented in.
    presented: std::collections::BTreeMap<SurfaceId, (String, bool)>,
    /// Surfaces whose session release was asked.
    disposing: BTreeSet<SurfaceId>,
    /// Surfaces whose open was refused or fell back, with the surface
    /// version it happened at: not asked again until the version changes or
    /// the surface changes again (a resync included).
    open_refused: std::collections::BTreeMap<SurfaceId, u64>,
    /// Cards with a call in flight that their host events wait for.
    busy: std::collections::BTreeMap<SurfaceId, InFlight>,
    /// Host events waiting for the call in flight on their card, in order
    /// (one consumer per card: the oldest is sent first).
    queued: std::collections::BTreeMap<SurfaceId, Vec<QueuedEvent>>,
    /// Card steps held while the app lock is closed (a re-read or a
    /// submit); they are dispatched once it opens.
    held_due: Vec<CardDue>,
}

/// A call in flight on a card that its host events wait for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InFlight {
    /// A host event call (`card.session.event`); `input` when it carries a
    /// text draft. Settled by [`CardEvent::EventSettled`], whatever the reply.
    Event {
        /// The event is a text draft.
        input: bool,
    },
    /// The resume of a suspended card, asked for its waiting host events.
    Resume,
}

/// A host event on a card that waits behind the call in flight on its card.
/// A text draft merges only into a draft of the same key at the tail of the
/// queue (the whole value, the newest wins); anything else is appended, so a
/// tap stays behind every draft of its press and before any later draft.
#[derive(Debug, Clone)]
pub struct QueuedEvent {
    moment_id: MomentId,
    surface_id: SurfaceId,
    key: String,
    event: CardInput,
    seen: CardSeen,
    /// The signed version of the tree the event was valid on when it found
    /// its card suspended (the resume brings a fresh session, so the event
    /// is rebased onto the resumed tree afterwards). `None`: not valid.
    pre_resume: Option<u64>,
}

impl QueuedEvent {
    /// The queued form of a host event intent, if it is one.
    fn of(kind: &IntentKind) -> Option<Self> {
        match kind {
            IntentKind::CardInteract { moment_id, surface_id, key, event, seen } => Some(Self {
                moment_id: moment_id.clone(),
                surface_id: surface_id.clone(),
                key: key.clone(),
                event: event.clone(),
                seen: seen.clone(),
                pre_resume: None,
            }),
            _ => None,
        }
    }

    fn is_input(&self) -> bool {
        matches!(self.event, CardInput::Input { .. })
    }

    fn into_intent(self) -> Intent {
        Intent::human(IntentKind::CardInteract {
            moment_id: self.moment_id,
            surface_id: self.surface_id,
            key: self.key,
            event: self.event,
            seen: self.seen,
        })
    }
}

impl CardMarks {
    /// Whether a host event for `surface` must wait (a call is in flight, or
    /// events are already waiting).
    fn holds(&self, surface: &SurfaceId) -> bool {
        self.busy.contains_key(surface) || self.queued.get(surface).is_some_and(|q| !q.is_empty())
    }

    /// Queues a host event. A draft replaces the last queued event only when
    /// that is a draft of the same key; otherwise the event is appended.
    fn enqueue(&mut self, event: QueuedEvent) {
        let queue = self.queued.entry(event.surface_id.clone()).or_default();
        if event.is_input()
            && let Some(last) = queue.last_mut()
            && last.is_input()
            && last.key == event.key
        {
            *last = event;
            return;
        }
        queue.push(event);
    }
}

/// Whether `kind` is a card intent: it is decided and dispatched by the
/// account session that holds the card (its queue and pool), also when live.
pub fn is_card_intent(kind: &IntentKind) -> bool {
    matches!(
        kind,
        IntentKind::CardOpen { .. }
            | IntentKind::CardPresent { .. }
            | IntentKind::CardInteract { .. }
            | IntentKind::CardSlotAnswer { .. }
            | IntentKind::CardSubmit { .. }
            | IntentKind::CardRefresh { .. }
            | IntentKind::CardSuspend { .. }
            | IntentKind::CardDispose { .. }
            | IntentKind::CardSuspendAll
            | IntentKind::CardResumeAll
            | IntentKind::CardRetainVisible { .. }
    )
}

/// The surface a user host event addresses (the card is acted on by a person).
fn card_user_surface(kind: &IntentKind) -> Option<&SurfaceId> {
    match kind {
        IntentKind::CardInteract { surface_id, .. }
        | IntentKind::CardSlotAnswer { surface_id, .. }
        | IntentKind::CardSubmit { surface_id, .. } => Some(surface_id),
        _ => None,
    }
}

/// Whether an effect is a host event call (`card.session.event`).
fn is_event_call(effect: &Effect) -> bool {
    match effect {
        Effect::Call { method, .. } | Effect::SecretCall { method, .. } => {
            method == methods::CARD_SESSION_EVENT
        }
        _ => false,
    }
}

/// The card state an accepted intent records (see [`card_mark`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardMark {
    /// A session open was asked for this surface.
    Open(SurfaceId),
    /// This surface was presented in this locale and theme.
    Present(SurfaceId, String, bool),
    /// This surface's session release was asked.
    Dispose(SurfaceId),
}

/// The card state `kind` records when it is accepted, if it is a card intent
/// that opens or presents a session.
pub fn card_mark(kind: &IntentKind) -> Option<CardMark> {
    match kind {
        IntentKind::CardOpen { surface_id, .. } => Some(CardMark::Open(surface_id.clone())),
        IntentKind::CardPresent { surface_id, locale, dark, .. } => {
            Some(CardMark::Present(surface_id.clone(), locale.clone(), *dark))
        }
        IntentKind::CardDispose { surface_id, .. } => Some(CardMark::Dispose(surface_id.clone())),
        _ => None,
    }
}

/// What one clock movement or step applied.
#[derive(Debug, Default)]
pub struct Applied {
    /// Merged changes.
    pub changes: ChangeSet,
    /// Inspector records for the applied envelopes.
    pub records: Vec<InspectorEntry>,
    /// Links the host confirmed: the app opens these URLs.
    pub open_urls: Vec<String>,
}

/// The policy result of a dispatched intent.
#[derive(Debug)]
pub enum DispatchOutcome {
    /// Effects went to the source; labels for display.
    Submitted(Vec<String>),
    /// Policy requires the confirm strip first. Nothing was submitted.
    NeedsConfirmation(Box<Intent>),
    /// The intent was refused.
    Refused(IntentError),
    /// There is no active account to dispatch to.
    NoAccount,
}

/// What feeds a session.
enum Backing {
    /// A scenario-playing mock with its own clock (feature `demo`).
    #[cfg(feature = "demo")]
    Mock { scenario: String, source: Box<MockSource> },
    /// A live source on the wall clock.
    Live { label: String, source: Box<dyn Source> },
}

/// A store fed by a scenario-playing or live source.
pub struct AccountSession {
    backing: Backing,
    store: Store,
    clock_ms: u64,
    live_epoch: Timestamp,
    live_started: std::time::Instant,
    monotonic_now: std::rc::Rc<dyn Fn() -> std::time::Instant>,
    time_floor: Timestamp,
    paused: bool,
    /// Message bodies asked of the source and not delivered yet (asked once).
    requested: BTreeSet<MomentId>,
    focused_context: Option<ContextId>,
    /// What the card intents accepted so far (see [`CardMarks`]).
    card_marks: std::cell::RefCell<CardMarks>,
    /// Links the host confirmed since the last publish (URLs to open).
    open_urls: Vec<String>,
    networking_deadlines: std::collections::BTreeMap<
        ramflux_desktop_core::AccountId,
        (ramflux_desktop_core::NetworkingReason, Timestamp),
    >,
}

impl std::fmt::Debug for AccountSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountSession")
            .field("source", &self.scenario_name())
            .field("last_seq", &self.store.last_seq())
            .finish_non_exhaustive()
    }
}

impl AccountSession {
    /// Creates a session at offset 0 with nothing applied.
    #[cfg(feature = "demo")]
    pub fn new(scenario: &Scenario) -> Self {
        Self::with_epoch(scenario, scenario.epoch)
    }

    /// Creates a session whose offset 0 is `epoch` instead of the scenario's own.
    #[cfg(feature = "demo")]
    pub fn with_epoch(scenario: &Scenario, epoch: Timestamp) -> Self {
        Self::start(Backing::Mock {
            scenario: scenario.name.clone(),
            source: Box::new(MockSource::with_epoch(scenario, epoch)),
        })
    }

    /// A session fed by a live `source`, named `label` (shown by the
    /// developer panel instead of a scenario name).
    pub fn live(label: impl Into<String>, source: Box<dyn Source>) -> Self {
        Self::start(Backing::Live { label: label.into(), source })
    }

    /// A live session whose monotonic time follows its owning executor.
    pub fn live_with_clock(
        label: impl Into<String>,
        source: Box<dyn Source>,
        now: impl Fn() -> std::time::Instant + 'static,
    ) -> Self {
        let mut session = Self::live(label, source);
        session.live_started = now();
        session.monotonic_now = std::rc::Rc::new(now);
        session
    }

    fn start(backing: Backing) -> Self {
        Self {
            backing,
            store: Store::new(),
            clock_ms: 0,
            live_epoch: super::wall_clock_now(),
            live_started: std::time::Instant::now(),
            monotonic_now: std::rc::Rc::new(std::time::Instant::now),
            time_floor: Timestamp(0),
            paused: false,
            requested: BTreeSet::new(),
            focused_context: None,
            card_marks: std::cell::RefCell::new(CardMarks::default()),
            open_urls: Vec::new(),
            networking_deadlines: Default::default(),
        }
    }

    /// Whether a live source feeds the session.
    pub fn is_live(&self) -> bool {
        matches!(self.backing, Backing::Live { .. })
    }

    /// Name of the scenario this session plays, or the live source's label.
    pub fn scenario_name(&self) -> &str {
        match &self.backing {
            #[cfg(feature = "demo")]
            Backing::Mock { scenario, .. } => scenario,
            Backing::Live { label, .. } => label,
        }
    }

    /// The scenario-playing source, when this session has one: tests and the
    /// developer panel choose what the mock daemon does next (a refusal, a
    /// lost connection, an account that can be recovered).
    #[cfg(feature = "demo")]
    pub fn mock_mut(&mut self) -> Option<&mut MockSource> {
        match &mut self.backing {
            Backing::Mock { source, .. } => Some(source),
            Backing::Live { .. } => None,
        }
    }

    #[cfg(feature = "demo")]
    fn mock(&self) -> Option<&MockSource> {
        match &self.backing {
            Backing::Mock { source, .. } => Some(source),
            Backing::Live { .. } => None,
        }
    }

    /// Offset 0 of a scenario's clock (the inspector's times); 0 when live.
    fn epoch(&self) -> Timestamp {
        #[cfg(feature = "demo")]
        return self.mock().map_or(Timestamp(0), MockSource::epoch);
        #[cfg(not(feature = "demo"))]
        Timestamp(0)
    }

    fn source_mut(&mut self) -> &mut dyn Source {
        match &mut self.backing {
            #[cfg(feature = "demo")]
            Backing::Mock { source, .. } => source.as_mut(),
            Backing::Live { source, .. } => source.as_mut(),
        }
    }

    /// Tells the source which context the user looks at.
    pub fn focus(&mut self, context: Option<&ContextId>) {
        self.focused_context = context.cloned();
        self.source_mut().focus(context);
    }

    /// Asks the source again for message bodies a view shows but the
    /// store's bounded cache no longer holds. Each body is asked once until
    /// it arrives (or the bodies are dropped on lock).
    pub fn request_bodies(&mut self, moments: &[MomentId]) {
        let fresh: Vec<MomentId> = moments
            .iter()
            .filter(|id| self.store.body(id).is_none() && !self.requested.contains(*id))
            .cloned()
            .collect();
        if fresh.is_empty() {
            return;
        }
        self.requested.extend(fresh.iter().cloned());
        self.source_mut().request_bodies(&fresh);
    }

    /// The view reached the oldest message shown of `context`: asks the
    /// source for the older page.
    pub fn request_older(&mut self, context: &ramflux_desktop_core::ContextId) {
        self.source_mut().request_older(context);
    }

    /// Asks the source for a call's media session (for the media engine).
    pub fn request_call_media(&mut self, call: &ramflux_desktop_core::CallId) {
        self.source_mut().request_call_media(call);
    }

    /// A media session that arrived from the source, if any.
    pub fn take_call_media(
        &mut self,
    ) -> Option<(ramflux_desktop_core::CallId, Result<ramflux_desktop_core::CallMedia, String>)>
    {
        self.source_mut().take_call_media()
    }

    /// The recovery phrase the source handed out for the running creation
    /// ceremony, once (a secret: it is not part of the store).
    pub fn take_recovery_phrase(&mut self) -> Option<ramflux_desktop_core::RecoveryPhrase> {
        self.source_mut().take_recovery_phrase()
    }

    /// The account's store, read-only.
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Offset of the session clock in ms.
    pub fn clock_ms(&self) -> u64 {
        self.clock_ms
    }

    /// Time of the session clock: scenario time, or the wall clock when live.
    pub fn now(&self) -> Timestamp {
        #[cfg(feature = "demo")]
        let scenario = self.mock().map(|source| source.epoch().saturating_add_ms(self.clock_ms));
        #[cfg(not(feature = "demo"))]
        let scenario = None;
        match scenario {
            Some(now) => now,
            None => self
                .live_epoch
                .saturating_add_ms(
                    u64::try_from(
                        (self.monotonic_now)()
                            .saturating_duration_since(self.live_started)
                            .as_millis(),
                    )
                    .unwrap_or(u64::MAX)
                    .max(self.clock_ms),
                )
                .max(self.time_floor),
        }
    }

    /// Advances a live session to an independently fired monotonic deadline.
    pub(super) fn reached_deadline(&mut self, deadline: Timestamp) {
        if self.is_live() {
            self.time_floor = self.time_floor.max(deadline);
        }
    }

    pub(super) fn logical_deadline(&self) -> Option<Timestamp> {
        let now = self.now();
        let ceremony = self.store.ceremony().filter(|c| c.is_active()).map(|c| c.expires_at);
        let recovery = self.store.recovery().filter(|c| c.is_active()).map(|c| c.expires_at);
        ceremony.into_iter().chain(recovery).filter(|due| *due > now).min()
    }

    pub(super) fn needs_render_time(&self) -> bool {
        self.networking_deadlines
            .keys()
            .any(|id| self.networking_retry_remaining(id).is_some_and(|ms| ms > 0))
            || self.focused_context.as_ref().is_some_and(|id| self.store.context_moments(id).any(|m| matches!(&m.kind, ramflux_desktop_core::MomentKind::Message(p) if matches!(p.receipt, ramflux_desktop_core::Receipt::WaitingForNode { .. }))))
            || self.logical_deadline().is_some()
            || ramflux_desktop_core::selectors::current_call(&self.store).is_some()
    }

    /// Remaining monotonic retry hint for the workspace header.
    pub fn networking_retry_remaining(
        &self,
        account: &ramflux_desktop_core::AccountId,
    ) -> Option<u64> {
        self.networking_deadlines.get(account).map(|(_, due)| due.saturating_since(self.now()))
    }

    /// Offset of the next scheduled event, if any (scenarios only).
    #[cfg(feature = "demo")]
    pub fn next_due(&self) -> Option<u64> {
        self.mock().and_then(MockSource::next_due)
    }

    /// Number of effects a mock source received (0 when live).
    #[cfg(feature = "demo")]
    pub fn submitted_effects(&self) -> usize {
        self.mock().map_or(0, |source| source.effects().len())
    }

    /// Labels of every effect a mock source received, oldest first.
    #[cfg(feature = "demo")]
    pub fn effect_labels(&self) -> impl Iterator<Item = &str> {
        self.mock().into_iter().flat_map(|s| s.effects().iter().map(|r| r.label.as_str()))
    }

    /// Every effect a mock source received with its (secret-free) body, oldest first.
    #[cfg(feature = "demo")]
    pub fn effects(&self) -> impl Iterator<Item = (&str, &ramflux_desktop_core::JsonValue)> {
        self.mock()
            .into_iter()
            .flat_map(|s| s.effects().iter().map(|r| (r.label.as_str(), &r.body)))
    }

    /// Whether the clock is frozen.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Freezes or unfreezes the clock.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    /// Moves the clock forward by `ms` and applies due envelopes. Does
    /// nothing while paused.
    pub fn advance(&mut self, ms: u64) -> Applied {
        if self.paused && !self.is_live() {
            return Applied::default();
        }
        self.clock_ms = self.clock_ms.saturating_add(ms);
        let mut applied = self.pump();
        if ms > 0 {
            applied.changes.mark_clock();
        }
        applied
    }

    /// Applies exactly the next scheduled envelope, paused or not, moving the
    /// clock to it when it lies ahead.
    #[cfg(feature = "demo")]
    pub fn step(&mut self) -> Applied {
        let mut applied = Applied::default();
        let Backing::Mock { source, .. } = &mut self.backing else { return applied };
        if let Some(env) = source.drain_one() {
            self.clock_ms = self.clock_ms.max(source.now_ms());
            self.apply(&env, &mut applied);
            applied.changes.mark_clock();
        }
        applied
    }

    /// Resets the store and replays everything scheduled up to `ms`,
    /// including responses to effects submitted before the seek.
    #[cfg(feature = "demo")]
    pub fn seek(&mut self, ms: u64) -> Applied {
        let Backing::Mock { source, .. } = &mut self.backing else { return Applied::default() };
        source.rewind();
        self.store = Store::new();
        self.requested.clear();
        *self.card_marks.borrow_mut() = CardMarks::default();
        self.clock_ms = ms;
        let mut applied = self.pump();
        let mut changes = ChangeSet::reset();
        changes.mark_clock();
        applied.changes = changes;
        applied
    }

    /// Whether a card on screen should be opened now: the store holds no
    /// session for it, none is being opened, and the last open of this
    /// surface was not refused at its current version.
    pub fn card_open_pending(&self, surface: &SurfaceId) -> bool {
        let marks = self.card_marks.borrow();
        let refused = marks.open_refused.get(surface) == Some(&self.surface_version(surface));
        self.store.card(surface).is_none() && !marks.opening.contains(surface) && !refused
    }

    /// The version of the surface a card belongs to (0 when unknown).
    fn surface_version(&self, surface: &SurfaceId) -> u64 {
        self.store
            .surface_moment(surface)
            .and_then(|moment| self.store.surface(moment))
            .map_or(0, |s| s.version)
    }

    /// Releases every card session this account holds (the account is being
    /// removed or signed out). The releases are dispatched like any other.
    pub fn dispose_all_cards(&mut self) -> Vec<InspectorEntry> {
        let mut records = Vec::new();
        for surface_id in self.cards_held() {
            let Some(moment_id) = self.card_moment(&surface_id) else { continue };
            let kind = IntentKind::CardDispose { moment_id, surface_id };
            records.extend(self.dispatch(Intent::human(kind)).1);
        }
        records
    }

    /// The cards the pool holds, by surface.
    pub fn cards_held(&self) -> Vec<SurfaceId> {
        self.store.card_pool().recent(usize::MAX).into_iter().map(SurfaceId::new).collect()
    }

    /// The moment that presents a held card.
    pub fn card_moment(&self, surface: &SurfaceId) -> Option<MomentId> {
        self.store.surface_moment(surface).cloned()
    }

    /// Whether a card's session release was asked and not answered yet.
    pub fn card_disposing(&self, surface: &SurfaceId) -> bool {
        self.card_marks.borrow().disposing.contains(surface)
    }

    /// Whether a resumed card must be presented in another locale or theme.
    pub fn card_presentation_stale(&self, surface: &SurfaceId, locale: &str, dark: bool) -> bool {
        let Some(record) = self.store.card(surface) else { return false };
        record.tree.is_some()
            && record.resumed(self.store.card_pool(), surface)
            && self
                .card_marks
                .borrow()
                .presented
                .get(surface)
                .is_none_or(|p| p.0 != locale || p.1 != dark)
    }

    /// Dispatches the bus steps the store queued for cards (a re-read, a
    /// submit the runtime asked for, a pool suspend or dispose).
    /// While the app lock is closed, a re-read and a submit wait for it to
    /// open; a suspend or dispose goes at once.
    fn dispatch_card_due(&mut self) -> Vec<InspectorEntry> {
        let mut records = Vec::new();
        let locked = self.cards_locked();
        let mut due = std::mem::take(&mut self.card_marks.borrow_mut().held_due);
        due.extend(self.store.take_card_due());
        for due in due {
            if locked && matches!(due, CardDue::Refresh(_) | CardDue::Submit(_)) {
                let mut marks = self.card_marks.borrow_mut();
                if !marks.held_due.contains(&due) {
                    marks.held_due.push(due);
                }
                continue;
            }
            let surface_id = due.surface().clone();
            let Some(moment_id) = self.store.surface_moment(&surface_id).cloned() else {
                continue;
            };
            let kind = match due {
                CardDue::Refresh(_) => IntentKind::CardRefresh { moment_id, surface_id },
                CardDue::Submit(_) => IntentKind::CardSubmit { moment_id, surface_id },
                CardDue::Suspend(_) => IntentKind::CardSuspend { moment_id, surface_id },
                CardDue::Dispose(_) => IntentKind::CardDispose { moment_id, surface_id },
            };
            records.extend(self.dispatch(Intent::human(kind)).1);
        }
        records
    }

    /// Runs policy for `intent` at the session time and submits the
    /// resulting effects. A confirmation request is handed back unsubmitted.
    /// A host event on a card waits while another call on that card is in
    /// flight, while the card is suspended, or while the app lock is closed
    /// (see [`QueuedEvent`]).
    pub fn dispatch(&mut self, intent: Intent) -> (DispatchOutcome, Vec<InspectorEntry>) {
        if let Some(mut event) = QueuedEvent::of(&intent.kind) {
            let suspended = self.card_suspended(&event.surface_id);
            if suspended {
                event.pre_resume = self.pre_resume_version(&event);
            }
            let holds = suspended
                || self.cards_locked()
                || self.card_marks.borrow().holds(&event.surface_id);
            if holds {
                self.card_marks.borrow_mut().enqueue(event);
                // Sent now when no call is in flight on the card.
                let records = self.dispatch_card_queue();
                return (DispatchOutcome::Submitted(Vec::new()), records);
            }
        }
        self.dispatch_now(intent)
    }

    /// Whether the app lock holds this account closed: no card is resumed,
    /// re-read or sent host events until it opens.
    fn cards_locked(&self) -> bool {
        self.store.any_app_locked()
    }

    /// Drops the host events waiting on `surface`. When any were waiting the
    /// card shows that the user's action was not sent (they act again on the
    /// card as it is).
    fn drop_queue(&mut self, surface: &SurfaceId) {
        let dropped = self.card_marks.borrow_mut().queued.remove(surface);
        if dropped.is_some_and(|queue| !queue.is_empty()) {
            self.store.card_refuse(surface, CARD_CHANGED_REASON);
        }
    }

    /// Dispatches waiting host events, one per card with no call in flight
    /// (nothing while the app lock is closed).
    fn dispatch_card_queue(&mut self) -> Vec<InspectorEntry> {
        let mut records = Vec::new();
        if self.cards_locked() {
            return records;
        }
        loop {
            let ready = {
                let marks = self.card_marks.borrow();
                marks
                    .queued
                    .iter()
                    .find(|(surface, queue)| {
                        !queue.is_empty() && !marks.busy.contains_key(*surface)
                    })
                    .map(|(surface, _)| surface.clone())
            };
            let Some(surface) = ready else { break };
            if self.card_suspended(&surface) {
                // The resume consumes a tap, so the events wait for it and
                // are sent on the resumed tree (see `rebase_after_resume`).
                let Some(moment_id) = self.queued_moment(&surface) else {
                    self.drop_queue(&surface);
                    continue;
                };
                self.card_marks.borrow_mut().busy.insert(surface.clone(), InFlight::Resume);
                let refresh = Intent::human(IntentKind::CardRefresh {
                    moment_id,
                    surface_id: surface.clone(),
                });
                let (outcome, recorded) = self.dispatch_now(refresh);
                records.extend(recorded);
                if !matches!(&outcome, DispatchOutcome::Submitted(labels) if !labels.is_empty()) {
                    self.card_marks.borrow_mut().busy.remove(&surface);
                    self.drop_queue(&surface);
                }
                continue;
            }
            let mut next = {
                let mut marks = self.card_marks.borrow_mut();
                let Some(queue) = marks.queued.get_mut(&surface) else { break };
                let next = queue.remove(0);
                if queue.is_empty() {
                    marks.queued.remove(&surface);
                }
                next
            };
            // An event taken on this desktop's own chain of stamps follows
            // the held version where it may (a draft whose node keeps its
            // kind and target; a tap whose node and signed version are
            // exactly what the user saw); any other stamp is checked as is.
            let input = next.is_input();
            if let Some(seen) =
                self.store.card_rebased(&next.surface_id, &next.key, &next.seen, input)
            {
                next.seen = seen;
            }
            let (outcome, recorded) = self.dispatch_now(next.into_intent());
            records.extend(recorded);
            // An event that was not sent means what waits behind it (a tap
            // behind its drafts) must not fire on its own: the queue is
            // dropped, and the card says so.
            let sent = matches!(&outcome, DispatchOutcome::Submitted(labels) if !labels.is_empty());
            if !sent {
                self.drop_queue(&surface);
            }
        }
        records
    }

    /// The held card of `surface` is suspended in the pool.
    fn card_suspended(&self, surface: &SurfaceId) -> bool {
        self.store
            .card(surface)
            .is_some_and(|r| r.tree.is_some() && !r.resumed(self.store.card_pool(), surface))
    }

    fn queued_moment(&self, surface: &SurfaceId) -> Option<MomentId> {
        let marks = self.card_marks.borrow();
        marks.queued.get(surface)?.first().map(|q| q.moment_id.clone())
    }

    /// The signed version an event is valid on in the held tree (before a
    /// suspend): its stamp names the held version, or it may be rebased onto
    /// it (see [`ramflux_desktop_core::CardRecord::rebasable`]).
    fn pre_resume_version(&self, event: &QueuedEvent) -> Option<u64> {
        let record = self.store.card(&event.surface_id)?;
        let tree = record.tree.as_ref()?;
        let valid = event.seen.stamp == tree.stamp()
            || record.rebasable(&event.key, &event.seen, event.is_input());
        valid.then(|| tree.signed_version())
    }

    /// Notes, for the events waiting on a card that is being suspended, the
    /// version each is valid on (they wait behind a call that was in flight
    /// when they were taken, so they were not checked yet).
    fn note_pre_resume(&mut self, surface: &SurfaceId) {
        let Some(mut queue) = self.card_marks.borrow_mut().queued.remove(surface) else {
            return;
        };
        for event in &mut queue {
            if event.pre_resume.is_none() {
                event.pre_resume = self.pre_resume_version(event);
            }
        }
        self.card_marks.borrow_mut().queued.insert(surface.clone(), queue);
    }

    /// Rebases the events that waited for a resume onto the resumed tree. A
    /// text draft whose node keeps its kind and input target follows it (the
    /// whole value replaces the draft); a tap only when the signed version
    /// and the node's drawn content are what the user saw. Anything else is
    /// dropped, and the card says so: the user acts again on it as it is.
    fn rebase_after_resume(&mut self, surface: &SurfaceId) {
        let Some(tree) = self.store.card(surface).and_then(|r| r.tree.clone()) else {
            self.drop_queue(surface);
            return;
        };
        let stamp = tree.stamp();
        let signed = tree.signed_version();
        let mut dropped = false;
        {
            let mut marks = self.card_marks.borrow_mut();
            let Some(queue) = marks.queued.get_mut(surface) else { return };
            queue.retain_mut(|q| {
                let same_node = q.pre_resume.is_some()
                    && tree.node(&q.key).is_some_and(|n| {
                        n.kind == q.seen.kind
                            && n.attrs.get("input_pointer").and_then(serde_json::Value::as_str)
                                == q.seen.input_pointer.as_deref()
                    });
                let keep = same_node
                    && (q.is_input()
                        || (q.pre_resume == Some(signed)
                            && q.seen.content.is_some()
                            && q.seen.content == tree.content(&q.key)));
                if keep {
                    q.seen.stamp = stamp;
                    q.pre_resume = None;
                } else {
                    dropped = true;
                }
                keep
            });
            if queue.is_empty() {
                marks.queued.remove(surface);
            }
        }
        if dropped {
            self.store.card_refuse(surface, CARD_CHANGED_REASON);
        }
    }

    fn dispatch_now(&mut self, intent: Intent) -> (DispatchOutcome, Vec<InspectorEntry>) {
        let mark = card_mark(&intent.kind);
        let user_surface = card_user_surface(&intent.kind).cloned();
        let input =
            matches!(&intent.kind, IntentKind::CardInteract { event: CardInput::Input { .. }, .. });
        let host_event = matches!(
            &intent.kind,
            IntentKind::CardInteract { .. } | IntentKind::CardSlotAnswer { .. }
        );
        let submit = matches!(&intent.kind, IntentKind::CardSubmit { .. });
        let (decided, mut records) = self.decide(intent);
        let effects = match decided {
            Ok(effects) => effects,
            Err(outcome) => {
                // A tap, draft or answer that is not sent is never silent.
                if let (DispatchOutcome::Refused(_), Some(surface), true) =
                    (&outcome, &user_surface, host_event)
                {
                    self.store.card_refuse(surface, CARD_CHANGED_REASON);
                }
                return (outcome, records);
            }
        };
        self.note_card_mark(mark);
        if let Some(surface) = &user_surface {
            self.store.card_bump(surface);
            if submit && !effects.is_empty() {
                self.store.card_submit_sent(surface);
            }
        }
        let mut labels = Vec::new();
        for effect in effects {
            if let Some(surface) = &user_surface
                && is_event_call(&effect)
            {
                self.card_marks
                    .borrow_mut()
                    .busy
                    .insert(surface.clone(), InFlight::Event { input });
            }
            let label = effect_label(&effect);
            records.push(InspectorEntry::Effect { label: label.clone() });
            labels.push(label);
            self.submit(effect);
        }
        (DispatchOutcome::Submitted(labels), records)
    }

    /// Runs policy for `intent` against this session's store without
    /// submitting: the effects to submit, or the outcome that ends here
    /// (refusal or confirmation request).
    pub fn decide(
        &self,
        intent: Intent,
    ) -> (Result<Vec<Effect>, DispatchOutcome>, Vec<InspectorEntry>) {
        let name = intent_name(&intent);
        let actor = audit_label(&intent);
        let mut records = Vec::new();
        let effects = match handle_intent(&self.store, intent, self.now()) {
            Ok(effects) => effects,
            Err(error) => {
                records.push(InspectorEntry::Intent {
                    intent: name,
                    actor,
                    result: format!("refused: {error}"),
                });
                return (Err(DispatchOutcome::Refused(error)), records);
            }
        };
        let mut submit = Vec::new();
        for effect in effects {
            if let Effect::RequireConfirmation { intent } = effect {
                records.push(InspectorEntry::Intent {
                    intent: name,
                    actor,
                    result: "confirmation required".to_owned(),
                });
                return (Err(DispatchOutcome::NeedsConfirmation(intent)), records);
            }
            submit.push(effect);
        }
        (Ok(submit), records)
    }

    /// Records that a card intent was accepted for submission: a card opens
    /// once until its first answer, and a presentation is remembered so the
    /// same locale and theme are not sent again.
    pub fn note_card_mark(&self, mark: Option<CardMark>) {
        let mut marks = self.card_marks.borrow_mut();
        match mark {
            Some(CardMark::Open(surface)) => {
                marks.opening.insert(surface);
            }
            Some(CardMark::Present(surface, locale, dark)) => {
                marks.presented.insert(surface, (locale, dark));
            }
            Some(CardMark::Dispose(surface)) => {
                marks.disposing.insert(surface);
            }
            None => {}
        }
    }

    /// Hands one effect to the source.
    pub fn submit(&mut self, effect: Effect) {
        self.source_mut().submit(effect);
    }

    fn pump(&mut self) -> Applied {
        let mut applied = Applied::default();
        let now = self.now();
        for env in self.source_mut().drain(now) {
            self.apply(&env, &mut applied);
        }
        applied.records.extend(self.dispatch_card_due());
        applied.records.extend(self.dispatch_card_queue());
        applied.open_urls = std::mem::take(&mut self.open_urls);
        if self.source_mut().recovery_phrase_ready() {
            applied
                .changes
                .merge(ramflux_desktop_core::Changes { ceremony: true, ..Default::default() });
        }
        if !self.requested.is_empty() {
            let store = &self.store;
            self.requested.retain(|id| store.body(id).is_none() && store.moment(id).is_some());
        }
        applied
    }

    fn apply(&mut self, env: &EventEnvelope, applied: &mut Applied) {
        if let DomainEvent::Surface(_) = &env.event {
            // A surface change or resync lets a refused open be asked again.
            self.card_marks.borrow_mut().open_refused.clear();
        }
        if let DomainEvent::SourceStatusChanged(status) = &env.event
            && *status != ramflux_desktop_core::SourceStatus::Connected
        {
            // The store forgets every card with the connection (see
            // `Store::cards_reset`): nothing about them is in flight any more,
            // and cards on screen are opened again once it is back.
            *self.card_marks.borrow_mut() = CardMarks::default();
        }
        // What the card's waiting events must do once the fact is applied.
        let mut drop_waiting = false;
        let mut resumed_waiting = false;
        let mut own_tap_reply = false;
        if let DomainEvent::Card(card) = &env.event {
            let surface = card.surface_id().clone();
            let in_flight = self.card_marks.borrow().busy.get(&surface).copied();
            {
                let mut marks = self.card_marks.borrow_mut();
                marks.opening.remove(&surface);
                match card {
                    CardEvent::Disposed { .. } => {
                        // The card is gone: nothing of it waits any more.
                        marks.disposing.remove(&surface);
                        marks.busy.remove(&surface);
                        marks.queued.remove(&surface);
                    }
                    CardEvent::Refused { .. }
                    | CardEvent::Stale { .. }
                    | CardEvent::Fallback { .. } => {
                        // A refused dispose is not in flight any more: the
                        // pool may ask again. A refused resume ends it. Events
                        // waiting on the card are dropped (and the card says
                        // so); an event call in flight settles on its own.
                        if matches!(card, CardEvent::Refused { .. }) {
                            marks.disposing.remove(&surface);
                        }
                        if in_flight == Some(InFlight::Resume) {
                            marks.busy.remove(&surface);
                        }
                        drop_waiting = true;
                    }
                    CardEvent::EventSettled { .. } => {
                        if matches!(in_flight, Some(InFlight::Event { .. })) {
                            marks.busy.remove(&surface);
                        }
                    }
                    CardEvent::Batch { own_reply: true, .. } => {
                        own_tap_reply = in_flight == Some(InFlight::Event { input: false });
                    }
                    CardEvent::Resumed { .. } => {
                        resumed_waiting = in_flight == Some(InFlight::Resume);
                    }
                    _ => {}
                }
            }
            if matches!(card, CardEvent::Suspended { .. }) {
                self.note_pre_resume(&surface);
            }
            // The browser opens only for the link the sheet showed: the one
            // the store holds for this card (a suspend or resume clears it),
            // and only an https link the host accepts.
            if let CardEvent::LinkOpened { url, .. } = card
                && ramflux_desktop_core::https_host(url).is_some()
                && self
                    .store
                    .card(&surface)
                    .and_then(|record| record.link.as_ref())
                    .is_some_and(|link| &link.url == url)
            {
                self.open_urls.push(url.clone());
            }
        }
        let result = self.store.apply(env);
        if let DomainEvent::Card(card) = &env.event {
            let surface = card.surface_id();
            if own_tap_reply {
                // A tap of this desktop changed the tree: no event taken
                // before it may follow the new version.
                self.store.card_reset_chain(surface);
            }
            if resumed_waiting {
                self.rebase_after_resume(surface);
                self.card_marks.borrow_mut().busy.remove(surface);
            }
            if drop_waiting {
                self.drop_queue(surface);
            }
        }
        if let DomainEvent::Card(card) = &env.event
            && matches!(card, CardEvent::Refused { .. } | CardEvent::Fallback { .. })
            && self.store.card(card.surface_id()).is_none()
        {
            let version = self.surface_version(card.surface_id());
            self.card_marks.borrow_mut().open_refused.insert(card.surface_id().clone(), version);
        }
        applied.records.push(InspectorEntry::Envelope {
            seq: env.seq,
            at_ms: env.at.saturating_since(self.epoch()),
            event: event_name(&env.event),
            error: result.as_ref().err().map(ToString::to_string),
        });
        if let Ok(changes) = result {
            let now = self.now();
            for id in &changes.accounts {
                let reason = self
                    .store
                    .account(id)
                    .and_then(|a| a.networking_reason.as_ref())
                    .filter(|r| !r.requires_action() && r.retry_after_ms.is_some());
                if let Some(reason) = reason {
                    if self.networking_deadlines.get(id).is_none_or(|(seen, _)| seen != reason) {
                        self.networking_deadlines.insert(
                            id.clone(),
                            (
                                reason.clone(),
                                now.saturating_add_ms(reason.retry_after_ms.unwrap_or(0)),
                            ),
                        );
                    }
                } else {
                    self.networking_deadlines.remove(id);
                }
            }
            applied.changes.merge(changes);
        }
    }
}

/// The audit pair of an intent as an inspector label, e.g.
/// `Agent(p-orbit), confirmed by Tap` (developer inspector only, untranslated).
pub fn audit_label(intent: &Intent) -> String {
    match &intent.confirmed_by {
        Some(confirmation) => {
            format!("{:?}, confirmed by {:?}", intent.proposed_by, confirmation.gesture())
        }
        None => format!("{:?}", intent.proposed_by),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn live_now_follows_injected_monotonic_time() {
        use std::{
            cell::Cell,
            rc::Rc,
            time::{Duration, Instant},
        };
        let scenario = ramflux_desktop_test::load_fixture("s02_daily").expect("scenario");
        let base = Instant::now();
        let now = Rc::new(Cell::new(base));
        let clock = now.clone();
        let session = super::AccountSession::live_with_clock(
            "clock",
            Box::new(ramflux_desktop_test::MockSource::new(&scenario)),
            move || clock.get(),
        );
        let epoch = session.now();
        now.set(base + Duration::from_secs(60));
        assert_eq!(session.now(), epoch.saturating_add_ms(60_000));
        now.set(base + Duration::from_secs(61));
        assert_eq!(session.now(), epoch.saturating_add_ms(61_000));
    }

    use ramflux_desktop_core::{
        Actor, ConfirmationIssuer, GrantId, GrantState, IntentKind, ParticipantId, methods,
    };
    use ramflux_desktop_test::load_fixture;

    use super::*;

    fn session(name: &str) -> AccountSession {
        AccountSession::new(&load_fixture(name).expect("fixture"))
    }

    fn tap() -> ramflux_desktop_core::HumanConfirmation {
        ConfirmationIssuer::new().issue_tap()
    }

    fn approve(actor: Actor, confirmed: bool) -> Intent {
        let intent = Intent {
            proposed_by: actor,
            confirmed_by: None,
            kind: IntentKind::Approve { grant_id: GrantId::new("g-rotate") },
        };
        if confirmed { intent.confirmed_by_human(tap()) } else { intent }
    }

    #[test]
    fn seek_to_zero_replays_to_the_same_state() {
        let mut live = session("s02_daily");
        live.advance(1_000);
        live.advance(2_000);
        let before = format!("{:?}", live.store());

        let at_zero = load_fixture("s02_daily")
            .expect("fixture")
            .timeline
            .iter()
            .filter(|(offset, _)| *offset == 0)
            .count();
        let applied = live.seek(0);
        assert!(applied.changes.is_reset());
        assert_eq!(live.store().last_seq(), u64::try_from(at_zero).unwrap_or(0), "only t=0 events");
        live.seek(3_000);
        assert_eq!(format!("{:?}", live.store()), before);
    }

    #[test]
    fn paused_clock_does_not_move_but_step_applies() {
        let mut s = session("s04_safety");
        s.set_paused(true);
        assert!(s.advance(10_000).changes.is_empty());
        assert_eq!(s.store().last_seq(), 0);
        assert_eq!(s.step().records.len(), 1);
        assert_eq!(s.store().last_seq(), 1);
    }

    #[test]
    fn high_risk_approval_waits_until_a_human_confirmation_for_humans_and_agents() {
        let mut s = session("s03_intervention");
        s.advance(1_000);
        let agent = Actor::Agent(ParticipantId::new("p-orbit"));
        let (outcome, _) = s.dispatch(approve(agent.clone(), false));
        let DispatchOutcome::NeedsConfirmation(intent) = outcome else {
            panic!("an unconfirmed agent proposal waits for the confirmation");
        };
        assert_eq!(intent.proposed_by, agent);
        let (outcome, records) = s.dispatch(intent.confirmed_by_human(tap()));
        assert!(
            matches!(outcome, DispatchOutcome::Submitted(ref labels) if labels == &[methods::GRANT_APPROVE]),
            "{outcome:?}"
        );
        assert!(records.iter().all(|r| !format!("{r:?}").contains("secret")));
        s.advance(500);
        let grant = s.store().grant(&GrantId::new("g-rotate")).expect("grant");
        assert_eq!(grant.state, GrantState::Approved);
        let (outcome, _) = s.dispatch(approve(Actor::Human, true));
        assert!(matches!(outcome, DispatchOutcome::Refused(_)), "a decided grant is refused");
    }

    /// Card host events through the session: its queue, rebase and lock
    /// rules, against a scripted card runtime.
    mod cards {
        use std::cell::RefCell;
        use std::rc::Rc;

        use ramflux_desktop_core::{
            AccountId, AppLock, AppLockState, CARD_CHANGED_REASON, CardBatch, CardEvent,
            CardHostSlot, CardInput, CardLifecycle, CardMutation, CardRecord, CardSlotOutcome,
            CardTree, DomainEvent, Effect, EventEnvelope, Intent, IntentKind, JsonValue,
            MUTATIONS_SCHEMA, MomentId, Source, SurfaceId, Timestamp,
        };
        use ramflux_desktop_test::{MockSource, load_fixture, mock_cards};
        use serde_json::json;

        use super::super::{AccountSession, CardMarks, DispatchOutcome, QueuedEvent};

        const MOMENT: &str = "m-window";
        const SURFACE: &str = "s-window";

        type Calls = Rc<RefCell<Vec<(String, JsonValue)>>>;
        type Replies = Rc<RefCell<Vec<DomainEvent>>>;

        /// The scenario's mock for everything but cards; card calls are
        /// recorded and answered by the test.
        struct Script {
            inner: MockSource,
            seq: u64,
            calls: Calls,
            replies: Replies,
        }

        impl Source for Script {
            fn submit(&mut self, effect: Effect) {
                let method = effect.method().unwrap_or_default().to_owned();
                if method.starts_with("card.session.") {
                    let body = effect.body().cloned().unwrap_or_default();
                    self.calls.borrow_mut().push((method, body));
                } else {
                    self.inner.submit(effect);
                }
            }

            fn drain(&mut self, now: Timestamp) -> Vec<EventEnvelope> {
                let mut events: Vec<DomainEvent> =
                    self.inner.drain(now).into_iter().map(|e| e.event).collect();
                events.extend(self.replies.borrow_mut().drain(..));
                events
                    .into_iter()
                    .map(|event| {
                        self.seq += 1;
                        EventEnvelope::new(self.seq, now, event)
                    })
                    .collect()
            }
        }

        struct World {
            session: AccountSession,
            calls: Calls,
            replies: Replies,
        }

        fn surface() -> SurfaceId {
            SurfaceId::new(SURFACE)
        }

        impl World {
            /// The form card of the scenario, opened.
            fn open() -> Self {
                let scenario = load_fixture("s03_intervention").expect("test: fixture");
                let calls = Calls::default();
                let replies = Replies::default();
                let script = Script {
                    inner: MockSource::with_epoch(&scenario, Timestamp(0)),
                    seq: 0,
                    calls: calls.clone(),
                    replies: replies.clone(),
                };
                let mut world = Self {
                    session: AccountSession::live("cards", Box::new(script)),
                    calls,
                    replies,
                };
                world.session.advance(0);
                let (outcome, _) = world.session.dispatch(Intent::human(IntentKind::CardOpen {
                    moment_id: MomentId::new(MOMENT),
                    surface_id: surface(),
                    locale: "en".into(),
                    dark: false,
                }));
                assert!(matches!(outcome, DispatchOutcome::Submitted(_)), "{outcome:?}");
                world.reply(mock_cards::respond(
                    "card.session.open",
                    &json!({ "surface_id": SURFACE }),
                ));
                assert!(
                    world.record().tree.is_some(),
                    "the card opened: {:?} {:?}",
                    world.record(),
                    world.session.store().rejections()
                );
                world
            }

            fn reply(&mut self, events: Vec<DomainEvent>) {
                self.replies.borrow_mut().extend(events);
                self.session.advance(0);
            }

            fn record(&self) -> CardRecord {
                self.session.store().card(&surface()).cloned().expect("test: card")
            }

            fn tree(&self) -> CardTree {
                self.record().tree.expect("test: tree")
            }

            fn key(&self, kind: &str) -> String {
                self.tree().nodes().find(|n| n.kind == kind).map(|n| n.key.clone()).expect("key")
            }

            fn interact(&mut self, kind: &str, event: CardInput) -> DispatchOutcome {
                let key = self.key(kind);
                let seen = self.tree().seen(&key).expect("test: seen");
                self.session
                    .dispatch(Intent::human(IntentKind::CardInteract {
                        moment_id: MomentId::new(MOMENT),
                        surface_id: surface(),
                        key,
                        event,
                        seen,
                    }))
                    .0
            }

            fn type_text(&mut self, text: &str) -> DispatchOutcome {
                self.interact("TextField", CardInput::Input { value: json!(text) })
            }

            fn tap(&mut self) -> DispatchOutcome {
                self.interact("Button", CardInput::Activate)
            }

            fn count(&self, method: &str) -> usize {
                self.calls.borrow().iter().filter(|(m, _)| m == method).count()
            }

            fn last(&self, method: &str) -> JsonValue {
                self.calls
                    .borrow()
                    .iter()
                    .rev()
                    .find(|(m, _)| m == method)
                    .map(|(_, b)| b.clone())
                    .expect("test: call")
            }

            /// A reply batch of this desktop's own event: the held tree
            /// moved on by one edit with `mutations`.
            fn own_batch(&self, mutations: Vec<CardMutation>) -> CardBatch {
                let tree = self.tree();
                CardBatch {
                    schema: MUTATIONS_SCHEMA.to_owned(),
                    session: tree.session(),
                    previous_version: tree.tree_version(),
                    version: tree.tree_version() + 1,
                    signed_version: tree.signed_version(),
                    signed_hash: tree.signed_hash().to_owned(),
                    edit_seq: tree.stamp().edit_seq + 1,
                    mutations,
                    host_slots: tree.host_slots().to_vec(),
                    locale: "en".into(),
                    format_locale: "en".into(),
                    direction: "ltr".into(),
                    generating: false,
                    lifecycle: CardLifecycle::Open,
                    replacement_ref: None,
                }
            }

            /// The node of `kind` with one attribute changed, as an upsert.
            fn changed(&self, kind: &str, attr: &str, value: &str) -> CardMutation {
                let tree = self.tree();
                let node = tree.nodes().find(|n| n.kind == kind).cloned().expect("node");
                let parent = node.parent_key.clone();
                let index = parent
                    .as_deref()
                    .and_then(|p| tree.node(p))
                    .and_then(|p| p.children.iter().position(|c| *c == node.key))
                    .unwrap_or(0);
                let mut node = node;
                node.attrs.insert(attr.to_owned(), json!(value));
                CardMutation::Upsert { parent, index, node: Box::new(node) }
            }

            fn own_reply(&self, mutations: Vec<CardMutation>) -> DomainEvent {
                DomainEvent::Card(CardEvent::Batch {
                    surface_id: surface(),
                    batch: Box::new(self.own_batch(mutations)),
                    draft_conflict: false,
                    own_reply: true,
                })
            }

            fn lock(&mut self, state: AppLockState) {
                self.reply(vec![DomainEvent::AppLockChanged {
                    account_id: AccountId::new("acct-main"),
                    lock: AppLock { enabled: true, state, ..AppLock::default() },
                }]);
            }
        }

        fn settled() -> DomainEvent {
            DomainEvent::Card(CardEvent::EventSettled { surface_id: surface() })
        }

        const EVENT: &str = "card.session.event";

        #[test]
        fn an_event_reply_without_a_batch_frees_the_card_for_the_next_tap() {
            let mut world = World::open();
            world.tap();
            assert_eq!(world.count(EVENT), 1);
            // `edited` without a batch: the call settles and nothing else.
            world.reply(vec![settled()]);
            world.tap();
            assert_eq!(world.count(EVENT), 2, "the second tap is sent, not held forever");
        }

        #[test]
        fn a_tap_behind_a_draft_whose_reply_relabels_the_button_is_refused_with_a_notice() {
            let mut world = World::open();
            world.type_text("Grace");
            world.tap();
            assert_eq!(world.count(EVENT), 1, "the tap waits behind the draft");
            let relabel = world.changed("Text", "text", "Delete everything");
            let reply = world.own_reply(vec![relabel]);
            world.reply(vec![reply, settled()]);
            assert_eq!(world.count(EVENT), 1, "the tap is not sent on a changed button");
            assert_eq!(world.record().refusal.as_deref(), Some(CARD_CHANGED_REASON));
        }

        #[test]
        fn a_tap_behind_a_draft_that_changes_only_the_field_follows_it() {
            let mut world = World::open();
            world.type_text("Grace");
            world.tap();
            let field = world.changed("TextField", "value", "Grace");
            let reply = world.own_reply(vec![field]);
            world.reply(vec![reply, settled()]);
            assert_eq!(world.count(EVENT), 2, "the tap follows its own draft");
            assert_eq!(world.record().refusal, None);
        }

        #[test]
        fn a_draft_merges_only_with_the_draft_at_the_tail() {
            let event = |key: &str, input: Option<&str>| QueuedEvent {
                moment_id: MomentId::new(MOMENT),
                surface_id: surface(),
                key: key.to_owned(),
                event: match input {
                    Some(text) => CardInput::Input { value: json!(text) },
                    None => CardInput::Activate,
                },
                seen: ramflux_desktop_core::CardSeen::default(),
                pre_resume: None,
            };
            let mut marks = CardMarks::default();
            marks.enqueue(event("field", Some("1")));
            marks.enqueue(event("button", None));
            marks.enqueue(event("field", Some("2")));
            assert_eq!(marks.queued[&surface()].len(), 3, "no draft jumps over the tap");
            marks.enqueue(event("field", Some("3")));
            let queue = &marks.queued[&surface()];
            assert_eq!(queue.len(), 3, "a draft replaces the draft at the tail");
            assert!(
                matches!(&queue[0].event, CardInput::Input { value } if value == "1"),
                "the draft before the tap keeps its value"
            );
            assert!(matches!(&queue[2].event, CardInput::Input { value } if value == "3"));
        }

        #[test]
        fn waiting_events_dropped_on_a_stale_card_leave_a_notice() {
            let mut world = World::open();
            world.type_text("Grace");
            world.tap();
            world.reply(vec![
                DomainEvent::Card(CardEvent::Stale { surface_id: surface() }),
                settled(),
            ]);
            assert_eq!(world.count(EVENT), 1, "the waiting tap is dropped");
            assert_eq!(world.record().refusal.as_deref(), Some(CARD_CHANGED_REASON));
        }

        #[test]
        fn events_waiting_behind_a_call_survive_a_suspend_and_follow_the_resume() {
            let mut world = World::open();
            world.type_text("Grace");
            world.tap();
            let field = world.changed("TextField", "value", "Grace");
            let reply = world.own_reply(vec![field]);
            world.reply(vec![
                reply,
                settled(),
                DomainEvent::Card(CardEvent::Suspended { surface_id: surface() }),
            ]);
            assert_eq!(world.count("card.session.resume"), 1, "the card is resumed for the tap");
            assert_eq!(world.count(EVENT), 1);
            world.reply(mock_cards::respond(
                "card.session.resume",
                &json!({ "surface_id": SURFACE }),
            ));
            assert_eq!(world.count(EVENT), 2, "the tap is sent on the resumed tree");
            assert_eq!(world.last(EVENT)["session"], json!(world.tree().session()));
            assert_eq!(world.record().refusal, None);
        }

        #[test]
        fn a_tap_dropped_by_a_resume_on_a_new_version_leaves_a_notice() {
            let mut world = World::open();
            world.reply(vec![DomainEvent::Card(CardEvent::Suspended { surface_id: surface() })]);
            world.tap();
            assert_eq!(world.count("card.session.resume"), 1);
            let mut resumed =
                mock_cards::respond("card.session.resume", &json!({ "surface_id": SURFACE }));
            if let Some(DomainEvent::Card(CardEvent::Resumed { batch, .. })) = resumed.first_mut() {
                batch.signed_version += 1;
            }
            world.reply(resumed);
            assert_eq!(world.count(EVENT), 0, "a tap is never sent on a version not seen");
            assert_eq!(world.record().refusal.as_deref(), Some(CARD_CHANGED_REASON));
            // The card is resumed now: the next tap goes at once.
            world.tap();
            assert_eq!(world.count(EVENT), 1);
            assert_eq!(world.count("card.session.resume"), 1, "no second resume");
        }

        #[test]
        fn nothing_is_resumed_re_read_submitted_or_sent_while_the_app_lock_is_closed() {
            let mut world = World::open();
            world.lock(AppLockState::Locked);
            world.tap();
            world.reply(vec![DomainEvent::Card(CardEvent::Stale { surface_id: surface() })]);
            assert_eq!(world.count(EVENT), 0, "a tap waits for the lock to open");
            assert_eq!(world.count("card.session.update"), 0, "a re-read waits too");
            world.lock(AppLockState::Unlocked);
            assert_eq!(world.count("card.session.update"), 1, "the re-read goes once open");
        }

        #[test]
        fn a_held_submit_goes_once_the_lock_opens_and_a_failed_one_is_retried_by_the_user() {
            let mut world = World::open();
            world.lock(AppLockState::Locked);
            world
                .reply(vec![DomainEvent::Card(CardEvent::PendingSubmit { surface_id: surface() })]);
            assert_eq!(world.count("card.session.submit"), 0);
            world.lock(AppLockState::Unlocked);
            assert_eq!(world.count("card.session.submit"), 1);
            world.reply(vec![DomainEvent::Card(CardEvent::SubmitFailed { surface_id: surface() })]);
            assert!(world.record().submit_failed);
            assert_eq!(world.count("card.session.submit"), 1, "never retried by itself");
            let (outcome, _) = world.session.dispatch(Intent::human(IntentKind::CardSubmit {
                moment_id: MomentId::new(MOMENT),
                surface_id: surface(),
            }));
            assert!(matches!(outcome, DispatchOutcome::Submitted(ref l) if !l.is_empty()));
            assert_eq!(world.count("card.session.submit"), 2);
            assert!(!world.record().submit_failed);
        }

        #[test]
        fn an_approval_without_verified_facts_cannot_be_answered() {
            let mut world = World::open();
            let button = world.key("Button");
            let token = world.tree().token_for(&button).expect("token");
            let slot = |facts: Option<JsonValue>| {
                let mut request = serde_json::Map::new();
                request.insert("kind".into(), json!("approval"));
                request.insert("request_id".into(), json!("r-1"));
                request.insert("request_ref".into(), json!("appr-1"));
                if let Some(facts) = facts {
                    request.insert("facts".into(), facts);
                }
                CardHostSlot {
                    id: format!("[approval,{button}]"),
                    key: button.clone(),
                    token: token.clone(),
                    request,
                }
            };
            let mut batch = world.own_batch(Vec::new());
            batch.host_slots = vec![slot(None)];
            world.reply(vec![DomainEvent::Card(CardEvent::Batch {
                surface_id: surface(),
                batch: Box::new(batch),
                draft_conflict: false,
                own_reply: false,
            })]);
            let answer = |world: &mut World| {
                let seen = ramflux_desktop_core::CardSeen {
                    stamp: world.tree().stamp(),
                    ..Default::default()
                };
                world
                    .session
                    .dispatch(Intent::human(IntentKind::CardSlotAnswer {
                        moment_id: MomentId::new(MOMENT),
                        surface_id: surface(),
                        slot_id: format!("[approval,{}]", world.key("Button")),
                        outcome: CardSlotOutcome::Confirm,
                        seen,
                    }))
                    .0
            };
            assert!(matches!(answer(&mut world), DispatchOutcome::Refused(_)));
            assert_eq!(world.count("card.session.slot"), 0);

            let mut batch = world.own_batch(Vec::new());
            batch.host_slots = vec![slot(Some(json!({
                "requester": "p-orbit",
                "scope": [{ "name": "task", "value": "t-1" }],
            })))];
            world.reply(vec![DomainEvent::Card(CardEvent::Batch {
                surface_id: surface(),
                batch: Box::new(batch),
                draft_conflict: false,
                own_reply: false,
            })]);
            assert!(matches!(answer(&mut world), DispatchOutcome::Submitted(_)));
            assert_eq!(world.count("card.session.slot"), 1);
        }

        #[test]
        fn a_lost_connection_lets_cards_open_again() {
            let mut world = World::open();
            world.type_text("Grace");
            world.reply(vec![DomainEvent::SourceStatusChanged(
                ramflux_desktop_core::SourceStatus::Disconnected,
            )]);
            assert!(world.session.store().card(&surface()).is_none());
            assert!(world.session.card_open_pending(&surface()), "the card is opened again");
        }
    }
}
