// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Topics of the daemon's event stream that say a list changed.
//!
//! Event bodies carry ids and metadata, never content, so an event is not
//! shown as such: it calls for the reads that refresh what it names, and
//! those reads report the differences as usual. A new conversation from
//! someone else (`conversation.changed`, or a `message.received` naming a
//! conversation not shown yet) therefore appears through the conversation
//! directory, the same way the poll finds it. Until the daemon pushes these
//! topics the poll is the only path.

use ramflux_desktop_core::{ContextId, ContextKind};
use serde_json::Value;

use super::{Translator, text};
use crate::plan::fetch::{Fetch, Paged};

/// The list-changing topics the desktop subscribes to.
pub const TOPICS: [&str; 9] = [
    "conversation.changed",
    "message.received",
    "message.updated",
    "group.message.received",
    "group.changed",
    "contact.changed",
    "safety.changed",
    "object.changed",
    "object.transfer.progress",
];

impl Translator {
    /// The reads a pushed event calls for (none for other topics).
    pub fn pushed_reads(&self, topic: &str, body: &Value) -> Vec<Fetch> {
        let conversations = Fetch::first(Paged::Conversations);
        match topic {
            "conversation.changed" => vec![conversations],
            "message.received" | "message.updated" => {
                let context = ContextId::new(text(body, "conversation_id"));
                match self.contexts.get(&context).map(|c| c.kind) {
                    Some(ContextKind::Group) => vec![Fetch::first(Paged::Groups), conversations],
                    Some(_) => vec![Fetch::Messages(context), conversations],
                    // A conversation not shown yet: the directory lists it.
                    None => vec![conversations],
                }
            }
            "group.message.received" | "group.changed" => {
                let context = ContextId::new(text(body, "conversation_id"));
                let mut reads = vec![Fetch::first(Paged::Groups), conversations];
                if let Some(group) = self.directory.group_of(&context) {
                    reads.push(Fetch::GroupReceive { context, group: group.to_owned() });
                }
                reads
            }
            "contact.changed" | "safety.changed" => {
                vec![Fetch::ContactList, Fetch::first(Paged::Identities)]
            }
            "object.changed" | "object.transfer.progress" => vec![Fetch::first(Paged::Objects)],
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::translate::Scope;

    #[test]
    fn a_new_conversation_is_read_from_the_directory() {
        let translator =
            Translator::new(Scope::Account(ramflux_desktop_core::AccountId::new("a")), "s");
        let reads =
            translator.pushed_reads("message.received", &json!({ "conversation_id": "new" }));
        assert_eq!(reads, [Fetch::first(Paged::Conversations)]);
        assert_eq!(
            translator.pushed_reads("conversation.changed", &json!({})),
            [Fetch::first(Paged::Conversations)]
        );
        assert!(translator.pushed_reads("mcp.tool.started", &json!({})).is_empty());
    }
}
