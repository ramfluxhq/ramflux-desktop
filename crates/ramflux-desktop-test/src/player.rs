// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Timeline control over a store fed by a [`MockSource`].

use ramflux_desktop_core::{
    ApplyError, Effect, EventEnvelope, Intent, IntentError, Source, Store, handle_intent,
};

use crate::mock_source::MockSource;
use crate::scenario::Scenario;

/// Drives a store through a scenario with pause, step, advance, and seek.
#[derive(Debug)]
pub struct ScenarioPlayer {
    source: MockSource,
    store: Store,
    now_ms: u64,
    paused: bool,
    applied: u64,
    errors: Vec<ApplyError>,
}

impl ScenarioPlayer {
    /// Creates a player at offset 0 with nothing applied.
    pub fn new(scenario: &Scenario) -> Self {
        Self {
            source: MockSource::new(scenario),
            store: Store::new(),
            now_ms: 0,
            paused: false,
            applied: 0,
            errors: Vec::new(),
        }
    }

    /// The store being driven.
    pub fn store(&self) -> &Store {
        &self.store
    }

    /// The underlying source.
    pub fn source(&self) -> &MockSource {
        &self.source
    }

    /// Current offset in ms.
    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }

    /// Number of envelopes applied without error.
    pub fn applied(&self) -> u64 {
        self.applied
    }

    /// Apply errors since the last seek.
    pub fn errors(&self) -> &[ApplyError] {
        &self.errors
    }

    /// Whether time is frozen.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Freezes time; [`ScenarioPlayer::advance`] becomes a no-op.
    pub fn pause(&mut self) {
        self.paused = true;
    }

    /// Unfreezes time.
    pub fn resume(&mut self) {
        self.paused = false;
    }

    /// Moves time forward by `ms` and applies due events. Returns the number
    /// of envelopes delivered; 0 while paused.
    pub fn advance(&mut self, ms: u64) -> usize {
        if self.paused {
            return 0;
        }
        self.now_ms = self.now_ms.saturating_add(ms);
        self.pump()
    }

    /// Applies exactly the next scheduled event, paused or not, moving time
    /// to it if it lies ahead. Returns false when nothing is scheduled.
    pub fn step(&mut self) -> bool {
        match self.source.drain_one() {
            Some(env) => {
                self.now_ms = self.now_ms.max(self.source.now_ms());
                self.apply(&env);
                true
            }
            None => false,
        }
    }

    /// Resets the store and replays everything scheduled up to `ms`,
    /// including responses triggered before the seek.
    pub fn seek(&mut self, ms: u64) -> usize {
        self.source.rewind();
        self.store = Store::new();
        self.applied = 0;
        self.errors.clear();
        self.now_ms = ms;
        self.pump()
    }

    /// Applies every scheduled event regardless of time and pause.
    pub fn run_to_end(&mut self) -> usize {
        let mut delivered = 0;
        while self.step() {
            delivered += 1;
        }
        delivered
    }

    /// Validates an intent against the store, submits the resulting calls to
    /// the source, and returns the effects the app must handle itself
    /// (confirmation requests).
    ///
    /// # Errors
    ///
    /// Returns [`IntentError`] when the intent is refused.
    pub fn dispatch(&mut self, intent: Intent) -> Result<Vec<Effect>, IntentError> {
        let mut for_app = Vec::new();
        for effect in handle_intent(&self.store, intent)? {
            match effect {
                Effect::RequireConfirmation { .. } => for_app.push(effect),
                Effect::Call { .. } | Effect::Unlock { .. } => self.source.submit(effect),
            }
        }
        Ok(for_app)
    }

    fn pump(&mut self) -> usize {
        let now = self.source.epoch().saturating_add_ms(self.now_ms);
        let envelopes = self.source.drain(now);
        for env in &envelopes {
            self.apply(env);
        }
        envelopes.len()
    }

    fn apply(&mut self, env: &EventEnvelope) {
        match self.store.apply(env) {
            Ok(_) => self.applied += 1,
            Err(error) => self.errors.push(error),
        }
    }
}
