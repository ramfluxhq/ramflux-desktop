// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! One delayed retry per failed fetch, independent of the rendering clock.

use crate::plan::fetch::Fetch;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Duration;
use tokio::task::AbortHandle;
use tokio::time::Instant;

/// Maximum concurrent follow-up reads in a scope.
pub(super) const MAX_READS: usize = 8;

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
    queue: VecDeque<Fetch>,
    focus_queue: VecDeque<Fetch>,
    prioritized: BTreeSet<Fetch>,
    pub(super) running: BTreeMap<Fetch, AbortHandle>,
    /// A newer action/event needs a read after the in-flight answer.
    pub(super) rerun: BTreeSet<Fetch>,
}

impl ReadRecovery {
    /// Promote an already queued read, or remember priority for its next run.
    pub(super) fn prioritize(&mut self, fetch: &Fetch) {
        if self.prioritized.insert(fetch.clone()) && self.queued.contains(fetch) {
            self.queue.retain(|queued| queued != fetch);
            self.focus_queue.push_back(fetch.clone());
        }
    }

    pub(super) fn enqueue(&mut self, fetch: Fetch) {
        if self.queued.insert(fetch.clone()) {
            if self.prioritized.contains(&fetch) {
                self.focus_queue.push_back(fetch);
            } else {
                self.queue.push_back(fetch);
            }
        }
    }

    pub(super) fn pop_queued(&mut self) -> Option<Fetch> {
        let fetch = self.focus_queue.pop_front().or_else(|| self.queue.pop_front())?;
        self.queued.remove(&fetch);
        Some(fetch)
    }

    pub(super) fn unqueue(&mut self, fetch: &Fetch) {
        if self.queued.remove(fetch) {
            self.queue.retain(|queued| queued != fetch);
            self.focus_queue.retain(|queued| queued != fetch);
        }
        self.prioritized.remove(fetch);
    }

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
        self.queue.clear();
        self.focus_queue.clear();
        self.prioritized.clear();
        self.rerun.clear();
        for (_, task) in std::mem::take(&mut self.running) {
            task.abort();
        }
    }
}
