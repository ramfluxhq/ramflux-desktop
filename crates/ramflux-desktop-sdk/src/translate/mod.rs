// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Daemon answers and pushed events to domain events.
//!
//! The translator owns what the source already told the store (known nodes,
//! contexts, messages, grants), so it emits only differences: a re-snapshot
//! after a reconnect upserts what changed and removes what disappeared, and
//! a poll that returns the same messages emits nothing. It never invents
//! data the daemon did not report; where the daemon lacks a field the
//! desktop contract has, the field keeps its neutral default (see the gap
//! ledger).
//!
//! - `daemon`: daemon status, nodes, accounts.
//! - `account`: account status, devices, integrations.
//! - `contacts`: participants and contact links.
//! - `conversation`: conversations, groups and their messages.
//! - `grants`: approvals, grants and standing approvals.
//! - `grant_failures`: fail-closed grant errors.
//! - `effects`: answers to effects, and the audit record of confirmed ones.

mod account;
mod contacts;
mod conversation;
mod daemon;
mod effects;
mod grant_failures;
mod grants;

use std::collections::{BTreeMap, BTreeSet};

use ramflux_desktop_core::{
    Account, AccountId, BodyCache, Context, ContextId, ContextKind, DeviceId, DomainEvent, GrantId,
    GrantState, Listing, MomentId, NodeId, ParticipantId, StandingApproval, StandingApprovalId,
    Timestamp,
};
use serde_json::Value;

use crate::plan::Known;
use crate::plan::fetch::Fetch;

/// The source's scope.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// Daemon status, nodes and the account list (the entry page).
    Daemon,
    /// One unlocked account.
    Account(AccountId),
}

impl Scope {
    /// The account, for an account scope.
    pub fn account(&self) -> Option<&AccountId> {
        match self {
            Self::Daemon => None,
            Self::Account(id) => Some(id),
        }
    }
}

/// Events plus the reads they call for.
#[derive(Debug, Default, PartialEq)]
pub struct Translated {
    /// Domain events, in order.
    pub events: Vec<DomainEvent>,
    /// Reads to run next.
    pub follow: Vec<Fetch>,
}

impl Translated {
    fn events(events: Vec<DomainEvent>) -> Self {
        Self { events, follow: Vec::new() }
    }
}

/// Decrypted bodies kept for messages whose stored copy is ciphertext (a
/// group's), by daemon message id. Bounded like the store's body cache.
pub const PLAINTEXT_CAPACITY: usize = 16_384;

/// What the source told the store about one stored message: a digest of its
/// payload (to emit only differences) and whether it was shown as a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Reported {
    digest: u64,
    message: bool,
}

/// A stable digest of a payload, for change detection only.
pub(crate) fn digest(kind: &ramflux_desktop_core::MomentKind) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::hash::DefaultHasher::new();
    format!("{kind:?}").hash(&mut hasher);
    hasher.finish()
}

/// What the source already reported for one scope.
#[derive(Debug)]
pub struct Translator {
    scope: Scope,
    socket: String,
    nodes: BTreeMap<NodeId, ramflux_desktop_core::Node>,
    accounts: BTreeMap<AccountId, Account>,
    labels: BTreeMap<AccountId, String>,
    principal: Option<String>,
    self_device: Option<String>,
    devices: BTreeMap<DeviceId, ramflux_desktop_core::Device>,
    participants: BTreeMap<ParticipantId, ramflux_desktop_core::ContactLink>,
    links: BTreeMap<ParticipantId, crate::plan::KnownLink>,
    contexts: BTreeMap<ContextId, Context>,
    pins: Vec<ContextId>,
    pins_recorded: bool,
    pins_changed: bool,
    moments: BTreeMap<MomentId, Reported>,
    resend: BTreeSet<MomentId>,
    plaintext: BodyCache,
    grants: BTreeMap<GrantId, GrantState>,
    bindings: BTreeMap<GrantId, grant_failures::Binding>,
    daemon_grants: BTreeMap<String, String>,
    standing: BTreeMap<StandingApprovalId, StandingApproval>,
    integrations: Option<ramflux_desktop_core::Integrations>,
    focus: Option<ContextId>,
    listed: BTreeSet<Listing>,
    answered: BTreeSet<Fetch>,
}

impl Translator {
    /// A translator for `scope`, attached to `socket` (shown as daemon info).
    pub fn new(scope: Scope, socket: impl Into<String>) -> Self {
        Self {
            scope,
            socket: socket.into(),
            nodes: BTreeMap::new(),
            accounts: BTreeMap::new(),
            labels: BTreeMap::new(),
            principal: None,
            self_device: None,
            devices: BTreeMap::new(),
            participants: BTreeMap::new(),
            links: BTreeMap::new(),
            contexts: BTreeMap::new(),
            pins: Vec::new(),
            pins_recorded: false,
            pins_changed: false,
            moments: BTreeMap::new(),
            resend: BTreeSet::new(),
            plaintext: BodyCache::with_capacity(PLAINTEXT_CAPACITY),
            grants: BTreeMap::new(),
            bindings: BTreeMap::new(),
            daemon_grants: BTreeMap::new(),
            standing: BTreeMap::new(),
            integrations: None,
            focus: None,
            listed: BTreeSet::new(),
            answered: BTreeSet::new(),
        }
    }

    /// The scope.
    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    /// Records the context the user looks at.
    pub fn set_focus(&mut self, context: Option<ContextId>) {
        self.focus = context;
    }

    /// Seeds the pins recorded in the desktop settings (pin order). Once
    /// the desktop has a record, it wins over pins the daemon reports.
    pub fn set_pins(&mut self, pins: Vec<ContextId>) {
        self.pins = pins;
        self.pins_recorded = true;
        self.pins_changed = false;
    }

    /// The pinned contexts in pin order.
    pub fn pins(&self) -> &[ContextId] {
        &self.pins
    }

    /// Whether the pins changed since the last call (they need saving).
    pub fn take_pins_changed(&mut self) -> bool {
        std::mem::take(&mut self.pins_changed)
    }

    /// Contexts to keep fresh: the focused one and the pinned ones.
    pub fn watched(&self) -> Vec<(ContextId, ContextKind)> {
        let mut ids: BTreeSet<&ContextId> = self.pins.iter().collect();
        ids.extend(self.focus.iter());
        ids.into_iter()
            .filter_map(|id| self.contexts.get(id).map(|c| (id.clone(), c.kind)))
            .collect()
    }

    /// The reads that keep the watched contexts fresh.
    pub fn poll_reads(&self) -> Vec<Vec<Fetch>> {
        self.watched()
            .into_iter()
            .map(|(id, kind)| match kind {
                ContextKind::Group => vec![Fetch::GroupReceive(id)],
                _ => vec![Fetch::Receive(id.clone()), Fetch::Messages(id)],
            })
            .collect()
    }

    /// Translates one read.
    pub fn fetched(&mut self, fetch: &Fetch, value: &Value, now: Timestamp) -> Translated {
        let mut out = self.translate(fetch, value, now);
        out.events.extend(self.listing_answered(fetch));
        out
    }

    /// The listings a read completes, the first time it is answered.
    fn listing_answered(&mut self, fetch: &Fetch) -> Option<DomainEvent> {
        if matches!(fetch, Fetch::ConversationList | Fetch::GroupList) {
            self.answered.insert(fetch.clone());
        }
        let listing = match fetch {
            Fetch::NodeList => Listing::Nodes,
            Fetch::AccountList => Listing::Accounts,
            // Contexts are conversations and groups: listed once both answered.
            Fetch::ConversationList | Fetch::GroupList
                if self.answered.contains(&Fetch::ConversationList)
                    && self.answered.contains(&Fetch::GroupList) =>
            {
                Listing::Contexts
            }
            Fetch::ContactList => Listing::Contacts,
            Fetch::Approvals => Listing::Approvals,
            _ => return None,
        };
        self.listed(&[listing])
    }

    /// `ListingsLoaded` for those of `listings` not reported yet.
    fn listed(&mut self, listings: &[Listing]) -> Option<DomainEvent> {
        let fresh: Vec<Listing> =
            listings.iter().copied().filter(|l| self.listed.insert(*l)).collect();
        (!fresh.is_empty()).then_some(DomainEvent::ListingsLoaded(fresh))
    }

    /// The account's snapshot is complete. The daemon lists no objects (gap
    /// ledger), so the objects collection is known to be empty from here.
    pub fn snapshot_done(&mut self) -> Vec<DomainEvent> {
        if self.scope.account().is_none() {
            return Vec::new();
        }
        self.listed(&[Listing::Objects]).into_iter().collect()
    }

    fn translate(&mut self, fetch: &Fetch, value: &Value, now: Timestamp) -> Translated {
        match fetch {
            Fetch::DaemonStatus => Translated::events(self.daemon_status(value)),
            Fetch::NodeList => Translated::events(self.node_list(value)),
            Fetch::AccountList => self.account_list(value),
            Fetch::AccountStatus(id) => Translated::events(self.account_status(id, value)),
            Fetch::DeviceList => Translated::events(self.device_list(value)),
            Fetch::ConversationList => self.conversation_list(value),
            Fetch::GroupList => self.group_list(value),
            Fetch::ContactList => Translated::events(self.contact_list(value)),
            Fetch::Approvals => Translated::events(self.approval_list(value)),
            Fetch::Grants => Translated::events(self.grant_list(value)),
            Fetch::StandingApprovals => Translated::events(self.standing_list(value)),
            Fetch::McpTools => Translated::events(self.mcp_tools(value)),
            Fetch::Messages(context) => Translated::events(self.messages(context, value, now)),
            Fetch::GroupReceive(context) => {
                self.remember_plaintext(value);
                Translated::events(self.messages(context, value, now))
            }
            Fetch::Receive(_) => {
                self.remember_plaintext(value);
                Translated::default()
            }
        }
    }

    /// This scope's account was found locked: report it and forget what was
    /// shown (plaintext stays only in the daemon).
    pub fn own_account_locked(&mut self) -> Vec<DomainEvent> {
        let Some(id) = self.scope.account().cloned() else { return Vec::new() };
        self.plaintext.clear();
        self.set_lock(&id, ramflux_desktop_core::LockState::Locked)
    }

    /// The view asked for these messages' bodies again (its store evicted
    /// them): they are re-emitted by the next read of their contexts, which
    /// this returns.
    pub fn request_bodies(&mut self, moments: &[MomentId]) -> Vec<Fetch> {
        let mut contexts = BTreeSet::new();
        for id in moments {
            let Some(context) = self
                .contexts
                .keys()
                .find(|c| id.as_str().starts_with(conversation::message_moment_id(c, "").as_str()))
            else {
                continue;
            };
            if self.moments.contains_key(id) {
                self.resend.insert(id.clone());
                contexts.insert(context.clone());
            }
        }
        contexts
            .into_iter()
            .map(|id| match self.contexts.get(&id).map(|c| c.kind) {
                Some(ContextKind::Group) => Fetch::GroupReceive(id),
                _ => Fetch::Messages(id),
            })
            .collect()
    }

    /// Translates one pushed event.
    pub fn pushed(&mut self, topic: &str, body: &Value) -> Vec<DomainEvent> {
        match topic {
            "mcp.approval.request" => {
                let id = GrantId::new(text(body, "approval_id"));
                if self.grants.contains_key(&id) {
                    self.approval_changed(&id, body)
                } else {
                    self.approval(body).into_iter().collect()
                }
            }
            "mcp.approval.granted" => self.approval_decided(body, GrantState::Approved),
            "mcp.approval.denied" => self.approval_decided(body, GrantState::Denied),
            // Tool starts and A2UI action records are not bound to a
            // conversation yet; they have no moment to land in (gap ledger).
            _ => Vec::new(),
        }
    }
}

impl Known for Translator {
    fn context_kind(&self, context: &ContextId) -> Option<ContextKind> {
        self.contexts.get(context).map(|c| c.kind)
    }

    fn self_device(&self) -> Option<&str> {
        self.self_device.as_deref()
    }

    fn daemon_grant_id(&self, approval: &str) -> Option<&str> {
        self.daemon_grants.get(approval).map(String::as_str)
    }

    fn self_principal(&self) -> Option<&str> {
        self.principal.as_deref()
    }

    fn contact_link(&self, participant: &str) -> Option<&crate::plan::KnownLink> {
        self.known_link(participant)
    }
}

/// Unix seconds to contract milliseconds.
pub(crate) fn seconds(value: &Value) -> Option<Timestamp> {
    value.as_i64().map(|s| Timestamp(s.saturating_mul(1000)))
}

/// A string field, or empty.
pub(crate) fn text<'a>(value: &'a Value, field: &str) -> &'a str {
    value.get(field).and_then(Value::as_str).unwrap_or_default()
}

/// The array under `field`, or empty.
pub(crate) fn list<'a>(value: &'a Value, field: &str) -> &'a [Value] {
    value.get(field).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}
