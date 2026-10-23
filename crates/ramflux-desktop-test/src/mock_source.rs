// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A scenario-playing [`Source`].

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;

use ramflux_desktop_core::{
    DomainEvent, Effect, EventEnvelope, JsonValue, MomentId, MomentKind, PrivacyRevealPayload,
    Source, Timestamp,
};

use crate::local_echo::{ECHO_DELAY_MS, audit_echo, local_echo};
use crate::scenario::{ResponseRule, Scenario};

/// Ordering key of a scheduled event: (offset ms, insertion order).
type Key = (u64, u64);

/// A submitted effect as seen by the mock. Secrets are never recorded.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectRecord {
    /// Offset at which it was submitted.
    pub at_ms: u64,
    /// Method name, or `"RequireConfirmation"`.
    pub label: String,
    /// Index of the matching response rule.
    pub rule: Option<usize>,
    /// Whether the built-in local echo answered it.
    pub echoed: bool,
    /// The call body (never contains secrets; secret calls carry them beside it).
    pub body: JsonValue,
}

/// Plays a scenario as a source.
///
/// Every event lives in one schedule keyed by offset. Delivery assigns
/// increasing `seq` from 1 and `at = epoch + offset`. An added moment written
/// with `at: 0` is stamped with the delivery time; any other `at` is a signed
/// offset from the epoch, so fixtures can carry history (for example
/// yesterday's messages) that stays relative when the epoch moves. Reveal
/// records and node probes written with `at: 0` are stamped the same way.
/// Responses are scheduled into the same schedule, which makes
/// [`MockSource::rewind`] replay them deterministically.
///
/// Effects no scenario rule answers fall back to the built-in local echo
/// (see [`crate::local_echo`]); confirmed calls are always audited.
#[derive(Debug)]
pub struct MockSource {
    epoch: Timestamp,
    rules: Vec<ResponseRule>,
    schedule: BTreeMap<Key, DomainEvent>,
    next_order: u64,
    cursor: Option<Key>,
    seq: u64,
    now_ms: u64,
    effects: Vec<EffectRecord>,
}

impl MockSource {
    /// Creates a source for `scenario`.
    pub fn new(scenario: &Scenario) -> Self {
        Self::with_epoch(scenario, scenario.epoch)
    }

    /// Creates a source for `scenario` with offset 0 at `epoch` instead of
    /// the scenario's own epoch (the app anchors live playback to the wall clock).
    pub fn with_epoch(scenario: &Scenario, epoch: Timestamp) -> Self {
        let mut source = Self {
            epoch,
            rules: scenario.responses.clone(),
            schedule: BTreeMap::new(),
            next_order: 0,
            cursor: None,
            seq: 0,
            now_ms: 0,
            effects: Vec::new(),
        };
        for (offset, event) in &scenario.timeline {
            source.schedule_at(*offset, event.clone());
        }
        source
    }

    /// Scenario epoch.
    pub fn epoch(&self) -> Timestamp {
        self.epoch
    }

    /// Current offset in ms.
    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }

    /// Effects submitted so far.
    pub fn effects(&self) -> &[EffectRecord] {
        &self.effects
    }

    /// Schedules one extra event `delay_ms` after the current offset.
    pub fn inject(&mut self, delay_ms: u64, event: DomainEvent) {
        self.schedule_at(self.now_ms.saturating_add(delay_ms), event);
    }

    /// Offset of the next undelivered event.
    pub fn next_due(&self) -> Option<u64> {
        self.pending().next().map(|((due, _), _)| *due)
    }

    /// Delivers exactly the next undelivered event, moving time to its offset.
    pub fn drain_one(&mut self) -> Option<EventEnvelope> {
        let (key, event) = self.pending().next().map(|(k, e)| (*k, e.clone()))?;
        self.now_ms = self.now_ms.max(key.0);
        Some(self.deliver(key, event))
    }

    /// Restarts delivery from offset 0, keeping every scheduled event,
    /// including responses already triggered.
    pub fn rewind(&mut self) {
        self.cursor = None;
        self.seq = 0;
        self.now_ms = 0;
    }

    fn schedule_at(&mut self, offset: u64, event: DomainEvent) {
        self.schedule.insert((offset, self.next_order), event);
        self.next_order += 1;
    }

    fn pending(&self) -> impl Iterator<Item = (&Key, &DomainEvent)> {
        let lower = self.cursor.map_or(Bound::Unbounded, Bound::Excluded);
        self.schedule.range((lower, Bound::Unbounded))
    }

    fn deliver(&mut self, key: Key, mut event: DomainEvent) -> EventEnvelope {
        self.cursor = Some(key);
        self.seq += 1;
        let at = self.epoch.saturating_add_ms(key.0);
        match &mut event {
            DomainEvent::MomentAdded(moment) => {
                moment.at = if moment.at == Timestamp(0) {
                    at
                } else {
                    Timestamp(self.epoch.millis().saturating_add(moment.at.millis()))
                };
            }
            DomainEvent::MomentUpdated {
                kind:
                    Some(MomentKind::PrivacyReveal(PrivacyRevealPayload {
                        revealed: Some(record), ..
                    })),
                ..
            } if record.at == Timestamp(0) => record.at = at,
            DomainEvent::NodeProbed { probe, .. } if probe.at == Timestamp(0) => probe.at = at,
            _ => {}
        }
        EventEnvelope::new(self.seq, at, event)
    }
}

impl Source for MockSource {
    fn submit(&mut self, effect: Effect) {
        let label = effect.method().unwrap_or("RequireConfirmation").to_owned();
        let rule = self.rules.iter().position(|r| r.on.matches(&effect));
        let echo = if rule.is_none() { local_echo(&effect) } else { Vec::new() };
        let echoed = !echo.is_empty();
        let body = effect.body().cloned().unwrap_or_default();
        self.effects.push(EffectRecord { at_ms: self.now_ms, label, rule, echoed, body });
        if let Some(index) = rule {
            let emit = self.rules[index].emit.clone();
            for (delay, event) in emit {
                self.inject(delay, event);
            }
        }
        for event in echo {
            self.inject(ECHO_DELAY_MS, event);
        }
        let at = self.epoch.saturating_add_ms(self.now_ms.saturating_add(ECHO_DELAY_MS));
        if let Some(event) = audit_echo(&effect, at) {
            self.inject(ECHO_DELAY_MS, event);
        }
    }

    /// Delivers the bodies again from the newest message payload already
    /// delivered for each moment, as `MomentUpdated` events due now.
    fn request_bodies(&mut self, moments: &[MomentId]) {
        let mut wanted: BTreeSet<&MomentId> = moments.iter().collect();
        let Some(cursor) = self.cursor else { return };
        let mut found = Vec::new();
        for event in self.schedule.range(..=cursor).rev().map(|(_, e)| e) {
            if wanted.is_empty() {
                break;
            }
            let (id, kind) = match event {
                DomainEvent::MomentAdded(m) => (&m.id, Some(&m.kind)),
                DomainEvent::MomentUpdated { moment_id, kind, .. } => (moment_id, kind.as_ref()),
                _ => continue,
            };
            if let Some(kind @ MomentKind::Message(_)) = kind
                && wanted.remove(id)
            {
                found.push(DomainEvent::MomentUpdated {
                    moment_id: id.clone(),
                    lifecycle: None,
                    kind: Some(kind.clone()),
                });
            }
        }
        for event in found {
            self.inject(0, event);
        }
    }

    fn drain(&mut self, now: Timestamp) -> Vec<EventEnvelope> {
        self.now_ms = now.saturating_since(self.epoch);
        let due: Vec<(Key, DomainEvent)> = self
            .pending()
            .take_while(|((offset, _), _)| *offset <= self.now_ms)
            .map(|(k, e)| (*k, e.clone()))
            .collect();
        due.into_iter().map(|(key, event)| self.deliver(key, event)).collect()
    }
}
