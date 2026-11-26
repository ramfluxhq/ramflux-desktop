// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The card runtime's native tree as the desktop holds it
//! (`ramflux.card.tree.v1` and `ramflux.card.mutations.v1`).
//!
//! The tree is a flat node map keyed by the runtime's canonical key; each
//! node names its children in order. A mutation batch applies atomically:
//! it must name the held session and the held tree version, it is applied to
//! a copy, the copy is validated, and only then does it replace the held
//! tree. A refused batch leaves the held tree as it was; the caller pauses
//! interaction and rebuilds the card from its open batch.
//!
//! Pure data: no GPUI types and no bus access (see `ramflux-desktop-sdk`).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Schema tag of a mutation batch.
pub const MUTATIONS_SCHEMA: &str = "ramflux.card.mutations.v1";
/// Largest batch the runtime sends, in bytes of its JSON.
pub const MAX_BATCH_BYTES: usize = 256 * 1024;
/// Largest number of operations in one batch.
pub const MAX_BATCH_OPS: usize = 512;
/// Largest number of nodes in one tree.
pub const MAX_NODES: usize = 256;

/// Why a batch or tree was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CardApplyError {
    /// The JSON is not a mutation batch of this schema.
    #[error("card batch is malformed")]
    Malformed,
    /// Over the byte or operation limit.
    #[error("card batch is over its limit")]
    Limit,
    /// The batch names another session or tree version than the held tree.
    /// The held tree is unchanged.
    #[error("card batch does not follow the held tree")]
    Mismatch,
    /// The batch would leave an invalid tree. The held tree is unchanged.
    #[error("card batch leaves an invalid tree")]
    Invalid,
}

/// Runtime-owned lifecycle of a card (spec 3.4), as the tree reports it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardLifecycle {
    /// The author may still change the card.
    #[default]
    Open,
    /// The author closed the card.
    Closed,
    /// The card was retired.
    Retired,
    /// A newer card replaced this one.
    Superseded,
}

/// What the held tree shows the user, derived from the tree itself.
/// Interaction states (submitting, consumed, out of sync, over limit,
/// unsupported) are overlaid by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardPhase {
    /// Interactive.
    Ready,
    /// The author is still writing the card.
    Generating,
    /// Closed by the author.
    Closed,
    /// Retired.
    Retired,
    /// Replaced by a newer card.
    Superseded,
}

/// The token that fences one host event or slot reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardToken {
    /// The session the batch named.
    pub session: u64,
    /// The signed version the tree shows.
    pub signed_version: u64,
    /// The edit sequence of the tree.
    pub edit_seq: u64,
    /// The tree version the token was issued at.
    pub tree_version: u64,
    /// The node the event addresses.
    pub key: String,
}

/// One node of the tree, as the runtime serializes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardNode {
    /// Canonical key of the node.
    pub key: String,
    /// Component id in the card.
    pub component_id: String,
    /// Source instance path of the resolver.
    pub source_instance: Vec<String>,
    /// Resolver scope.
    pub scope: String,
    /// Parent key; `None` for the root.
    pub parent_key: Option<String>,
    /// Resolved slot, as the runtime reports it.
    pub slot: Value,
    /// Child keys, in order.
    pub children: Vec<String>,
    /// Component kind (one of the 20 catalog kinds; the renderer decides
    /// what an unknown kind shows).
    pub kind: String,
    /// Kind-specific attributes. Untrusted display data.
    pub attrs: Map<String, Value>,
}

/// A host-owned slot the runtime asks the host to fill (approval, link,
/// modal, image). Its fields beyond `id`, `key` and `token` depend on `kind`
/// and are kept as the runtime sent them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardHostSlot {
    /// Slot id, `[kind,key]`.
    pub id: String,
    /// Key of the node that owns the slot.
    pub key: String,
    /// Token that fences the reply.
    pub token: CardToken,
    /// The request: `kind` and its fields.
    #[serde(flatten)]
    pub request: Map<String, Value>,
}

/// One operation of a batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum CardMutation {
    /// Remove a node (descendants first in the batch).
    Remove {
        /// The node to remove.
        key: String,
    },
    /// Insert or replace a node under `parent` at `index` (parents first).
    /// A `parent` of `None` makes it the root.
    Upsert {
        /// Parent key; `None` for the root.
        parent: Option<String>,
        /// Position among the parent's children.
        index: usize,
        /// The node as it now reads (boxed: it is much larger than `Remove`).
        node: Box<CardNode>,
    },
}

/// A `ramflux.card.mutations.v1` batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardBatch {
    /// Schema tag; must be [`MUTATIONS_SCHEMA`].
    pub schema: String,
    /// Session the batch belongs to.
    pub session: u64,
    /// Tree version the batch starts from.
    pub previous_version: u64,
    /// Tree version after the batch.
    pub version: u64,
    /// Signed version the tree shows after the batch.
    pub signed_version: u64,
    /// Signed hash the tree shows after the batch.
    pub signed_hash: String,
    /// Edit sequence after the batch.
    pub edit_seq: u64,
    /// The operations, in order.
    pub mutations: Vec<CardMutation>,
    /// The complete replacement host-slot list.
    pub host_slots: Vec<CardHostSlot>,
    /// String-catalog tag after fallback.
    #[serde(default = "english")]
    pub locale: String,
    /// CLDR locale for numbers, dates and plurals.
    #[serde(default = "english")]
    pub format_locale: String,
    /// `rtl` or `ltr`.
    #[serde(default = "ltr")]
    pub direction: String,
    /// The author still has an open text stream on this card.
    #[serde(default)]
    pub generating: bool,
    /// Lifecycle of the card.
    #[serde(default)]
    pub lifecycle: CardLifecycle,
    /// The card that replaced this one, when verified.
    #[serde(default)]
    pub replacement_ref: Option<String>,
}

fn english() -> String {
    "en".to_owned()
}

fn ltr() -> String {
    "ltr".to_owned()
}

/// Parses one batch from its bytes, refusing oversize input first.
pub fn parse_batch(bytes: &[u8]) -> Result<CardBatch, CardApplyError> {
    if bytes.len() > MAX_BATCH_BYTES {
        return Err(CardApplyError::Limit);
    }
    let batch: CardBatch = serde_json::from_slice(bytes).map_err(|_| CardApplyError::Malformed)?;
    if batch.schema != MUTATIONS_SCHEMA {
        return Err(CardApplyError::Malformed);
    }
    Ok(batch)
}

/// The held tree of one card session.
#[derive(Debug, Clone, PartialEq)]
pub struct CardTree {
    session: u64,
    signed_version: u64,
    signed_hash: String,
    edit_seq: u64,
    tree_version: u64,
    root: Option<String>,
    nodes: BTreeMap<String, CardNode>,
    host_slots: Vec<CardHostSlot>,
    locale: String,
    format_locale: String,
    direction: String,
    generating: bool,
    lifecycle: CardLifecycle,
    replacement_ref: Option<String>,
}

impl CardTree {
    /// An empty tree for `session`, at tree version zero. The first batch of
    /// an open starts from here.
    #[must_use]
    pub fn empty(session: u64) -> Self {
        Self {
            session,
            signed_version: 0,
            signed_hash: String::new(),
            edit_seq: 0,
            tree_version: 0,
            root: None,
            nodes: BTreeMap::new(),
            host_slots: Vec::new(),
            locale: english(),
            format_locale: english(),
            direction: ltr(),
            generating: false,
            lifecycle: CardLifecycle::Open,
            replacement_ref: None,
        }
    }

    /// The session the tree belongs to (the batch session, not a handle).
    #[must_use]
    pub const fn session(&self) -> u64 {
        self.session
    }

    /// The tree version the next batch must start from.
    #[must_use]
    pub const fn tree_version(&self) -> u64 {
        self.tree_version
    }

    /// The signed version shown.
    #[must_use]
    pub const fn signed_version(&self) -> u64 {
        self.signed_version
    }

    /// The signed hash shown.
    #[must_use]
    pub fn signed_hash(&self) -> &str {
        &self.signed_hash
    }

    /// The root node's key, once a batch has set it.
    #[must_use]
    pub fn root(&self) -> Option<&str> {
        self.root.as_deref()
    }

    /// One node by key.
    #[must_use]
    pub fn node(&self, key: &str) -> Option<&CardNode> {
        self.nodes.get(key)
    }

    /// The node count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether no node is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The host slots of the held version.
    #[must_use]
    pub fn host_slots(&self) -> &[CardHostSlot] {
        &self.host_slots
    }

    /// The tag of the string catalog in use.
    #[must_use]
    pub fn locale(&self) -> &str {
        &self.locale
    }

    /// The CLDR locale used for numbers, dates and plurals.
    #[must_use]
    pub fn format_locale(&self) -> &str {
        &self.format_locale
    }

    /// `rtl` or `ltr`; lay the tree out in this direction.
    #[must_use]
    pub fn direction(&self) -> &str {
        &self.direction
    }

    /// The lifecycle the tree reports.
    #[must_use]
    pub const fn lifecycle(&self) -> CardLifecycle {
        self.lifecycle
    }

    /// The replacing card, when the tree is superseded.
    #[must_use]
    pub fn replacement_ref(&self) -> Option<&str> {
        self.replacement_ref.as_deref()
    }

    /// What the tree shows the user, from the tree alone.
    #[must_use]
    pub const fn phase(&self) -> CardPhase {
        match self.lifecycle {
            CardLifecycle::Closed => CardPhase::Closed,
            CardLifecycle::Retired => CardPhase::Retired,
            CardLifecycle::Superseded => CardPhase::Superseded,
            CardLifecycle::Open if self.generating => CardPhase::Generating,
            CardLifecycle::Open => CardPhase::Ready,
        }
    }

    /// The token for a host event on `key`, from the held version. `None`
    /// when the tree holds no such node.
    #[must_use]
    pub fn token_for(&self, key: &str) -> Option<CardToken> {
        self.nodes.contains_key(key).then(|| CardToken {
            session: self.session,
            signed_version: self.signed_version,
            edit_seq: self.edit_seq,
            tree_version: self.tree_version,
            key: key.to_owned(),
        })
    }

    /// Applies `batch` atomically. Refused batches leave the tree unchanged.
    pub fn apply(&mut self, batch: &CardBatch) -> Result<(), CardApplyError> {
        if batch.schema != MUTATIONS_SCHEMA {
            return Err(CardApplyError::Malformed);
        }
        if batch.mutations.len() > MAX_BATCH_OPS {
            return Err(CardApplyError::Limit);
        }
        if batch.session != self.session
            || batch.previous_version != self.tree_version
            || batch.version <= batch.previous_version
        {
            return Err(CardApplyError::Mismatch);
        }
        let mut next = self.clone();
        for mutation in &batch.mutations {
            next.apply_one(mutation)?;
        }
        next.signed_version = batch.signed_version;
        next.signed_hash.clone_from(&batch.signed_hash);
        next.edit_seq = batch.edit_seq;
        next.tree_version = batch.version;
        next.host_slots.clone_from(&batch.host_slots);
        next.locale.clone_from(&batch.locale);
        next.format_locale.clone_from(&batch.format_locale);
        next.direction.clone_from(&batch.direction);
        next.generating = batch.generating;
        next.lifecycle = batch.lifecycle;
        next.replacement_ref.clone_from(&batch.replacement_ref);
        next.validate()?;
        *self = next;
        Ok(())
    }

    fn apply_one(&mut self, mutation: &CardMutation) -> Result<(), CardApplyError> {
        match mutation {
            CardMutation::Remove { key } => {
                let removed = self.nodes.remove(key).ok_or(CardApplyError::Invalid)?;
                self.detach(key, removed.parent_key.as_deref());
                if self.root.as_deref() == Some(key.as_str()) {
                    self.root = None;
                }
                Ok(())
            }
            CardMutation::Upsert { parent, index, node } => {
                if node.parent_key != *parent {
                    return Err(CardApplyError::Invalid);
                }
                if let Some(previous) = self.nodes.get(&node.key).map(|n| n.parent_key.clone()) {
                    self.detach(&node.key, previous.as_deref());
                }
                match parent {
                    Some(parent) => {
                        // A parent's own payload may already list a child that
                        // arrives later in the batch: place it by index, once.
                        let holder = self.nodes.get_mut(parent).ok_or(CardApplyError::Invalid)?;
                        holder.children.retain(|child| *child != node.key);
                        if *index > holder.children.len() {
                            return Err(CardApplyError::Invalid);
                        }
                        holder.children.insert(*index, node.key.clone());
                    }
                    None => self.root = Some(node.key.clone()),
                }
                self.nodes.insert(node.key.clone(), node.as_ref().clone());
                Ok(())
            }
        }
    }

    fn detach(&mut self, key: &str, parent: Option<&str>) {
        if let Some(parent) = parent
            && let Some(holder) = self.nodes.get_mut(parent)
        {
            holder.children.retain(|child| child != key);
        }
    }

    /// Checks that the nodes form one tree under the root, within limits.
    fn validate(&self) -> Result<(), CardApplyError> {
        if self.nodes.len() > MAX_NODES {
            return Err(CardApplyError::Limit);
        }
        let root = self.root.as_deref().ok_or(CardApplyError::Invalid)?;
        let root_node = self.nodes.get(root).ok_or(CardApplyError::Invalid)?;
        if root_node.parent_key.is_some() {
            return Err(CardApplyError::Invalid);
        }
        let mut seen = BTreeSet::new();
        let mut stack = vec![root];
        while let Some(key) = stack.pop() {
            if !seen.insert(key) {
                return Err(CardApplyError::Invalid);
            }
            let node = self.nodes.get(key).ok_or(CardApplyError::Invalid)?;
            for child in &node.children {
                let child_node = self.nodes.get(child).ok_or(CardApplyError::Invalid)?;
                if child_node.parent_key.as_deref() != Some(key) {
                    return Err(CardApplyError::Invalid);
                }
                stack.push(child.as_str());
            }
        }
        if seen.len() != self.nodes.len() {
            return Err(CardApplyError::Invalid);
        }
        Ok(())
    }
}
