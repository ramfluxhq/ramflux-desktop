// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Card sessions the desktop holds for one account.
//!
//! The card runtime allows two resident sessions per process and 32 per
//! account. The desktop keeps at most [`MAX_RESUMED`] cards resumed (the
//! most recently touched ones); the others are suspended and keep their last
//! tree. At most [`MAX_OPEN`] cards are held; when a card leaves the loaded
//! timeline window its session is disposed.
//!
//! The pool only decides. It returns the suspend, resume and dispose steps
//! the caller must perform on the bus, and the caller reports the session a
//! resume created with [`CardPool::adopt`] (a resume issues a fresh session
//! id). Recency is a logical clock, never wall time.

use std::collections::{BTreeMap, BTreeSet};

/// Resumed (live) sessions per account.
pub const MAX_RESUMED: usize = 2;
/// Held sessions per account.
pub const MAX_OPEN: usize = 24;

/// One held card session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardEntry {
    /// The session the runtime knows the card by.
    pub session: u64,
    /// Whether the session is resumed (live) or suspended.
    pub resumed: bool,
    used: u64,
}

/// The bus steps a pool change requires, in order: suspend first, then
/// resume, then dispose.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PoolPlan {
    /// Cards to suspend, with their sessions.
    pub suspend: Vec<(String, u64)>,
    /// The card to resume, with its current session.
    pub resume: Option<(String, u64)>,
    /// Cards to dispose, with their sessions.
    pub dispose: Vec<(String, u64)>,
}

impl PoolPlan {
    /// Whether the plan asks for no bus call.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.suspend.is_empty() && self.resume.is_none() && self.dispose.is_empty()
    }
}

/// The card sessions of one account, keyed by the card's surface reference.
#[derive(Debug, Default, Clone)]
pub struct CardPool {
    clock: u64,
    cards: BTreeMap<String, CardEntry>,
}

impl CardPool {
    /// An empty pool.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The held entry of `card`.
    #[must_use]
    pub fn entry(&self, card: &str) -> Option<&CardEntry> {
        self.cards.get(card)
    }

    /// The number of held sessions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cards.len()
    }

    /// Whether no session is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    /// Records a session the runtime just opened for `card`. It is resumed,
    /// and the pool is brought back within its limits without evicting
    /// `card` itself.
    pub fn opened(&mut self, card: &str, session: u64) -> PoolPlan {
        let used = self.tick();
        let previous =
            self.cards.insert(card.to_owned(), CardEntry { session, resumed: true, used });
        let mut plan = PoolPlan::default();
        if let Some(previous) = previous {
            plan.dispose.push((card.to_owned(), previous.session));
        }
        self.enforce(card, &mut plan);
        plan
    }

    /// Marks `card` as touched (a tap or focus). A suspended card is resumed
    /// and the least recently used resumed cards are suspended to stay within
    /// [`MAX_RESUMED`]. An unknown card yields an empty plan.
    pub fn touch(&mut self, card: &str) -> PoolPlan {
        let used = self.tick();
        let mut plan = PoolPlan::default();
        let Some(entry) = self.cards.get_mut(card) else {
            return plan;
        };
        entry.used = used;
        if !entry.resumed {
            entry.resumed = true;
            plan.resume = Some((card.to_owned(), entry.session));
        }
        self.enforce(card, &mut plan);
        plan
    }

    /// Records the session a resume created for `card` (fresh token identity).
    pub fn adopt(&mut self, card: &str, session: u64) {
        if let Some(entry) = self.cards.get_mut(card) {
            entry.session = session;
        }
    }

    /// Records that a suspend of `card` completed.
    pub fn suspended(&mut self, card: &str) {
        if let Some(entry) = self.cards.get_mut(card) {
            entry.resumed = false;
        }
    }

    /// Drops `card` from the pool, returning its session to dispose.
    pub fn remove(&mut self, card: &str) -> Option<u64> {
        self.cards.remove(card).map(|entry| entry.session)
    }

    /// Keeps only the cards in `visible` (the loaded timeline window). The
    /// others are removed and their sessions returned for disposal.
    pub fn retain_visible(&mut self, visible: &BTreeSet<String>) -> PoolPlan {
        let gone: Vec<String> =
            self.cards.keys().filter(|card| !visible.contains(*card)).cloned().collect();
        let dispose = gone
            .into_iter()
            .filter_map(|card| self.cards.remove(&card).map(|entry| (card, entry.session)))
            .collect();
        PoolPlan { dispose, ..PoolPlan::default() }
    }

    /// Plans the suspend of every resumed card (window hidden or app locked).
    /// The cards stay held; resume them with [`CardPool::touch`].
    pub fn suspend_all(&mut self) -> PoolPlan {
        let mut suspend = Vec::new();
        for (card, entry) in &mut self.cards {
            if entry.resumed {
                entry.resumed = false;
                suspend.push((card.clone(), entry.session));
            }
        }
        PoolPlan { suspend, ..PoolPlan::default() }
    }

    fn tick(&mut self) -> u64 {
        self.clock = self.clock.saturating_add(1);
        self.clock
    }

    /// Brings the pool within its limits, never evicting `keep`.
    fn enforce(&mut self, keep: &str, plan: &mut PoolPlan) {
        while self.cards.values().filter(|entry| entry.resumed).count() > MAX_RESUMED {
            let Some(victim) = self.least_recent(keep, true) else {
                break;
            };
            if let Some(entry) = self.cards.get_mut(&victim) {
                entry.resumed = false;
                plan.suspend.push((victim, entry.session));
            }
        }
        while self.cards.len() > MAX_OPEN {
            let Some(victim) = self.least_recent(keep, false) else {
                break;
            };
            if let Some(entry) = self.cards.remove(&victim) {
                plan.dispose.push((victim, entry.session));
            }
        }
    }

    /// The least recently used card other than `keep`; with `resumed_only`,
    /// among the resumed ones.
    fn least_recent(&self, keep: &str, resumed_only: bool) -> Option<String> {
        self.cards
            .iter()
            .filter(|(card, entry)| card.as_str() != keep && (!resumed_only || entry.resumed))
            .min_by_key(|(_, entry)| entry.used)
            .map(|(card, _)| card.clone())
    }
}
