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
    /// Latest activity is newer than the loaded timeline, or its body is evicted.
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
/// Contexts/moments must come from one account session. Other account metadata
/// may be listed, but combining several accounts' context stores is unsupported.
///
/// # Errors
/// Returns [`QueryError::TooLong`] for an oversized query.
pub fn conversations<'a>(
    store: &'a Store,
    query: &str,
    offset: usize,
    limit: usize,
) -> Result<ConversationPage<'a>, QueryError> {
    conversations_with_activity(store, query, offset, limit, |id| {
        store.context(id).and_then(|context| context.last_activity)
    })
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
    let rows = ordered_conversations_with_activity(store, query, activity)?;
    let limit = limit.clamp(1, PAGE_LIMIT);
    let next_offset = offset.checked_add(limit).filter(|next| *next < rows.len());
    Ok(ConversationPage { rows: rows.into_iter().skip(offset).take(limit).collect(), next_offset })
}

/// Filters once, snapshots each activity once and sorts one complete directory.
/// Native virtual lists consume this; paged adapters use [`conversations`].
///
/// # Errors
/// Returns [`QueryError::TooLong`] before normalizing an oversized query.
pub fn ordered_conversations_with_activity<'a>(
    store: &'a Store,
    query: &str,
    activity: impl Fn(&ContextId) -> Option<Timestamp>,
) -> Result<Vec<ConversationRow<'a>>, QueryError> {
    if query.len() > QUERY_LIMIT {
        return Err(QueryError::TooLong);
    }
    if !is_visible(store) {
        return Ok(Vec::new());
    }
    let query = query.trim().to_lowercase();
    let mut rows: Vec<_> = store
        .contexts()
        .filter(|context| matches_query(store, context, &query))
        .map(|context| row(store, context, activity(&context.id)))
        .collect();
    rows.sort_by(|a, b| {
        b.context
            .pinned
            .cmp(&a.context.pinned)
            .then_with(|| b.at.cmp(&a.at))
            .then_with(|| a.context.id.cmp(&b.context.id))
    });
    Ok(rows)
}

/// Whether the current account permits displaying directory content.
#[must_use]
pub fn is_visible(store: &Store) -> bool {
    store.accounts().next().is_none()
        || store.accounts().any(|a| a.lock_state == LockState::Unlocked)
}

/// Matches an already normalized query against one context and its loaded history.
#[must_use]
pub fn matches_query(store: &Store, context: &Context, query: &str) -> bool {
    query.is_empty()
        || selectors::context_title(store, context).to_lowercase().contains(query)
        || context.id.as_str().to_lowercase().contains(query)
        || context.participants.iter().any(|id| {
            id.as_str().to_lowercase().contains(query)
                || store.participant(id).is_some_and(|p| p.display.to_lowercase().contains(query))
        })
        || store.context_moments(&context.id).any(|moment| {
            matches!(&moment.kind, MomentKind::Message(p) if !p.tombstone)
                && store.body(&moment.id).is_some_and(|body| body.to_lowercase().contains(query))
        })
}

/// Projects one context from the current account snapshot, without sorting.
#[must_use]
pub fn row<'a>(
    store: &'a Store,
    context: &'a Context,
    activity: Option<Timestamp>,
) -> ConversationRow<'a> {
    let latest = store.context_moments(&context.id).next_back();
    let newer_unloaded = activity.is_some_and(|at| latest.is_none_or(|moment| at > moment.at));
    let preview = if newer_unloaded {
        Preview::Unavailable
    } else {
        match latest.map(|m| &m.kind) {
            Some(MomentKind::Message(p)) if p.tombstone => Preview::Deleted,
            Some(MomentKind::Message(p)) => match latest.and_then(|m| store.body(&m.id)) {
                Some(text) if !text.is_empty() => Preview::Text(text),
                _ if p.attachment.is_some() || !p.media.is_empty() => Preview::Attachment,
                _ => Preview::Unavailable,
            },
            Some(_) => Preview::Activity,
            None => Preview::Empty,
        }
    };
    let local = selectors::self_participant(store);
    let receipt = latest.filter(|_| !newer_unloaded).and_then(|m| match &m.kind {
        MomentKind::Message(p) if local.as_ref() == Some(&m.origin) => Some(p.receipt),
        _ => None,
    });
    ConversationRow {
        context,
        title: selectors::context_title(store, context),
        preview,
        at: activity.max(latest.map(|m| m.at)),
        receipt,
        attention: selectors::needs_me_count(store, &context.id),
    }
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
    fn large_directory_samples_activity_once_per_context() {
        const DIRECTORY_SIZE: u64 = 2000;
        let mut store = Store::new();
        for ix in 0..DIRECTORY_SIZE {
            let context = serde_json::from_value(
                serde_json::json!({"id":format!("c-{ix:04}"),"kind":"Direct","title":"Peer"}),
            )
            .expect("test: directory context");
            apply(&mut store, ix + 1, DomainEvent::ContextUpserted(context));
        }
        let calls = std::cell::Cell::new(0_u64);
        let rows = ordered_conversations_with_activity(&store, "peer", |_| {
            calls.set(calls.get() + 1);
            Some(Timestamp(10))
        })
        .expect("test: complete ordered snapshot");
        assert_eq!(rows.len(), DIRECTORY_SIZE as usize, "all directory rows must survive one pass");
        assert_eq!(
            calls.get(),
            DIRECTORY_SIZE,
            "sorting and projection must reuse one source activity snapshot per context"
        );
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
    fn ordering_and_second_page_vectors_match_the_shared_model() {
        let mut store = Store::new();
        for (seq, id, pinned) in [(1, "b", false), (2, "p", true), (3, "a", false)] {
            let context = serde_json::from_value(
                serde_json::json!({"id":id,"kind":"Direct","title":id,"pinned":pinned}),
            )
            .expect("test: context fixture");
            apply(&mut store, seq, DomainEvent::ContextUpserted(context));
        }
        let activity = |id: &ContextId| if id.as_str() == "p" { None } else { Some(Timestamp(10)) };
        let all = conversations_with_activity(&store, "", 0, 100, activity)
            .expect("test: ordering vector");
        assert_eq!(
            all.rows.iter().map(|r| r.context.id.as_str()).collect::<Vec<_>>(),
            ["p", "a", "b"],
            "native and shared models must use pin/activity/stable-id ordering"
        );
        let second = conversations_with_activity(&store, "", 1, 2, activity)
            .expect("test: second page vector");
        assert_eq!(
            second.rows.iter().map(|r| r.context.id.as_str()).collect::<Vec<_>>(),
            ["a", "b"],
            "native and shared models must page the same sorted snapshot"
        );
        assert_eq!(second.next_offset, None, "the final page must terminate");
    }

    #[test]
    fn newer_unloaded_activity_suppresses_old_preview_and_receipt() {
        let mut store = Store::new();
        let account=serde_json::from_value(serde_json::json!({"id":"a","label":"Owner","lock_state":"Unlocked","self_participant_id":"self"})).expect("test: account fixture");
        apply(&mut store, 1, DomainEvent::AccountUpdated(account));
        let context =
            serde_json::from_value(serde_json::json!({"id":"c","kind":"Direct","title":"Peer"}))
                .expect("test: context fixture");
        apply(&mut store, 2, DomainEvent::ContextUpserted(context));
        let message = Moment {
            id: MomentId::new("m"),
            context_id: ContextId::new("c"),
            origin: ParticipantId::new("self"),
            at: Timestamp(5_000),
            order: None,
            lifecycle: Lifecycle::Done,
            kind: MomentKind::Message(MessagePayload {
                text: "old cached words".into(),
                receipt: Receipt::Delivered,
                ..Default::default()
            }),
        };
        apply(&mut store, 3, DomainEvent::MomentAdded(Box::new(message)));
        let page = conversations_with_activity(&store, "", 0, 100, |_| Some(Timestamp(10_000)))
            .expect("test: newer directory activity");
        let row = page.rows.first().expect("test: one row");
        assert_eq!(
            row.at,
            Some(Timestamp(10_000)),
            "new source activity must retain its timestamp"
        );
        assert_eq!(
            row.preview,
            Preview::Unavailable,
            "a new timestamp cannot label an older cached body as current"
        );
        assert_eq!(
            row.receipt, None,
            "an older outgoing receipt cannot describe unloaded activity"
        );
        let equal = conversations_with_activity(&store, "", 0, 100, |_| Some(Timestamp(5_000)))
            .expect("test: equal directory activity");
        let row = equal.rows.first().expect("test: one equal-time row");
        assert_eq!(
            row.preview,
            Preview::Text("old cached words"),
            "an equally current loaded body remains displayable"
        );
        assert_eq!(
            row.receipt,
            Some(Receipt::Delivered),
            "an equally current receipt remains authoritative"
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
