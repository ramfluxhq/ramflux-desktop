// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The renderer error log of one account session.
//!
//! After every applied envelope the session checks the surfaces it changed
//! against this client's renderer profile. Each new error (one per moment,
//! block and kind, however many versions repeat it) is logged here, counted
//! for Settings › Diagnostics, and handed back so the session can report it
//! to the surface's producer when the backend carries that report. Records
//! name ids and kinds only; block data never enters the log.

use std::collections::{BTreeMap, BTreeSet};

use ramflux_desktop_core::conformance::RendererErrorKind;
use ramflux_desktop_core::{BlockId, MomentId, MomentKind, RendererError, Store};

use crate::moments::surface::verdict as status_verdict;

/// Renderer errors kept for display.
pub const MAX_RENDERER_ERRORS: usize = 64;

/// Distinct errors remembered for de-duplication before the memory resets.
const MAX_SEEN: usize = 4_096;

type Key = (MomentId, Option<BlockId>, RendererErrorKind);

/// Renderer errors found in one session.
#[derive(Debug, Default)]
pub struct RendererErrors {
    recent: Vec<RendererError>,
    seen: BTreeSet<Key>,
    counts: BTreeMap<&'static str, u64>,
    total: u64,
}

impl RendererErrors {
    /// Checks the surfaces among `moments` and returns the errors not seen
    /// before.
    pub fn check<'a>(
        &mut self,
        store: &Store,
        moments: impl IntoIterator<Item = &'a MomentId>,
    ) -> Vec<RendererError> {
        let mut found = Vec::new();
        for id in moments {
            let Some(moment) = store.moment(id) else { continue };
            let MomentKind::AgentSurface(surface) = &moment.kind else { continue };
            // The store's own verdict too: a surface refused over a limit.
            let verdict = status_verdict(store, id, surface);
            if store.surface_conflict(id) {
                found.push(RendererError {
                    moment_id: id.clone(),
                    surface_id: surface.surface_id.clone(),
                    surface_hash: verdict.hash.clone(),
                    block_id: None,
                    block_kind: None,
                    kind: RendererErrorKind::StaleBase,
                });
            }
            found.extend(verdict.errors);
        }
        found.retain(|error| self.remember(error));
        found
    }

    fn remember(&mut self, error: &RendererError) -> bool {
        if self.seen.len() >= MAX_SEEN {
            self.seen.clear();
        }
        let key = (error.moment_id.clone(), error.block_id.clone(), error.kind);
        if !self.seen.insert(key) {
            return false;
        }
        if self.recent.len() == MAX_RENDERER_ERRORS {
            self.recent.remove(0);
        }
        self.recent.push(error.clone());
        *self.counts.entry(error.kind.name()).or_default() += 1;
        self.total += 1;
        true
    }

    /// Forgets everything (the store was reset).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Most recent errors, oldest first.
    pub fn recent(&self) -> &[RendererError] {
        &self.recent
    }

    /// Errors found so far.
    pub fn total(&self) -> u64 {
        self.total
    }

    /// Errors found so far, by kind name.
    pub fn counts(&self) -> &BTreeMap<&'static str, u64> {
        &self.counts
    }
}
