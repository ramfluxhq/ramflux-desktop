// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A scenario-playing [`Source`].

use std::collections::BTreeMap;
use std::ops::Bound;

use ramflux_desktop_core::{DomainEvent, Effect, EventEnvelope, Source, Timestamp};

use crate::scenario::{ResponseRule, Scenario};

/// Ordering key of a scheduled event: (offset ms, insertion order).
type Key = (u64, u64);

/// A submitted effect as seen by the mock. Secrets are never recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectRecord {
    /// Offset at which it was submitted.
    pub at_ms: u64,
    /// Method name, `"RequireConfirmation"`, or `"Unlock"`.
    pub label: String,
    /// Index of the matching response rule.
    pub rule: Option<usize>,
}

/// Plays a scenario as a source.
///
/// Every event lives in one schedule keyed by offset. Delivery assigns
/// increasing `seq` from 1 and `at = epoch + offset`, and stamps `at` on
/// added moments, so fixtures need not repeat timestamps. Responses are
/// scheduled into the same schedule, which makes [`MockSource::rewind`] replay
/// them deterministically.
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
        let mut source = Self {
            epoch: scenario.epoch,
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
        if let DomainEvent::MomentAdded(moment) = &mut event {
            moment.at = at;
        }
        EventEnvelope::new(self.seq, at, event)
    }
}

impl Source for MockSource {
    fn submit(&mut self, effect: Effect) {
        let label = match &effect {
            Effect::Call { method, .. } => method.clone(),
            Effect::RequireConfirmation { .. } => "RequireConfirmation".to_owned(),
            Effect::Unlock { .. } => "Unlock".to_owned(),
        };
        let rule = self.rules.iter().position(|r| r.on.matches(&effect));
        self.effects.push(EffectRecord { at_ms: self.now_ms, label, rule });
        if let Some(index) = rule {
            let emit = self.rules[index].emit.clone();
            for (delay, event) in emit {
                self.inject(delay, event);
            }
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
