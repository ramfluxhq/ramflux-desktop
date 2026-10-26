// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Verified peer kinds, name warnings, stale verification, authenticated
//! renames and task actors, as the translator reads them from the daemon's
//! contact, identity, request, stream, task and message rows.

mod common;

use ramflux_desktop_core::{
    AccountId, ContextId, DomainEvent, KindClaim, MomentKind, Participant, ParticipantId,
    ParticipantKind, PeerKind, SafetyState, TaskEventKind, Timestamp,
};
use ramflux_desktop_sdk::Scope;
use ramflux_desktop_sdk::plan::fetch::{Fetch, Paged};
use ramflux_desktop_sdk::translate::Translator;
use ramflux_desktop_sdk::translate::stream::Frame;
use serde_json::{Value, json};

fn translator() -> Translator {
    let mut t = Translator::new(Scope::Account(AccountId::new("acct-main")), "/tmp/rfd.sock");
    let frames = common::frames();
    t.fetched(&Fetch::first(Paged::Identities), &frames["identity.directory"], Timestamp(1));
    t.fetched(&Fetch::first(Paged::Conversations), &frames["conversation.directory"], Timestamp(1));
    t
}

fn participants(events: &[DomainEvent]) -> Vec<Participant> {
    events
        .iter()
        .filter_map(|e| match e {
            DomainEvent::ParticipantUpserted(p) => Some(p.clone()),
            _ => None,
        })
        .collect()
}

fn one(events: &[DomainEvent], id: &str) -> Participant {
    participants(events).into_iter().find(|p| p.id.as_str() == id).unwrap_or_else(|| {
        panic!("{id} reported in {events:?}");
    })
}

fn link(principal: &str, extra: Value) -> Value {
    let mut row = json!({
        "link_id": format!("link-{principal}"), "requester_id": "principal-me",
        "target_id": principal, "state": "accepted", "blocked": false,
        "principal_id": principal, "reachable": true, "verified": false,
        "root_commitment": format!("root-{principal}"), "root_source": "lookup", "node": null,
    });
    if let (Value::Object(row), Value::Object(extra)) = (&mut row, extra) {
        row.extend(extra);
    }
    row
}

#[test]
fn a_root_signed_profile_proves_the_kind_and_a_card_only_claims_it() {
    let mut t = translator();
    let contacts = json!({ "contacts": [
        link("principal-kai", json!({ "display_name": "Kai", "kind_asserted": "human",
                                      "kind_verified": true })),
        link("principal-bot", json!({ "display_name": "Ledger", "kind_asserted": "headless_ai",
                                      "kind_verified": false })),
        link("principal-legacy", json!({ "display_name": null, "kind_asserted": null,
                                         "kind_verified": false })),
    ] });
    let events = t.fetched(&Fetch::ContactList, &contacts, Timestamp(2)).events;
    let kai = one(&events, "principal-kai");
    assert_eq!(
        (kai.kind, kai.kind_claim),
        (ParticipantKind::Human, KindClaim::Verified(PeerKind::Human))
    );
    assert_eq!(kai.display, "Kai");
    assert_eq!(
        kai.safety,
        SafetyState::Unverified,
        "a signed kind is not a verified safety number"
    );
    let bot = one(&events, "principal-bot");
    assert_eq!(bot.kind_claim, KindClaim::Asserted(PeerKind::HeadlessAi));
    assert_eq!(bot.kind, ParticipantKind::Agent);
    assert!(!bot.name_warnings.any());
    // The safe default: nothing claimed, nothing verified, shown by id.
    let legacy = one(&events, "principal-legacy");
    assert_eq!((legacy.kind, legacy.kind_claim), (ParticipantKind::Unknown, KindClaim::None));
    assert_eq!(legacy.safety, SafetyState::Unverified);
    assert!(!legacy.named());
}

#[test]
fn name_warnings_and_a_stale_verification_come_from_the_identity_directory() {
    let mut t = translator();
    let identities = json!({ "identities": [
        { "principal_id": "principal-kai", "display_name": "Kаi", "kind": "human",
          "kind_verified": true, "safety_state": "verification_stale", "devices": [],
          "display_name_confusable_with": "principal-kay", "mixed_script": true,
          "root_source": "verified" },
        { "principal_id": "principal-lu", "display_name": "Lu", "kind": "bot",
          "kind_verified": false, "safety_state": "verified", "devices": [],
          "display_name_confusable_with": null, "mixed_script": false },
    ], "next_cursor": null });
    let events = t.fetched(&Fetch::first(Paged::Identities), &identities, Timestamp(2)).events;
    let kai = one(&events, "principal-kai");
    assert_eq!(kai.kind_claim, KindClaim::Verified(PeerKind::Human));
    assert_eq!(kai.safety, SafetyState::VerificationStale, "verify again");
    assert!(kai.safety.needs_attention(), "sticky like a key change");
    assert!(kai.name_warnings.mixed_script);
    assert_eq!(
        kai.name_warnings.confusable_with.as_ref().map(ParticipantId::as_str),
        Some("principal-kay")
    );
    let lu = one(&events, "principal-lu");
    assert_eq!(lu.kind_claim, KindClaim::Asserted(PeerKind::Bot));
    assert_eq!(lu.safety, SafetyState::Verified);
    assert!(!lu.name_warnings.any());

    // A contact row with only a card claim does not lower a verified one,
    // and a rename through the profile reaches the name.
    let contacts = json!({ "contacts": [
        link("principal-kai", json!({ "display_name": "Kаi", "kind_asserted": "human",
                                      "kind_verified": false, "mixed_script": true,
                                      "display_name_confusable_with": "principal-kay" })),
    ] });
    let events = t.fetched(&Fetch::ContactList, &contacts, Timestamp(3)).events;
    let kai = one(&events, "principal-kai");
    assert_eq!(kai.kind_claim, KindClaim::Verified(PeerKind::Human), "the proven claim stays");
    assert_eq!(kai.safety, SafetyState::VerificationStale);
    assert!(kai.name_warnings.mixed_script);
    assert!(kai.contact, "the link is what the row adds");
}

#[test]
fn a_request_row_says_what_the_daemon_knows_of_the_requester() {
    let mut t = translator();
    let listed = json!({ "requests": [{
        "request_id": "req-1", "principal_id": "principal-zoe", "root_commitment": "root-zoe",
        "direction": "inbound", "state": "pending", "message_count": 0, "expires_at": 1_790_604_800,
        "display_name": "Zoe", "kind_asserted": "human", "kind_verified": true,
        "display_name_confusable_with": "principal-zoey", "mixed_script": false,
    }], "contact_requests": "manual" });
    let events = t.fetched(&Fetch::ContactRequests, &listed, Timestamp(2)).events;
    let zoe = one(&events, "principal-zoe");
    assert_eq!(zoe.display, "Zoe");
    assert_eq!(zoe.kind_claim, KindClaim::Verified(PeerKind::Human));
    assert_eq!(
        zoe.name_warnings.confusable_with.as_ref().map(ParticipantId::as_str),
        Some("principal-zoey")
    );
    assert_eq!(zoe.reachable, None, "a request row says nothing about reachability");
    // A count-only event names the requester without touching those facts.
    let event = json!({ "request_id": "req-1", "principal_id": "principal-zoe",
                        "root_commitment": "root-zoe", "direction": "inbound",
                        "state": "pending", "message_count": 1 });
    let events = t.contact_request_event("contact.requested", &event).events;
    assert!(participants(&events).is_empty(), "the facts stay: {events:?}");
}

#[test]
fn an_authenticated_rename_is_a_name_change_in_every_context_of_the_principal() {
    let mut t = translator();
    let body = json!({ "principal_id": "principal-ana", "reason": "public_profile_renamed",
                       "old_display_name": "Ana", "new_display_name": "Ana Reyes" });
    let frame = Frame { seq: Some(41), at: Some(1_790_000_500) };
    let translated = t.streamed("contact.changed", &body, frame, Timestamp(5));
    let changes: Vec<_> = translated
        .events
        .iter()
        .filter_map(|e| match e {
            DomainEvent::MomentAdded(m) => match &m.kind {
                MomentKind::NameChange(p) => {
                    Some((m.context_id.clone(), m.origin.clone(), p.clone(), m.at))
                }
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(changes.len(), 1, "only the conversation with Ana: {changes:?}");
    let (context, origin, payload, at) = &changes[0];
    assert_eq!(context, &ContextId::new("dm-ana"));
    assert_eq!(origin.as_str(), "principal-ana");
    assert_eq!((payload.old_name.as_str(), payload.new_name.as_str()), ("Ana", "Ana Reyes"));
    assert_eq!(*at, Timestamp(1_790_000_500_000), "the stream entry's time, not arrival");
    assert!(
        translated.follow.contains(&Fetch::ContactList)
            && translated.follow.contains(&Fetch::first(Paged::Identities)),
        "the rows are read again: {:?}",
        translated.follow
    );
    // The same announcement again (a replay) adds nothing.
    let again = t.streamed("contact.changed", &body, frame, Timestamp(6));
    assert!(again.events.iter().all(|e| !matches!(e, DomainEvent::MomentAdded(_))), "{again:?}");
    let remembered = t.take_new_renames();
    assert_eq!(remembered.len(), 1, "remembered once: {remembered:?}");
    // Renamed back and forth: every rename is its own line (keyed by its
    // stream entry, not by the names).
    let back = json!({ "principal_id": "principal-ana", "reason": "public_profile_renamed",
                       "old_display_name": "Ana Reyes", "new_display_name": "Ana" });
    let added = |translated: &ramflux_desktop_sdk::translate::Translated| {
        translated.events.iter().filter(|e| matches!(e, DomainEvent::MomentAdded(_))).count()
    };
    let at = |seq: u64| Frame {
        seq: Some(seq),
        at: Some(1_790_000_000 + i64::try_from(seq).unwrap_or(0)),
    };
    assert_eq!(added(&t.streamed("contact.changed", &back, at(42), Timestamp(7))), 1);
    assert_eq!(
        added(&t.streamed("contact.changed", &body, at(43), Timestamp(8))),
        1,
        "A to B again"
    );
    // Another contact change is not a rename.
    let other = json!({ "link_id": "l", "requester_id": "a", "target_id": "b" });
    let other = t.streamed("contact.changed", &other, Frame::default(), Timestamp(9));
    assert!(other.events.is_empty(), "{other:?}");
}

#[test]
fn task_actors_and_event_lines_carry_the_verified_kind() {
    let mut t = translator();
    let view = json!({
        "id": "t-1", "task_id": "t-1", "conversation_id": "thread-team", "version": 2,
        "state": "running", "title": "Inspect the logs", "assignee": "principal-bo",
        "requester": "principal-me", "steps": [], "actor_kind_verified": false,
        "actors": {
            "requester": { "principal_id": "principal-me", "device_id": "device-me",
                           "actor_kind": "human", "actor_kind_verified": false },
            "assignee": { "principal_id": "principal-bo", "device_id": "device-bo",
                          "actor_kind": "headless_ai", "actor_kind_verified": true },
            "acceptor": { "principal_id": null, "device_id": "", "actor_kind": null,
                          "actor_kind_verified": false },
        },
    });
    let fetch = Fetch::Task(ContextId::new("thread-team"), "t-1".to_owned());
    let events = t.fetched(&fetch, &view, Timestamp(2)).events;
    let task = events
        .iter()
        .find_map(|e| match e {
            DomainEvent::TaskSnapshot(task) | DomainEvent::TaskUpserted(task) => Some(task.clone()),
            _ => None,
        })
        .expect("the task");
    assert!(task.assignee_kind_verified());
    let assignee = task.actors.assignee.expect("assignee");
    assert_eq!(assignee.kind, Some(PeerKind::HeadlessAi));
    assert_eq!(assignee.device_id.as_ref().map(|d| d.as_str()), Some("device-bo"));
    let requester = task.actors.requester.expect("requester");
    assert_eq!((requester.kind, requester.kind_verified), (Some(PeerKind::Human), false));
    assert!(task.actors.acceptor.is_none(), "nobody accepted yet");

    // The event line shows the row's flag; a sibling copy without the
    // fields is read from its text.
    let rendering = |unverified: &str| {
        format!(
            "Signed event signer=\"device:device-bo\" kind=\"accept\" content=\"Task t-1 accepted{unverified}\""
        )
    };
    let messages = json!({ "messages": [
        { "conversation_id": "thread-team", "message_id": "m-acc-1", "sender_id": "device-bo",
          "content_type": "task", "task_id": "t-1", "actor_kind_verified": true,
          "body_utf8": rendering(""), "deleted": false, "created_at": 1_790_000_100, "receipts": [] },
        { "conversation_id": "thread-team", "message_id": "m-acc-2", "sender_id": "principal-me",
          "body_utf8": rendering(" (actor kind unverified)"), "deleted": false,
          "created_at": 1_790_000_200, "receipts": [] },
    ] });
    let events =
        t.fetched(&Fetch::Messages(ContextId::new("thread-team")), &messages, Timestamp(3)).events;
    let lines: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            DomainEvent::MomentAdded(m) => match &m.kind {
                MomentKind::TaskEvent(p) => {
                    Some((m.id.as_str().to_owned(), p.event.clone(), p.actor_kind_verified))
                }
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 2, "{events:?}");
    assert!(lines.iter().any(|(id, event, verified)| id.contains("m-acc-1")
        && *event == TaskEventKind::Accepted
        && *verified));
    assert!(lines.iter().any(|(id, event, verified)| id.contains("m-acc-2")
        && *event == TaskEventKind::Accepted
        && !*verified));
}

#[test]
fn remembered_renames_are_shown_again_after_a_restart_once_listed() {
    let record = ramflux_desktop_sdk::renames::RenameRecord {
        principal_id: "principal-ana".into(),
        old_name: "Ana".into(),
        new_name: "Ana Reyes".into(),
        seq: Some(41),
        at: 1_790_000_500,
    };
    let mut t = Translator::new(Scope::Account(AccountId::new("acct-main")), "/tmp/rfd.sock");
    t.restore_renames(vec![record]);
    let frames = common::frames();
    let fetch = Fetch::first(Paged::Conversations);
    let events = t.fetched(&fetch, &frames["conversation.directory"], Timestamp(1)).events;
    let shown: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            DomainEvent::MomentAdded(m) if matches!(m.kind, MomentKind::NameChange(_)) => {
                Some((m.context_id.clone(), m.at))
            }
            _ => None,
        })
        .collect();
    assert_eq!(shown, [(ContextId::new("dm-ana"), Timestamp(1_790_000_500_000))]);
    assert!(t.take_new_renames().is_empty(), "already remembered");
}
