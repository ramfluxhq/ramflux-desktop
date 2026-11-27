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
//! The store applies the runtime's facts (`CardEvent`) to a [`CardRecord`]
//! per surface and to the pool. What the pool decides that needs a bus call
//! (a suspend or a dispose, or a re-read or submit the runtime asked for) is
//! queued as a [`CardDue`]; the session takes the queue after applying events
//! and turns each item into an intent.
//!
//! The pool only decides. It returns the suspend, resume and dispose steps
//! the caller must perform on the bus, and the caller reports the session a
//! resume created with [`CardPool::adopt`] (a resume issues a fresh session
//! id). Recency is a logical clock, never wall time.

use std::collections::{BTreeMap, BTreeSet};

use super::Store;
use super::changes::Changes;
use crate::card_event::{CardEvent, CardLink, CardReceipt, CardSlotOutcome, CardStatus};
use crate::ids::SurfaceId;
use crate::model::{CardBatch, CardSeen, CardStamp, CardTree};

/// Resumed (live) sessions per account.
pub const MAX_RESUMED: usize = 2;
/// Held sessions per account.
pub const MAX_OPEN: usize = 24;
/// Stamps kept in one card's rebase chain (the oldest are dropped first).
pub const MAX_CHAIN: usize = 64;

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

    /// Bumps the recency of a held card the user acted on, without changing
    /// its resume state (the pool plan for that action is already decided).
    pub fn bump(&mut self, card: &str) {
        let used = self.tick();
        if let Some(entry) = self.cards.get_mut(card) {
            entry.used = used;
        }
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

    /// Records that a resume of `card` completed: the card is resumed under
    /// the session the resume created (fresh token identity).
    pub fn adopt(&mut self, card: &str, session: u64) {
        if let Some(entry) = self.cards.get_mut(card) {
            entry.session = session;
            entry.resumed = true;
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

    /// The `limit` most recently used held cards, most recent first.
    #[must_use]
    pub fn recent(&self, limit: usize) -> Vec<String> {
        let mut cards: Vec<(&String, &CardEntry)> = self.cards.iter().collect();
        cards.sort_by_key(|(_, entry)| std::cmp::Reverse(entry.used));
        cards.into_iter().take(limit).map(|(card, _)| card.clone()).collect()
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

/// What the store still needs the desktop to do about a card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardDue {
    /// Read the card again (`card.session.update`).
    Refresh(SurfaceId),
    /// Send the submit the runtime's button press asked for.
    Submit(SurfaceId),
    /// Suspend the card's session (pool bound).
    Suspend(SurfaceId),
    /// Dispose the card's session (pool bound).
    Dispose(SurfaceId),
}

impl CardDue {
    /// The surface the step is about.
    pub fn surface(&self) -> &SurfaceId {
        match self {
            Self::Refresh(s) | Self::Submit(s) | Self::Suspend(s) | Self::Dispose(s) => s,
        }
    }
}

/// The runtime's card for one surface, as the desktop holds it.
#[derive(Clone, PartialEq)]
pub struct CardRecord {
    /// The session the held tree belongs to (fresh after a resume).
    pub session: u64,
    /// The held tree; `None` until the open batch applied.
    pub tree: Option<CardTree>,
    /// The overlay status.
    pub status: CardStatus,
    /// The link sheet the runtime asked for, while it is shown.
    pub link: Option<CardLink>,
    /// The last refusal (its reason key), until the next accepted call.
    pub refusal: Option<String>,
    /// The runtime discarded a local draft in favour of the signed update.
    pub draft_discarded: bool,
    /// The receipt of the last accepted submit.
    pub receipt: Option<CardReceipt>,
    /// The last submit did not reach the daemon or its answer is unknown:
    /// the button stays consumed and the user may retry the submit.
    pub submit_failed: bool,
    /// The stamps this desktop produced by absorbing its own replies since
    /// the last outside change (open, resume, signed update, present,
    /// re-read), oldest first, the last one being the held version. A text
    /// draft sent on an older stamp of this chain may be rebased onto the
    /// held version; nothing else may.
    pub chain: Vec<CardStamp>,
}

impl std::fmt::Debug for CardRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CardRecord")
            .field("session", &self.session)
            .field("tree", &self.tree.as_ref().map(CardTree::tree_version))
            .field("status", &self.status)
            .field("link", &self.link)
            .field("refusal", &self.refusal)
            .field("submit_failed", &self.submit_failed)
            .finish_non_exhaustive()
    }
}

impl CardRecord {
    fn new(session: u64, status: CardStatus) -> Self {
        Self {
            session,
            tree: None,
            status,
            link: None,
            refusal: None,
            draft_discarded: false,
            receipt: None,
            submit_failed: false,
            chain: Vec::new(),
        }
    }

    /// Whether an event taken on `seen` for `key` may be re-sent on the held
    /// version. The stamp must be in the chain (only this desktop's own text
    /// drafts changed the tree since), and the node must keep its kind and
    /// input target. A text draft (`input`) needs nothing more: its whole
    /// value replaces the draft. Any other event (a tap) also needs the
    /// signed version and the node's drawn content (its attributes, label
    /// and action) to be exactly what the user saw.
    pub fn rebasable(&self, key: &str, seen: &CardSeen, input: bool) -> bool {
        let Some(tree) = self.tree.as_ref() else { return false };
        let Some(node) = tree.node(key) else { return false };
        let same_node = self.chain.contains(&seen.stamp)
            && node.kind == seen.kind
            && node.attrs.get("input_pointer").and_then(serde_json::Value::as_str)
                == seen.input_pointer.as_deref();
        if !same_node {
            return false;
        }
        input
            || (seen.stamp.signed_version == tree.signed_version()
                && seen.content.is_some()
                && seen.content == tree.content(key))
    }

    fn reset_chain(&mut self) {
        self.chain.clear();
        if let Some(tree) = &self.tree {
            self.chain.push(tree.stamp());
        }
    }

    fn extend_chain(&mut self) {
        if let Some(tree) = &self.tree {
            self.chain.push(tree.stamp());
            let excess = self.chain.len().saturating_sub(MAX_CHAIN);
            self.chain.drain(..excess);
        }
    }

    /// Whether the held session is resumed (live) in the pool.
    pub fn resumed(&self, pool: &CardPool, surface: &SurfaceId) -> bool {
        pool.entry(surface.as_str()).is_some_and(|e| e.resumed)
    }
}

/// Applies the open or resume batch to a fresh tree of `session`.
fn fresh_tree(session: u64, batch: &CardBatch) -> Option<CardTree> {
    let mut tree = CardTree::empty(session);
    tree.apply(batch).ok().map(|()| tree)
}

impl Store {
    /// The card of `surface`, when the runtime holds one for it.
    pub fn card(&self, surface: &SurfaceId) -> Option<&CardRecord> {
        self.card_records.get(surface)
    }

    /// The pool of the card sessions this account holds.
    pub fn card_pool(&self) -> &CardPool {
        &self.cards
    }

    /// Takes what the store asks the desktop to do about cards.
    pub fn take_card_due(&mut self) -> Vec<CardDue> {
        std::mem::take(&mut self.card_due)
    }

    /// The `seen` of an event rebased onto the held version, when the event
    /// may be re-sent on it (see [`CardRecord::rebasable`]); `None` otherwise.
    pub fn card_rebased(
        &self,
        surface: &SurfaceId,
        key: &str,
        seen: &CardSeen,
        input: bool,
    ) -> Option<CardSeen> {
        let record = self.card_records.get(surface)?;
        if !record.rebasable(key, seen, input) {
            return None;
        }
        let mut next = seen.clone();
        next.stamp = record.tree.as_ref()?.stamp();
        Some(next)
    }

    /// Records a refusal the desktop decided itself (an event taken on a
    /// version the card no longer holds, or one that was not sent). The card
    /// keeps its status: a consumed button stays consumed.
    pub fn card_refuse(&mut self, surface: &SurfaceId, reason: &str) {
        if let Some(record) = self.card_records.get_mut(surface) {
            record.refusal = Some(reason.to_owned());
        }
    }

    /// Bumps the pool recency of a card the user acted on.
    pub fn card_bump(&mut self, surface: &SurfaceId) {
        self.cards.bump(surface.as_str());
    }

    /// Ends the rebase chain at the held version: the tree changed by
    /// something other than this desktop's own text draft (its own tap), so
    /// no event taken before may follow it.
    pub fn card_reset_chain(&mut self, surface: &SurfaceId) {
        if let Some(record) = self.card_records.get_mut(surface) {
            record.reset_chain();
        }
    }

    /// Records that the user retried a failed submit (it is in flight again).
    pub fn card_submit_sent(&mut self, surface: &SurfaceId) {
        if let Some(record) = self.card_records.get_mut(surface) {
            record.submit_failed = false;
        }
    }

    /// Forgets every card session: the connection to the daemon was lost (or
    /// the account closed), and the runtime may no longer hold them. Cards
    /// on screen are opened again once the connection is back.
    pub(super) fn cards_reset(&mut self, changes: &mut Changes) {
        for surface in self.card_records.keys() {
            changes.cards.insert(surface.clone());
            if let Some(moment) = self.surface_ids.get(surface) {
                changes.moments.insert(moment.clone());
            }
        }
        self.card_records.clear();
        self.cards = CardPool::new();
        self.card_due.clear();
    }

    /// Queues the pool's bus steps (suspends and disposes) for the session.
    fn queue_pool_plan(&mut self, plan: &PoolPlan) {
        for (card, _) in &plan.suspend {
            self.card_due.push(CardDue::Suspend(SurfaceId::new(card)));
        }
        for (card, _) in &plan.dispose {
            self.card_due.push(CardDue::Dispose(SurfaceId::new(card)));
        }
    }

    /// Applies one card runtime fact. A fact about a card the store no
    /// longer holds (it was disposed, or its surface removed) is dropped.
    pub(super) fn card_event(&mut self, event: &CardEvent, changes: &mut Changes) {
        let surface = event.surface_id().clone();
        changes.cards.insert(surface.clone());
        if let Some(moment) = self.surface_moment(&surface).cloned() {
            changes.moments.insert(moment);
        }
        match event {
            CardEvent::Opened { session, batch, .. } => {
                let plan = self.cards.opened(surface.as_str(), *session);
                self.queue_pool_plan(&plan);
                let mut record = CardRecord::new(*session, CardStatus::Live);
                match fresh_tree(*session, batch) {
                    Some(tree) => {
                        record.tree = Some(tree);
                        record.reset_chain();
                    }
                    None => record.status = CardStatus::Stale,
                }
                self.card_records.insert(surface, record);
            }
            CardEvent::Resumed { session, batch, .. } => {
                self.cards.adopt(surface.as_str(), *session);
                let Some(record) = self.card_records.get_mut(&surface) else { return };
                record.session = *session;
                // A link sheet belongs to the session that asked for it.
                record.link = None;
                match fresh_tree(*session, batch) {
                    Some(tree) => {
                        record.tree = Some(tree);
                        record.reset_chain();
                        if record.status == CardStatus::Stale {
                            record.status = CardStatus::Live;
                        }
                    }
                    None => record.status = CardStatus::Stale,
                }
            }
            CardEvent::Batch { batch, draft_conflict, own_reply, .. } => {
                let Some(record) = self.card_records.get_mut(&surface) else { return };
                record.draft_discarded = *draft_conflict;
                record.refusal = None;
                let applied = record.tree.as_mut().map(|tree| tree.apply(batch));
                match applied {
                    Some(Ok(())) => {
                        record.status = CardStatus::Live;
                        if *own_reply {
                            record.extend_chain();
                        } else {
                            record.reset_chain();
                        }
                    }
                    _ => {
                        // A batch that does not follow the held tree: the
                        // card is read again, and nothing is interactive
                        // until then.
                        record.status = CardStatus::Stale;
                        record.chain.clear();
                        self.card_due.push(CardDue::Refresh(surface));
                    }
                }
            }
            CardEvent::Stale { .. } => {
                if let Some(record) = self.card_records.get_mut(&surface) {
                    record.status = CardStatus::Stale;
                    record.chain.clear();
                    self.card_due.push(CardDue::Refresh(surface));
                }
            }
            CardEvent::Fallback { .. } => {
                if let Some(record) = self.card_records.get_mut(&surface) {
                    record.status = CardStatus::Fallback;
                }
            }
            CardEvent::PendingSubmit { .. } => {
                if let Some(record) = self.card_records.get_mut(&surface) {
                    record.status = CardStatus::PendingSubmit;
                    record.submit_failed = false;
                    self.card_due.push(CardDue::Submit(surface));
                }
            }
            CardEvent::EventSettled { .. } => {}
            CardEvent::SubmitFailed { .. } => {
                if let Some(record) = self.card_records.get_mut(&surface)
                    && record.status == CardStatus::PendingSubmit
                {
                    record.submit_failed = true;
                }
            }
            CardEvent::Submitted { receipt, .. } => {
                if let Some(record) = self.card_records.get_mut(&surface) {
                    record.status = CardStatus::Consumed;
                    record.receipt = Some(receipt.clone());
                    record.refusal = None;
                    record.submit_failed = false;
                }
            }
            CardEvent::Suspended { .. } => {
                self.cards.suspended(surface.as_str());
                if let Some(record) = self.card_records.get_mut(&surface) {
                    // A link sheet belongs to the session that asked for it.
                    record.link = None;
                }
            }
            CardEvent::Disposed { .. } => {
                self.cards.remove(surface.as_str());
                self.card_records.remove(&surface);
            }
            CardEvent::OpenUrl { link, .. } => {
                if let Some(record) = self.card_records.get_mut(&surface) {
                    record.link = Some(link.clone());
                }
            }
            CardEvent::LinkOpened { .. } | CardEvent::LinkClosed { .. } => {
                if let Some(record) = self.card_records.get_mut(&surface) {
                    record.link = None;
                }
            }
            CardEvent::ApprovalVerdict { outcome, .. } => {
                if let Some(record) = self.card_records.get_mut(&surface)
                    && matches!(outcome, CardSlotOutcome::Confirm | CardSlotOutcome::Deny)
                {
                    record.status = CardStatus::AwaitingConfirmation;
                }
            }
            CardEvent::Refused { reason, .. } => {
                if let Some(record) = self.card_records.get_mut(&surface) {
                    record.refusal = Some(reason.clone());
                    if record.status == CardStatus::PendingSubmit {
                        record.status = CardStatus::Live;
                        record.submit_failed = false;
                    }
                }
            }
        }
    }
}
