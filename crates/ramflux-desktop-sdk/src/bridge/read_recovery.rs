// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! One delayed retry per failed fetch, independent of the rendering clock.

use crate::plan::fetch::Fetch;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;
use tokio::task::AbortHandle;
use tokio::time::Instant;

#[derive(Clone, Copy)]
enum Retry {
    Waiting(Instant),
    Retried,
    Exhausted,
}

#[derive(Default)]
pub(super) struct ReadRecovery {
    retries: BTreeMap<Fetch, Retry>,
    pub(super) queued: BTreeSet<Fetch>,
    pub(super) running: BTreeMap<Fetch, AbortHandle>,
    /// A newer action/event needs a read after the in-flight answer.
    pub(super) rerun: BTreeSet<Fetch>,
}

impl ReadRecovery {
    /// Returns true only when the second failure should be reported.
    pub(super) fn failed(&mut self, fetch: &Fetch, now: Instant) -> bool {
        match self.retries.get(fetch) {
            None => {
                self.retries.insert(fetch.clone(), Retry::Waiting(now + Duration::from_secs(1)));
                false
            }
            Some(Retry::Retried) => {
                self.retries.insert(fetch.clone(), Retry::Exhausted);
                true
            }
            Some(Retry::Waiting(_) | Retry::Exhausted) => false,
        }
    }

    pub(super) fn succeeded(&mut self, fetch: &Fetch) {
        self.retries.remove(fetch);
    }

    pub(super) fn rearm(&mut self, fetch: &Fetch) {
        self.retries.remove(fetch);
    }

    pub(super) fn blocks_timer(&self, fetch: &Fetch) -> bool {
        matches!(self.retries.get(fetch), Some(Retry::Waiting(_) | Retry::Exhausted))
    }

    pub(super) fn next_due(&self) -> Option<Instant> {
        self.retries
            .values()
            .filter_map(|state| match state {
                Retry::Waiting(at) => Some(*at),
                _ => None,
            })
            .min()
    }

    pub(super) fn due(&mut self, now: Instant) -> Vec<Fetch> {
        let mut due = Vec::new();
        for (fetch, state) in &mut self.retries {
            if matches!(state, Retry::Waiting(at) if now >= *at) {
                *state = Retry::Retried;
                due.push(fetch.clone());
            }
        }
        due
    }

    pub(super) fn cancel(&mut self) {
        self.retries.clear();
        self.queued.clear();
        self.rerun.clear();
        for (_, task) in std::mem::take(&mut self.running) {
            task.abort();
        }
    }
}
