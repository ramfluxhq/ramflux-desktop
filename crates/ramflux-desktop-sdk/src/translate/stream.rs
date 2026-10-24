// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Topics of the daemon's event stream and what each one calls for.
//!
//! Event bodies carry ids and metadata, never content, so most events are
//! not shown as such: an event calls for the reads that refresh what it
//! names, and those reads report the differences as usual (the translator
//! emits only what changed, so a replayed or repeated event changes
//! nothing). A conversation someone else starts (`conversation.changed`,
//! or a `message.received` naming a conversation not shown yet) appears
//! through the conversation directory.
//!
//! The daemon's own state topics are read directly: `account.state` (this
//! account locked) and `sync.status` (its background sync, shown in
//! diagnostics).

use ramflux_desktop_core::{ContextId, ContextKind, DaemonSyncState, Timestamp};
use serde_json::Value;

use super::{Translator, seconds, text};
use crate::plan::fetch::{Fetch, Paged};

/// Topics whose events call for reads.
pub const READ_TOPICS: [&str; 15] = [
    "conversation.changed",
    "message.received",
    "message.updated",
    "group.message.received",
    "group.changed",
    "contact.changed",
    "safety.changed",
    "object.changed",
    "object.transfer.progress",
    "mcp.approval.request",
    "mcp.approval.granted",
    "mcp.approval.denied",
    "message.unrouted",
    "surface.changed",
    "surface.rejected",
];

/// The daemon's state topics, read directly.
pub const STATE_TOPICS: [&str; 2] = ["account.state", "sync.status"];

/// Every topic the desktop subscribes to on the stream. Not subscribed:
/// `node.health` (it names no node), tool starts and the older A2UI records
/// (they have no conversation to land in; conversation surfaces come through
/// `surface.changed`).
pub fn topics() -> Vec<&'static str> {
    READ_TOPICS.iter().chain(STATE_TOPICS.iter()).copied().collect()
}

/// The daemon's background sync as `sync.status` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DaemonSync {
    /// Its state, when the daemon named a known one.
    pub state: Option<DaemonSyncState>,
    /// When it last ran.
    pub last_run: Option<Timestamp>,
    /// Inbound messages it holds but cannot route yet.
    pub unrouted: Option<u64>,
}

impl DaemonSync {
    /// Reads a `sync.status` body.
    pub fn from_body(body: &Value) -> Self {
        let state = match text(body, "state") {
            "running" => Some(DaemonSyncState::Running),
            "idle" => Some(DaemonSyncState::Idle),
            "error" => Some(DaemonSyncState::Error),
            _ => None,
        };
        Self {
            state,
            last_run: body.get("last_run").and_then(seconds),
            unrouted: body.get("unrouted").and_then(Value::as_u64),
        }
    }
}

/// Whether an `account.state` body says `account` was locked.
pub fn locks(body: &Value, account: &str) -> bool {
    text(body, "local_account_id") == account && text(body, "state") == "locked"
}

/// The conversation an event names, in any of the shapes the daemon uses:
/// a top-level `conversation_id`, a conversation row (`conversation.id`),
/// a message row (`message.conversation_id`), or, for
/// `conversation.changed`, the row itself (`id`).
pub fn conversation_of(topic: &str, body: &Value) -> Option<ContextId> {
    let nested = |outer: &str, field: &str| {
        body.get(outer).and_then(|v| v.get(field)).and_then(Value::as_str).filter(|s| !s.is_empty())
    };
    let top = |field: &str| body.get(field).and_then(Value::as_str).filter(|s| !s.is_empty());
    top("conversation_id")
        .or_else(|| nested("conversation", "id"))
        .or_else(|| nested("message", "conversation_id"))
        .or_else(|| (topic == "conversation.changed").then(|| top("id")).flatten())
        .map(ContextId::new)
}

impl Translator {
    /// The reads a stream event calls for (none for other topics).
    pub fn stream_reads(&self, topic: &str, body: &Value) -> Vec<Fetch> {
        let conversations = Fetch::first(Paged::Conversations);
        let context = conversation_of(topic, body);
        let kind = context.as_ref().and_then(|c| self.contexts.get(c)).map(|c| c.kind);
        match topic {
            "conversation.changed" => vec![conversations],
            "message.received" | "message.updated" | "group.message.received" => {
                match (context, kind) {
                    (Some(context), Some(kind)) => {
                        let mut reads = self.messages_read(&context, kind, false);
                        if kind == ContextKind::Group && reads.is_empty() {
                            reads.push(Fetch::first(Paged::Groups));
                        }
                        reads.push(conversations);
                        reads
                    }
                    // A conversation not shown yet: the directory lists it.
                    _ => vec![conversations],
                }
            }
            "group.changed" => vec![Fetch::first(Paged::Groups), conversations],
            "contact.changed" | "safety.changed" => {
                vec![Fetch::ContactList, Fetch::first(Paged::Identities)]
            }
            "object.changed" | "object.transfer.progress" => vec![Fetch::first(Paged::Objects)],
            "mcp.approval.request" | "mcp.approval.granted" | "mcp.approval.denied" => {
                vec![Fetch::first(Paged::Approvals), Fetch::Grants]
            }
            // Held until the daemon can route it; it lists it then.
            "message.unrouted" => vec![conversations],
            // Metadata only: read the surface when it names another state
            // than the one shown (a rejection names the surface, if any).
            "surface.changed" | "surface.rejected" => {
                let mut reads: Vec<Fetch> = match text(body, "surface_ref") {
                    "" => Vec::new(),
                    _ if topic == "surface.changed" && !self.surface_stale(body) => Vec::new(),
                    reference => vec![Fetch::Surface(reference.to_owned())],
                };
                if context.is_some() && kind.is_none() {
                    reads.push(conversations);
                }
                reads
            }
            _ => Vec::new(),
        }
    }

    /// The reads that show the watched contexts' stored messages (no pull
    /// from the node: the daemon receives by itself).
    pub fn watched_reads(&self) -> Vec<Fetch> {
        self.watched()
            .into_iter()
            .flat_map(|(id, kind)| self.messages_read(&id, kind, false))
            .collect()
    }

    /// The read that shows one context's stored messages, if it is known.
    pub fn context_reads(&self, context: &ContextId) -> Vec<Fetch> {
        self.contexts
            .get(context)
            .map(|c| self.messages_read(context, c.kind, false))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::translate::Scope;

    fn translator() -> Translator {
        Translator::new(Scope::Account(ramflux_desktop_core::AccountId::new("a")), "s")
    }

    #[test]
    fn a_new_conversation_is_read_from_the_directory() {
        let translator = translator();
        let received = json!({
            "message": { "id": "m-1", "conversation_id": "new" },
            "conversation": { "id": "new" },
        });
        assert_eq!(
            translator.stream_reads("message.received", &received),
            [Fetch::first(Paged::Conversations)]
        );
        assert_eq!(
            translator.stream_reads("conversation.changed", &json!({ "id": "new" })),
            [Fetch::first(Paged::Conversations)]
        );
        assert!(translator.stream_reads("mcp.tool.started", &json!({})).is_empty());
        assert!(translator.stream_reads("sync.status", &json!({})).is_empty());
    }

    #[test]
    fn conversations_are_found_in_every_body_shape() {
        let id = Some(ContextId::new("c-1"));
        let receipt = json!({ "message_id": "m", "conversation_id": "c-1" });
        assert_eq!(conversation_of("message.updated", &receipt), id);
        let row = json!({ "message": { "conversation_id": "c-1" }, "conversation": {} });
        assert_eq!(conversation_of("message.received", &row), id);
        let joined = json!({ "message": {}, "conversation": { "id": "c-1" } });
        assert_eq!(conversation_of("group.message.received", &joined), id);
        assert_eq!(conversation_of("conversation.changed", &json!({ "id": "c-1" })), id);
        assert_eq!(conversation_of("object.changed", &json!({ "id": "o-1" })), None);
    }

    #[test]
    fn daemon_state_topics_are_read_directly() {
        let sync = DaemonSync::from_body(&json!({ "state": "idle", "last_run": 7, "unrouted": 2 }));
        assert_eq!(sync.state, Some(DaemonSyncState::Idle));
        assert_eq!(sync.last_run, Some(Timestamp(7_000)));
        assert_eq!(sync.unrouted, Some(2));
        assert_eq!(DaemonSync::from_body(&json!({ "state": "odd" })).state, None);
        let locked = json!({ "local_account_id": "a", "state": "locked" });
        assert!(locks(&locked, "a"));
        assert!(!locks(&locked, "b"));
        assert!(!locks(&json!({ "local_account_id": "a", "state": "unlocked" }), "a"));
    }
}
