// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Daemon answers and pushed events to domain events.
//!
//! The translator owns what the source already told the store (known nodes,
//! contexts, messages, grants), so it emits only differences: a re-snapshot
//! after a reconnect upserts what changed and removes what disappeared, and
//! a poll that returns the same messages emits nothing. It never invents
//! data the daemon did not report: a field the daemon reports as null stays
//! unknown (see the gap ledger), and ids are never derived from one another.
//!
//! - `pages`: collecting the pages of a paged list.
//! - `daemon`: daemon status, nodes, accounts.
//! - `account`: account status, devices, integrations.
//! - `identity`: identities (participants, devices, safety).
//! - `contacts`: participants and contact links.
//! - `conversation`: conversations and groups as contexts, pins, settings.
//! - `messages`: stored messages as message moments.
//! - `objects`: the object directory.
//! - `grants`: approvals, grants and standing approvals.
//! - `interventions`: approvals shown in their conversation.
//! - `grant_failures`: fail-closed grant errors.
//! - `node_failures`: why a node lookup failed (Local Network privacy).
//! - `send_failures`: why a send failed, in the user's terms.
//! - `outgoing`: a started conversation and an attachment on its way,
//!   before the daemon lists them.
//! - `stream`: the reads a pushed list-changing event calls for.
//! - `surfaces`: agent surfaces the daemon holds, as surface moments.
//! - `surface_answers`: the daemon's answers to surface effects.
//! - `tasks`: conversation tasks the daemon holds, as task moments.
//! - `task_answers`: the daemon's answers to task effects, on their task.
//! - `effects`: answers to effects, and the audit record of confirmed ones.
//! - `reason`: locale keys with arguments.

mod account;
mod contact_failures;
mod contacts;
mod conversation;
mod daemon;
mod effects;
mod grant_failures;
mod grants;
mod identity;
mod interventions;
mod messages;
pub(crate) mod node_failures;
mod objects;
mod outgoing;
pub mod pages;
pub mod reason;
mod send_failures;
pub mod stream;
mod surface_answers;
mod surfaces;
pub mod task_answers;
mod task_decisions;
mod task_events;
pub mod tasks;

use std::collections::{BTreeMap, BTreeSet};

use ramflux_desktop_core::{
    Account, AccountId, BodyCache, Context, ContextId, ContextKind, DeviceId, DomainEvent, GrantId,
    GrantState, Listing, MomentId, NodeId, Object, ObjectId, Participant, ParticipantId,
    StandingApproval, StandingApprovalId, Timestamp,
};
use serde_json::Value;

use crate::directory::Directory;
use crate::plan::Known;
use crate::plan::fetch::{Fetch, Paged};
use pages::{Pages, Paging};

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
    node_relays: BTreeMap<NodeId, String>,
    self_device: Option<String>,
    /// Whether `self_device` came from `account.status.device_id` (which
    /// `device.list` then does not override).
    status_device: bool,
    devices: BTreeMap<DeviceId, ramflux_desktop_core::Device>,
    directory: Directory,
    pages: Pages,
    people: BTreeMap<ParticipantId, contacts::Person>,
    reported_people: BTreeMap<ParticipantId, Participant>,
    links: BTreeMap<ParticipantId, crate::plan::KnownLink>,
    conversation_rows: Option<Vec<Value>>,
    contexts: BTreeMap<ContextId, Context>,
    pins: Vec<ContextId>,
    pins_changed: bool,
    moments: BTreeMap<MomentId, Reported>,
    resend: BTreeSet<MomentId>,
    plaintext: BodyCache,
    grants: BTreeMap<GrantId, GrantState>,
    bindings: BTreeMap<GrantId, grant_failures::Binding>,
    daemon_grants: BTreeMap<String, String>,
    interventions: BTreeMap<GrantId, interventions::Placement>,
    standing: BTreeMap<StandingApprovalId, StandingApproval>,
    objects: BTreeMap<ObjectId, Object>,
    sent_attachments: BTreeMap<String, ObjectId>,
    /// Conversations this device started that the directory has not
    /// listed yet: a listing read before the start does not remove them.
    started: BTreeSet<ContextId>,
    /// Names this device gave group conversations that the directory does
    /// not list yet; group calls carry them meanwhile.
    renames: BTreeMap<ContextId, String>,
    message_objects: BTreeMap<String, ObjectId>,
    /// Surfaces reported, by the daemon's scoped `surface_ref`.
    surfaces: BTreeMap<String, surfaces::KnownSurface>,
    /// When each surface was posted (conversation, surface id, author), from
    /// its first message.
    surface_times: BTreeMap<(ContextId, String, String), Timestamp>,
    /// Tasks reported, by desktop task id.
    tasks: BTreeMap<ramflux_desktop_core::TaskId, tasks::KnownTask>,
    /// The approval (desktop grant) that requested each daemon grant id.
    requested_grants: BTreeMap<String, String>,
    /// Tasks whose request was refused as changed: said again on the newer
    /// task read back.
    review_again: BTreeSet<ramflux_desktop_core::TaskId>,
    /// Surface actions answering a task step, waiting for the author's
    /// receipt.
    surface_decisions: Vec<task_decisions::SurfaceDecision>,
    /// Task resolutions to send, taken by the bridge after each read.
    resolutions: Vec<Value>,
    /// Action receipts a resolution already named.
    resolved_receipts: BTreeSet<String>,
    /// Surfaces read before the task step answered in them was shown: read
    /// again, so an action already listed there resolves the step.
    surface_rereads: Vec<String>,
    /// Tasks the daemon pruned (`task.changed` `kind: removed`).
    removed_tasks: BTreeSet<ramflux_desktop_core::TaskId>,
    mcp_tools: Option<BTreeMap<String, Vec<String>>>,
    mcp_servers: Option<Vec<Value>>,
    integrations: Option<ramflux_desktop_core::Integrations>,
    focus: Option<ContextId>,
    listed: BTreeSet<Listing>,
    completed: BTreeSet<Paged>,
    /// Received direct messages listed since the bridge last took them
    /// (delivery receipts to send, see [`crate::receipts`]).
    delivered: Vec<crate::receipts::Delivered>,
}

impl Translator {
    /// A translator for `scope`, attached to `socket` (shown as daemon info).
    pub fn new(scope: Scope, socket: impl Into<String>) -> Self {
        Self {
            scope,
            socket: socket.into(),
            nodes: BTreeMap::new(),
            accounts: BTreeMap::new(),
            node_relays: BTreeMap::new(),
            self_device: None,
            status_device: false,
            devices: BTreeMap::new(),
            directory: Directory::default(),
            pages: Pages::default(),
            people: BTreeMap::new(),
            reported_people: BTreeMap::new(),
            links: BTreeMap::new(),
            conversation_rows: None,
            contexts: BTreeMap::new(),
            pins: Vec::new(),
            pins_changed: false,
            moments: BTreeMap::new(),
            resend: BTreeSet::new(),
            plaintext: BodyCache::with_capacity(PLAINTEXT_CAPACITY),
            grants: BTreeMap::new(),
            bindings: BTreeMap::new(),
            daemon_grants: BTreeMap::new(),
            interventions: BTreeMap::new(),
            standing: BTreeMap::new(),
            objects: BTreeMap::new(),
            sent_attachments: BTreeMap::new(),
            started: BTreeSet::new(),
            renames: BTreeMap::new(),
            message_objects: BTreeMap::new(),
            surfaces: BTreeMap::new(),
            surface_times: BTreeMap::new(),
            tasks: BTreeMap::new(),
            requested_grants: BTreeMap::new(),
            review_again: BTreeSet::new(),
            surface_decisions: Vec::new(),
            resolutions: Vec::new(),
            resolved_receipts: BTreeSet::new(),
            surface_rereads: Vec::new(),
            removed_tasks: BTreeSet::new(),
            mcp_tools: None,
            mcp_servers: None,
            integrations: None,
            focus: None,
            listed: BTreeSet::new(),
            completed: BTreeSet::new(),
            delivered: Vec::new(),
        }
    }

    /// The scope.
    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    /// What the directories listed so far.
    pub fn directory(&self) -> &Directory {
        &self.directory
    }

    /// Records the context the user looks at.
    pub fn set_focus(&mut self, context: Option<ContextId>) {
        self.focus = context;
    }

    /// Seeds the pins recorded in the desktop settings (pin order); the
    /// daemon reports none.
    pub fn set_pins(&mut self, pins: Vec<ContextId>) {
        self.pins = pins;
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

    /// The read that refreshes one context's messages: a group conversation
    /// is received through the group the directory links it to (none when
    /// no group is linked).
    fn messages_read(&self, context: &ContextId, kind: ContextKind, pull: bool) -> Vec<Fetch> {
        let mut reads = self.message_reads(context, kind, pull);
        if !reads.is_empty() {
            // The conversation's surfaces come with its messages.
            reads.push(Self::surfaces_read(context));
            // The conversation's tasks come with its messages.
            reads.push(Fetch::Tasks(context.clone(), None));
        }
        reads
    }

    fn message_reads(&self, context: &ContextId, kind: ContextKind, pull: bool) -> Vec<Fetch> {
        match kind {
            ContextKind::Group => self
                .directory
                .group_of(context)
                .map(|group| Fetch::GroupReceive {
                    context: context.clone(),
                    group: group.to_owned(),
                    title: self.kept_title(context).map(str::to_owned),
                })
                .into_iter()
                .collect(),
            _ if pull => vec![Fetch::Receive(context.clone()), Fetch::Messages(context.clone())],
            _ => vec![Fetch::Messages(context.clone())],
        }
    }

    /// The reads that keep the watched contexts fresh.
    pub fn poll_reads(&self) -> Vec<Vec<Fetch>> {
        self.watched()
            .into_iter()
            .map(|(id, kind)| self.messages_read(&id, kind, true))
            .filter(|reads| !reads.is_empty())
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
        let complete = |paged| self.completed.contains(&paged);
        let listing = match fetch {
            Fetch::NodeList => Listing::Nodes,
            Fetch::AccountList => Listing::Accounts,
            // Contexts are conversations and groups: listed once both are.
            Fetch::Page(Paged::Conversations | Paged::Groups, _)
                if complete(Paged::Conversations) && complete(Paged::Groups) =>
            {
                Listing::Contexts
            }
            Fetch::ContactList => Listing::Contacts,
            Fetch::Page(Paged::Approvals, _) if complete(Paged::Approvals) => Listing::Approvals,
            Fetch::Page(Paged::Objects, _) if complete(Paged::Objects) => Listing::Objects,
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

    fn translate(&mut self, fetch: &Fetch, value: &Value, now: Timestamp) -> Translated {
        match fetch {
            Fetch::DaemonStatus => Translated::events(self.daemon_status(value)),
            Fetch::NodeList => Translated::events(self.node_list(value)),
            Fetch::AccountList => self.account_list(value),
            Fetch::AccountStatus(id) => Translated::events(self.account_status(id, value)),
            Fetch::DeviceList => Translated::events(self.device_list(value)),
            Fetch::Page(paged, cursor) => match self.pages.page(*paged, cursor.as_deref(), value) {
                Paging::More(next) => Translated { events: Vec::new(), follow: vec![next] },
                Paging::Dropped => Translated::default(),
                Paging::Complete(rows) => {
                    self.completed.insert(*paged);
                    self.listing(*paged, &rows, now)
                }
            },
            Fetch::ContactList => Translated::events(self.contact_list(value)),
            Fetch::Grants => Translated::events(self.grant_list(value, now)),
            Fetch::StandingApprovals => Translated::events(self.standing_list(value)),
            Fetch::McpTools => Translated::events(self.mcp_tools(value)),
            Fetch::Messages(context) => self.messages(context, value, now),
            Fetch::GroupReceive { context, .. } => {
                self.remember_plaintext(value);
                self.messages(context, value, now)
            }
            Fetch::Surfaces(context, _) => {
                Translated { events: Vec::new(), follow: self.surface_list(context, value) }
            }
            Fetch::Surface(reference) => Translated::events(self.surface_view(reference, value)),
            Fetch::Tasks(context, _) => {
                let mut translated = self.task_list(context, value);
                translated.follow.extend(self.take_surface_rereads());
                translated
            }
            Fetch::Task(..) => {
                Translated { events: self.task_view(value), follow: self.take_surface_rereads() }
            }
            Fetch::Receive(_) => {
                self.remember_plaintext(value);
                Translated::default()
            }
        }
    }

    /// Translates one complete paged listing.
    fn listing(&mut self, paged: Paged, rows: &[Value], now: Timestamp) -> Translated {
        match paged {
            Paged::Identities => Translated::events(self.identities(rows)),
            Paged::Groups => Translated::events(self.groups(rows)),
            Paged::Conversations => self.conversations(rows),
            Paged::Objects => Translated::events(self.object_listing(rows)),
            Paged::Approvals => Translated::events(self.approval_list(rows, now)),
            Paged::McpServers => Translated::events(self.mcp_servers(rows)),
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
            .filter_map(|id| self.contexts.get(&id).map(|c| (id.clone(), c.kind)))
            .flat_map(|(id, kind)| self.messages_read(&id, kind, false))
            .collect()
    }

    /// Translates one pushed event.
    pub fn pushed(&mut self, topic: &str, body: &Value, now: Timestamp) -> Vec<DomainEvent> {
        match topic {
            "mcp.approval.request" => {
                let id = GrantId::new(text(body, "approval_id"));
                if self.grants.contains_key(&id) {
                    self.approval_changed(&id, body)
                } else {
                    self.approval(body, now)
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
        self.directory.self_principal.as_deref()
    }

    fn contact_link(&self, participant: &str) -> Option<&crate::plan::KnownLink> {
        self.known_link(participant)
    }

    fn directory(&self) -> Option<&Directory> {
        Some(&self.directory)
    }

    fn surface(&self, surface_ref: &str) -> Option<&crate::surface::SurfaceTarget> {
        self.surface_target(surface_ref)
    }

    fn kept_title(&self, context: &ContextId) -> Option<&str> {
        self.renames.get(context).map(String::as_str).or_else(|| self.directory.title_of(context))
    }

    fn task(&self, task_id: &str) -> Option<&crate::plan::task::TaskTarget> {
        self.task_target(task_id)
    }

    fn surface_task_binding(&self, surface_ref: &str) -> Option<(String, String)> {
        self.surface_step(surface_ref)
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
