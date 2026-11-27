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
    /// Cards with a host event call in flight.
    busy: BTreeSet<SurfaceId>,
    /// Host events waiting for the call in flight on their card, in order.
    queued: std::collections::BTreeMap<SurfaceId, Vec<QueuedEvent>>,
    /// Suspended cards whose resume was asked for waiting host events.
    resuming: BTreeSet<SurfaceId>,
}

/// A host event on a card that waits behind the event call in flight on its
/// card. A text draft replaces an older draft of the same key (the whole
/// value, the newest wins); a tap waits behind every draft of its press.
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
        self.busy.contains(surface) || self.queued.get(surface).is_some_and(|q| !q.is_empty())
    }

    /// Queues a host event; a draft replaces the queued draft of its key.
    fn enqueue(&mut self, event: QueuedEvent) {
        let queue = self.queued.entry(event.surface_id.clone()).or_default();
        if event.is_input()
            && let Some(slot) = queue.iter_mut().find(|q| q.is_input() && q.key == event.key)
        {
            *slot = event;
            return;
        }
        queue.push(event);
    }
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
    fn dispatch_card_due(&mut self) -> Vec<InspectorEntry> {
        let mut records = Vec::new();
        for due in self.store.take_card_due() {
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
    /// A host event on a card waits while another event call on that card is
    /// in flight (see [`QueuedEvent`]).
    pub fn dispatch(&mut self, intent: Intent) -> (DispatchOutcome, Vec<InspectorEntry>) {
        if let Some(mut event) = QueuedEvent::of(&intent.kind) {
            let suspended = self.card_suspended(&event.surface_id);
            if suspended {
                event.pre_resume = self.pre_resume_version(&event);
            }
            let holds = suspended || self.card_marks.borrow().holds(&event.surface_id);
            if holds {
                self.card_marks.borrow_mut().enqueue(event);
                // Sent now when no call is in flight on the card.
                let records = self.dispatch_card_queue();
                return (DispatchOutcome::Submitted(Vec::new()), records);
            }
        }
        self.dispatch_now(intent)
    }

    /// Dispatches waiting host events, one per card with no call in flight.
    fn dispatch_card_queue(&mut self) -> Vec<InspectorEntry> {
        let mut records = Vec::new();
        loop {
            let ready = {
                let marks = self.card_marks.borrow();
                marks
                    .queued
                    .iter()
                    .find(|(surface, queue)| !queue.is_empty() && !marks.busy.contains(*surface))
                    .map(|(surface, _)| surface.clone())
            };
            let Some(surface) = ready else { break };
            if self.card_suspended(&surface) {
                // The resume consumes a tap, so the events wait for it and
                // are sent on the resumed tree (see `rebase_after_resume`).
                let Some(moment_id) = self.queued_moment(&surface) else {
                    self.card_marks.borrow_mut().queued.remove(&surface);
                    continue;
                };
                {
                    let mut marks = self.card_marks.borrow_mut();
                    marks.busy.insert(surface.clone());
                    marks.resuming.insert(surface.clone());
                }
                let refresh = Intent::human(IntentKind::CardRefresh {
                    moment_id,
                    surface_id: surface.clone(),
                });
                let (outcome, recorded) = self.dispatch_now(refresh);
                records.extend(recorded);
                if !matches!(&outcome, DispatchOutcome::Submitted(labels) if !labels.is_empty()) {
                    let mut marks = self.card_marks.borrow_mut();
                    marks.busy.remove(&surface);
                    marks.resuming.remove(&surface);
                    marks.queued.remove(&surface);
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
            let was_input = next.is_input();
            // A draft or tap taken on this desktop's own chain of stamps
            // follows the held version; any other stamp is checked as it is.
            if let Some(seen) = self.store.card_rebased(&next.surface_id, &next.key, &next.seen) {
                next.seen = seen;
            }
            let (outcome, recorded) = self.dispatch_now(next.into_intent());
            records.extend(recorded);
            // A draft that was not taken means a tap behind it must not
            // fire without the value the user typed: the queue is dropped.
            if was_input && matches!(outcome, DispatchOutcome::Refused(_)) {
                self.card_marks.borrow_mut().queued.remove(&surface);
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

    /// The signed version an event is valid on in the held (suspended) tree:
    /// its stamp names the held version, or a text draft of this desktop's
    /// own chain that may be rebased.
    fn pre_resume_version(&self, event: &QueuedEvent) -> Option<u64> {
        let record = self.store.card(&event.surface_id)?;
        let tree = record.tree.as_ref()?;
        let valid = event.seen.stamp == tree.stamp()
            || (event.is_input() && record.rebasable(&event.key, &event.seen));
        valid.then(|| tree.signed_version())
    }

    /// Rebases the events that waited for a resume onto the resumed tree:
    /// all of them when the signed version is unchanged, a text draft whose
    /// node keeps its kind and input target otherwise (the whole value
    /// replaces the draft); anything else is dropped, the user acts again
    /// on the card as it is.
    fn rebase_after_resume(&mut self, surface: &SurfaceId) {
        let Some(tree) = self.store.card(surface).and_then(|r| r.tree.as_ref()) else {
            self.card_marks.borrow_mut().queued.remove(surface);
            return;
        };
        let stamp = tree.stamp();
        let signed = tree.signed_version();
        let mut marks = self.card_marks.borrow_mut();
        let Some(queue) = marks.queued.get_mut(surface) else { return };
        queue.retain_mut(|q| {
            let same_signed = q.pre_resume == Some(signed);
            let draft_ok = q.pre_resume.is_some()
                && q.is_input()
                && tree.node(&q.key).is_some_and(|n| {
                    n.kind == q.seen.kind
                        && n.attrs.get("input_pointer").and_then(serde_json::Value::as_str)
                            == q.seen.input_pointer.as_deref()
                });
            if same_signed || draft_ok {
                q.seen.stamp = stamp;
                q.pre_resume = None;
                true
            } else {
                false
            }
        });
        if queue.is_empty() {
            marks.queued.remove(surface);
        }
    }

    fn dispatch_now(&mut self, intent: Intent) -> (DispatchOutcome, Vec<InspectorEntry>) {
        let mark = card_mark(&intent.kind);
        let user_surface = card_user_surface(&intent.kind).cloned();
        let (decided, mut records) = self.decide(intent);
        let effects = match decided {
            Ok(effects) => effects,
            Err(outcome) => {
                if let (DispatchOutcome::Refused(IntentError::CardChanged(_)), Some(surface)) =
                    (&outcome, &user_surface)
                {
                    self.store.card_refuse(surface, CARD_CHANGED_REASON);
                }
                return (outcome, records);
            }
        };
        self.note_card_mark(mark);
        if let Some(surface) = &user_surface {
            self.store.card_bump(surface);
        }
        let mut labels = Vec::new();
        for effect in effects {
            if let Some(surface) = &user_surface
                && is_event_call(&effect)
            {
                self.card_marks.borrow_mut().busy.insert(surface.clone());
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
        if let DomainEvent::Card(card) = &env.event {
            let surface = card.surface_id();
            let mut marks = self.card_marks.borrow_mut();
            marks.opening.remove(surface);
            match card {
                CardEvent::Disposed { .. } => {
                    marks.disposing.remove(surface);
                    marks.busy.remove(surface);
                    marks.queued.remove(surface);
                }
                CardEvent::Refused { .. } => {
                    // A refused dispose is not in flight any more: the pool
                    // may ask again. Events waiting behind a refused one are
                    // dropped; the user acts again on the card as it is.
                    marks.disposing.remove(surface);
                    marks.busy.remove(surface);
                    marks.queued.remove(surface);
                }
                CardEvent::Stale { .. } | CardEvent::Fallback { .. } => {
                    marks.busy.remove(surface);
                    marks.queued.remove(surface);
                }
                CardEvent::Batch { own_reply: true, .. }
                | CardEvent::PendingSubmit { .. }
                | CardEvent::OpenUrl { .. } => {
                    marks.busy.remove(surface);
                }
                _ => {}
            }
            drop(marks);
            // The browser opens only for the link the sheet showed: the one
            // the store holds for this card.
            if let CardEvent::LinkOpened { url, .. } = card
                && self
                    .store
                    .card(surface)
                    .and_then(|record| record.link.as_ref())
                    .is_some_and(|link| &link.url == url)
            {
                self.open_urls.push(url.clone());
            }
        }
        let result = self.store.apply(env);
        if let DomainEvent::Card(card) = &env.event {
            let surface = card.surface_id();
            if matches!(card, CardEvent::Resumed { .. })
                && self.card_marks.borrow_mut().resuming.remove(surface)
            {
                self.rebase_after_resume(surface);
                self.card_marks.borrow_mut().busy.remove(surface);
            } else if matches!(
                card,
                CardEvent::Refused { .. }
                    | CardEvent::Stale { .. }
                    | CardEvent::Fallback { .. }
                    | CardEvent::Disposed { .. }
            ) {
                self.card_marks.borrow_mut().resuming.remove(surface);
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
}
