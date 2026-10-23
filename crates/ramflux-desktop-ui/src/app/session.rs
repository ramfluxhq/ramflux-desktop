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
//! The session declares this client's renderer profile to its source when
//! it starts, and checks every surface an envelope changed against it: new
//! renderer errors are logged ([`RendererErrors`]) and reported to the
//! source when the backend can carry the report.

use std::collections::BTreeSet;

use ramflux_desktop_core::{
    ContextId, Effect, EventEnvelope, Intent, IntentError, MomentId, Source, Store, Timestamp,
    handle_intent,
};
use ramflux_desktop_test::{MockSource, Scenario};

use super::changes::ChangeSet;
use super::inspector::InspectorEntry;
use super::names::{effect_label, event_name, intent_name};
use super::renderer_errors::RendererErrors;

/// What one clock movement or step applied.
#[derive(Debug, Default)]
pub struct Applied {
    /// Merged changes.
    pub changes: ChangeSet,
    /// Inspector records for the applied envelopes.
    pub records: Vec<InspectorEntry>,
}

/// The policy result of a dispatched intent.
#[derive(Debug)]
pub enum DispatchOutcome {
    /// Effects went to the source; labels for display.
    Submitted(Vec<String>),
    /// Policy requires the hold-to-confirm strip first. Nothing was submitted.
    NeedsConfirmation(Box<Intent>),
    /// The intent was refused.
    Refused(IntentError),
    /// There is no active account to dispatch to.
    NoAccount,
}

/// What feeds a session.
enum Backing {
    /// A scenario-playing mock with its own clock.
    Mock { scenario: String, source: Box<MockSource> },
    /// A live source on the wall clock.
    Live { label: String, source: Box<dyn Source> },
}

/// A store fed by a scenario-playing or live source.
pub struct AccountSession {
    backing: Backing,
    store: Store,
    clock_ms: u64,
    paused: bool,
    /// Message bodies asked of the source and not delivered yet (asked once).
    requested: BTreeSet<MomentId>,
    renderer_errors: RendererErrors,
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
    pub fn new(scenario: &Scenario) -> Self {
        Self::with_epoch(scenario, scenario.epoch)
    }

    /// Creates a session whose offset 0 is `epoch` instead of the scenario's own.
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

    fn start(backing: Backing) -> Self {
        let mut session = Self {
            backing,
            store: Store::new(),
            clock_ms: 0,
            paused: false,
            requested: BTreeSet::new(),
            renderer_errors: RendererErrors::default(),
        };
        let profile = crate::moments::surface::registry::profile();
        session.source_mut().renderer_profile(profile);
        session
    }

    /// Renderer errors found in this session.
    pub fn renderer_errors(&self) -> &RendererErrors {
        &self.renderer_errors
    }

    /// The renderer profile the mock source received (`None` when live).
    pub fn negotiated_profile(&self) -> Option<&ramflux_desktop_core::RendererProfile> {
        self.mock().and_then(MockSource::negotiated_profile)
    }

    /// Whether a live source feeds the session.
    pub fn is_live(&self) -> bool {
        matches!(self.backing, Backing::Live { .. })
    }

    /// Name of the scenario this session plays, or the live source's label.
    pub fn scenario_name(&self) -> &str {
        match &self.backing {
            Backing::Mock { scenario, .. } => scenario,
            Backing::Live { label, .. } => label,
        }
    }

    fn mock(&self) -> Option<&MockSource> {
        match &self.backing {
            Backing::Mock { source, .. } => Some(source),
            Backing::Live { .. } => None,
        }
    }

    fn source_mut(&mut self) -> &mut dyn Source {
        match &mut self.backing {
            Backing::Mock { source, .. } => source.as_mut(),
            Backing::Live { source, .. } => source.as_mut(),
        }
    }

    /// Tells the source which context the user looks at.
    pub fn focus(&mut self, context: Option<&ContextId>) {
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
        match self.mock() {
            Some(source) => source.epoch().saturating_add_ms(self.clock_ms),
            None => super::wall_clock_now(),
        }
    }

    /// Offset of the next scheduled event, if any (scenarios only).
    pub fn next_due(&self) -> Option<u64> {
        self.mock().and_then(MockSource::next_due)
    }

    /// Number of effects a mock source received (0 when live).
    pub fn submitted_effects(&self) -> usize {
        self.mock().map_or(0, |source| source.effects().len())
    }

    /// Labels of every effect a mock source received, oldest first.
    pub fn effect_labels(&self) -> impl Iterator<Item = &str> {
        self.mock().into_iter().flat_map(|s| s.effects().iter().map(|r| r.label.as_str()))
    }

    /// Every effect a mock source received with its (secret-free) body, oldest first.
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
        if (self.paused && !self.is_live()) || ms == 0 {
            return Applied::default();
        }
        self.clock_ms = self.clock_ms.saturating_add(ms);
        let mut applied = self.pump();
        applied.changes.mark_clock();
        applied
    }

    /// Applies exactly the next scheduled envelope, paused or not, moving the
    /// clock to it when it lies ahead.
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
    pub fn seek(&mut self, ms: u64) -> Applied {
        let Backing::Mock { source, .. } = &mut self.backing else { return Applied::default() };
        source.rewind();
        self.store = Store::new();
        self.requested.clear();
        self.renderer_errors.clear();
        self.clock_ms = ms;
        let mut applied = self.pump();
        let mut changes = ChangeSet::reset();
        changes.mark_clock();
        applied.changes = changes;
        applied
    }

    /// Runs policy for `intent` at the session time and submits the
    /// resulting effects. A confirmation request is handed back unsubmitted.
    pub fn dispatch(&mut self, intent: Intent) -> (DispatchOutcome, Vec<InspectorEntry>) {
        let (decided, mut records) = self.decide(intent);
        let effects = match decided {
            Ok(effects) => effects,
            Err(outcome) => return (outcome, records),
        };
        let mut labels = Vec::new();
        for effect in effects {
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
        if !self.requested.is_empty() {
            let store = &self.store;
            self.requested.retain(|id| store.body(id).is_none() && store.moment(id).is_some());
        }
        applied
    }

    fn apply(&mut self, env: &EventEnvelope, applied: &mut Applied) {
        let result = self.store.apply(env);
        applied.records.push(InspectorEntry::Envelope {
            seq: env.seq,
            at_ms: env.at.saturating_since(self.mock().map_or(Timestamp(0), MockSource::epoch)),
            event: event_name(&env.event),
            error: result.as_ref().err().map(ToString::to_string),
        });
        if let Ok(changes) = result {
            let errors = self.renderer_errors.check(&self.store, &changes.moments);
            self.report(errors, applied);
            applied.changes.merge(changes);
        }
    }

    /// Reports new renderer errors to the source, when the backend carries
    /// the report (a backend without it declares the method unsupported).
    fn report(&mut self, errors: Vec<ramflux_desktop_core::RendererError>, applied: &mut Applied) {
        if errors.is_empty()
            || self
                .store
                .unsupported_reason(ramflux_desktop_core::methods::RENDERER_ERROR)
                .is_some()
        {
            return;
        }
        for error in errors {
            let effect = error.to_effect();
            applied.records.push(InspectorEntry::Effect { label: effect_label(&effect) });
            self.submit(effect);
        }
    }
}

/// The audit pair of an intent as an inspector label, e.g.
/// `Agent(p-orbit), confirmed by Hold`.
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
    use ramflux_desktop_core::{
        Actor, ConfirmationIssuer, GrantId, GrantState, IntentKind, ParticipantId, methods,
    };
    use ramflux_desktop_test::load_fixture;

    use super::*;

    fn session(name: &str) -> AccountSession {
        AccountSession::new(&load_fixture(name).expect("fixture"))
    }

    fn hold(ms: u64) -> ramflux_desktop_core::HumanConfirmation {
        ConfirmationIssuer::new().issue_hold(ms).expect("a full hold")
    }

    fn approve(actor: Actor, confirmed: bool) -> Intent {
        let intent = Intent {
            proposed_by: actor,
            confirmed_by: None,
            kind: IntentKind::Approve { grant_id: GrantId::new("g-rotate") },
        };
        if confirmed { intent.confirmed_by_human(hold(1_200)) } else { intent }
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
    fn high_risk_approval_is_held_until_a_human_hold_for_humans_and_agents() {
        let mut s = session("s03_intervention");
        s.advance(1_000);
        let agent = Actor::Agent(ParticipantId::new("p-orbit"));
        let (outcome, _) = s.dispatch(approve(agent.clone(), false));
        let DispatchOutcome::NeedsConfirmation(intent) = outcome else {
            panic!("an unconfirmed agent proposal waits for the hold");
        };
        assert_eq!(intent.proposed_by, agent);
        let (outcome, records) = s.dispatch(intent.confirmed_by_human(hold(1_300)));
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
