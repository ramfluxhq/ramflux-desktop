// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Complete conversation directory, search and receipt presentations.
//! Previews borrow the body cache, so lock/reset cannot leave a retained copy.

use crate::{
    Context, ContextId, ContextKind, LockState, MomentKind, Receipt, Store, Timestamp, selectors,
};

/// Maximum number of rows returned in one page.
pub const PAGE_LIMIT: usize = 100;
/// Maximum UTF-8 bytes accepted in a search query.
pub const QUERY_LIMIT: usize = 1024;

/// Content state independent of localized interface wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preview<'a> {
    /// Text currently present in the body cache.
    Text(&'a str),
    /// Body not loaded or already evicted.
    Unavailable,
    /// The latest message was deleted; do not reveal its cached text.
    Deleted,
    /// Media or an object without a text body.
    Attachment,
    /// Latest item is a service, task, call or other non-message moment.
    Activity,
    /// No timeline yet.
    Empty,
}

/// One conversation row. Identity is the context ID, never its position.
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub struct ConversationRow<'a> {
    /// The source context, including unread, mute, pin and member facts.
    pub context: &'a Context,
    /// Resolved display title.
    pub title: String,
    /// Latest stream item's visible content.
    pub preview: Preview<'a>,
    /// Display time of the latest stream item, if one exists.
    pub at: Option<Timestamp>,
    /// Outgoing message receipt; never fabricated for incoming messages.
    pub receipt: Option<Receipt>,
    /// Number of pending decisions in this context.
    pub attention: usize,
}

/// A bounded result with an offset for the next request against the current store.
#[derive(Debug)]
#[non_exhaustive]
pub struct ConversationPage<'a> {
    /// Rows in pin/recent/id order.
    pub rows: Vec<ConversationRow<'a>>,
    /// Next offset, or `None` at the end. Reset to zero when the store changes.
    pub next_offset: Option<usize>,
}

/// Invalid query. This is a local resource limit, not a protocol limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QueryError {
    /// Search input exceeds the bounded local query budget.
    #[error("search query exceeds 1024 bytes")]
    TooLong,
}

/// Returns every matching conversation through bounded pages, including unpinned rows.
/// Searches titles, participant names/IDs and currently cached message bodies.
/// A locked account yields no rows. No content is loaded as a side effect.
///
/// # Errors
/// Returns [`QueryError::TooLong`] for an oversized query.
pub fn conversations<'a>(
    store: &'a Store,
    query: &str,
    offset: usize,
    limit: usize,
) -> Result<ConversationPage<'a>, QueryError> {
    conversations_with_activity(store, query, offset, limit, |_| None)
}

/// Builds a page with source directory activity, including unloaded timelines.
/// `activity` must read the same account snapshot as `store`.
///
/// # Errors
/// Returns [`QueryError::TooLong`] for an oversized query.
pub fn conversations_with_activity<'a>(
    store: &'a Store,
    query: &str,
    offset: usize,
    limit: usize,
    activity: impl Fn(&ContextId) -> Option<Timestamp>,
) -> Result<ConversationPage<'a>, QueryError> {
    if query.len() > QUERY_LIMIT {
        return Err(QueryError::TooLong);
    }
    if store.accounts().next().is_some()
        && !store.accounts().any(|a| a.lock_state == LockState::Unlocked)
    {
        return Ok(ConversationPage { rows: Vec::new(), next_offset: None });
    }
    let query = query.trim().to_lowercase();
    let mut contexts: Vec<_> = store
        .contexts()
        .filter(|context| {
            query.is_empty()
                || selectors::context_title(store, context).to_lowercase().contains(&query)
                || context.id.as_str().to_lowercase().contains(&query)
                || context.participants.iter().any(|id| {
                    id.as_str().to_lowercase().contains(&query)
                        || store
                            .participant(id)
                            .is_some_and(|p| p.display.to_lowercase().contains(&query))
                })
                || store.context_moments(&context.id).any(|moment| {
                    matches!(&moment.kind, MomentKind::Message(p) if !p.tombstone)
                        && store
                            .body(&moment.id)
                            .is_some_and(|body| body.to_lowercase().contains(&query))
                })
        })
        .collect();
    let latest_at =
        |id: &ContextId| activity(id).max(store.context_moments(id).next_back().map(|m| m.at));
    contexts.sort_by(|a, b| {
        let latest = &latest_at;
        b.pinned
            .cmp(&a.pinned)
            .then_with(|| latest(&b.id).cmp(&latest(&a.id)))
            .then_with(|| a.id.cmp(&b.id))
    });
    let limit = limit.clamp(1, PAGE_LIMIT);
    let next_offset = offset.checked_add(limit).filter(|next| *next < contexts.len());
    let local = selectors::self_participant(store);
    let rows = contexts
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|context| {
            let latest = store.context_moments(&context.id).next_back();
            let preview = match latest.map(|m| &m.kind) {
                Some(MomentKind::Message(p)) if p.tombstone => Preview::Deleted,
                Some(MomentKind::Message(p)) => match latest.and_then(|m| store.body(&m.id)) {
                    Some(text) if !text.is_empty() => Preview::Text(text),
                    _ if p.attachment.is_some() || !p.media.is_empty() => Preview::Attachment,
                    _ => Preview::Unavailable,
                },
                Some(_) => Preview::Activity,
                None => Preview::Empty,
            };
            let receipt = latest.and_then(|m| match &m.kind {
                MomentKind::Message(p) if local.as_ref() == Some(&m.origin) => Some(p.receipt),
                _ => None,
            });
            ConversationRow {
                context,
                title: selectors::context_title(store, context),
                preview,
                at: latest_at(&context.id),
                receipt,
                attention: selectors::needs_me_count(store, &context.id),
            }
        })
        .collect();
    Ok(ConversationPage { rows, next_offset })
}

/// Number of pending incoming requests and held messages for the strangers entry.
#[must_use]
pub fn strangers(store: &Store) -> (usize, u64) {
    store
        .contact_requests()
        .filter(|r| r.awaits_answer())
        .fold((0, 0), |(count, held), request| (count + 1, held + u64::from(request.held_messages)))
}

/// Whether a row represents a group rather than a direct peer.
#[must_use]
pub fn is_group(row: &ConversationRow<'_>) -> bool {
    row.context.kind == ContextKind::Group
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DomainEvent, EventEnvelope, Lifecycle, MessagePayload, Moment, MomentId, ParticipantId,
    };

    fn apply(store: &mut Store, seq: u64, event: DomainEvent) {
        store
            .apply(&EventEnvelope::new(seq, Timestamp(1), event))
            .expect("test: valid domain event");
    }

    #[test]
    fn directory_is_complete_searchable_and_bounded() {
        let mut store = Store::new();
        for n in 0..12 {
            let context: Context = serde_json::from_value(serde_json::json!({
                "id":format!("c-{n:02}"), "kind":"Direct", "title":format!("Peer {n}")
            }))
            .expect("test: minimal context");
            apply(&mut store, n + 1, DomainEvent::ContextUpserted(context));
        }
        let first = conversations(&store, "", 0, 7).expect("test: bounded query");
        assert_eq!(first.rows.len(), 7, "directory must exceed the compact stack's six rows");
        assert_eq!(first.next_offset, Some(7), "paging must expose the remaining rows");
        assert_eq!(
            conversations(&store, "", 7, 7).expect("test: second page").rows.len(),
            5,
            "pages must not lose directory rows"
        );
        assert_eq!(
            conversations(&store, "PEER 11", 0, 100).expect("test: title search").rows.len(),
            1,
            "search must fold case"
        );
        assert_eq!(
            conversations(&store, &"x".repeat(QUERY_LIMIT + 1), 0, 100).err(),
            Some(QueryError::TooLong),
            "oversized queries must fail before allocating search state"
        );
    }

    #[test]
    fn receipt_unread_and_lock_are_source_facts() {
        let mut store = Store::new();
        let account=serde_json::from_value(serde_json::json!({"id":"a","label":"Owner","lock_state":"Unlocked","self_participant_id":"self"})).expect("test: account fixture");
        apply(&mut store, 1, DomainEvent::AccountUpdated(account));
        let context = serde_json::from_value(
            serde_json::json!({"id":"c","kind":"Direct","title":"Peer","unread":4}),
        )
        .expect("test: context fixture");
        apply(&mut store, 2, DomainEvent::ContextUpserted(context));
        let message = Moment {
            id: MomentId::new("m"),
            context_id: ContextId::new("c"),
            origin: ParticipantId::new("self"),
            at: Timestamp(10),
            order: None,
            lifecycle: Lifecycle::Done,
            kind: MomentKind::Message(MessagePayload {
                text: "cached words".into(),
                receipt: Receipt::Delivered,
                ..Default::default()
            }),
        };
        apply(&mut store, 3, DomainEvent::MomentAdded(Box::new(message)));
        let rows = conversations(&store, "", 0, 100).expect("test: unlocked directory");
        let row = rows.rows.first().expect("test: one directory row");
        assert_eq!(
            row.receipt,
            Some(Receipt::Delivered),
            "receipt cannot advance merely because a row is displayed"
        );
        assert_eq!(row.context.unread, 4, "projection cannot consume unread state");
        assert_eq!(
            row.preview,
            Preview::Text("cached words"),
            "preview must come from the live body cache"
        );
        let account=serde_json::from_value(serde_json::json!({"id":"a","label":"Owner","lock_state":"Locked","self_participant_id":"self"})).expect("test: locked account fixture");
        apply(&mut store, 4, DomainEvent::AccountUpdated(account));
        assert!(
            conversations(&store, "", 0, 100).expect("test: locked directory").rows.is_empty(),
            "locking must hide content and metadata"
        );
    }

    #[test]
    fn deleted_message_never_reveals_cached_body_or_matches_search() {
        let mut store = Store::new();
        let context =
            serde_json::from_value(serde_json::json!({"id":"c", "kind":"Direct", "title":"Peer"}))
                .expect("test: context");
        apply(&mut store, 1, DomainEvent::ContextUpserted(context));
        let message = Moment {
            id: MomentId::new("m"),
            context_id: ContextId::new("c"),
            origin: ParticipantId::new("peer"),
            at: Timestamp(1),
            order: None,
            lifecycle: Lifecycle::Done,
            kind: MomentKind::Message(MessagePayload {
                text: "private words".into(),
                tombstone: true,
                ..Default::default()
            }),
        };
        apply(&mut store, 2, DomainEvent::MomentAdded(Box::new(message)));
        assert_eq!(
            conversations(&store, "", 0, 100)
                .expect("test: directory")
                .rows
                .first()
                .map(|r| r.preview),
            Some(Preview::Deleted),
            "tombstone must win over payload and cache"
        );
        assert!(
            conversations(&store, "private", 0, 100).expect("test: search").rows.is_empty(),
            "deleted text must not match search"
        );
    }
}
