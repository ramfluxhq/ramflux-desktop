// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Contact cards, contacts a resolving daemon reports as reachable, a new
//! group with its first members, and renaming a group conversation: the
//! requests the planner builds and what the translator makes of the
//! answers.

mod common;

use ramflux_desktop_core::{
    AccountId, ContactCard, ContextId, ContextKind, DomainEvent, Effect, FailureCode, Participant,
    ParticipantKind, SafetyState, Timestamp, methods,
};
use ramflux_desktop_sdk::Scope;
use ramflux_desktop_sdk::directory::Directory;
use ramflux_desktop_sdk::plan::fetch::{Fetch, Paged};
use ramflux_desktop_sdk::plan::{Known, Plan, plan};
use ramflux_desktop_sdk::translate::Translator;
use ramflux_sdk::{LocalBusGroupCreateRequest, LocalBusGroupMemberAddRequest};
use serde_json::{Value, json};

struct Fake(Directory);

impl Known for Fake {
    fn context_kind(&self, context: &ContextId) -> Option<ContextKind> {
        match context.as_str() {
            "thread-team" => Some(ContextKind::Group),
            "dm-ana" => Some(ContextKind::Direct),
            _ => None,
        }
    }
    fn self_device(&self) -> Option<&str> {
        Some("device-me")
    }
    fn daemon_grant_id(&self, _: &str) -> Option<&str> {
        None
    }
    fn directory(&self) -> Option<&Directory> {
        Some(&self.0)
    }
}

fn fake() -> Fake {
    Fake(common::directory())
}

fn call(method: &str, body: Value) -> Effect {
    Effect::Call { method: method.to_owned(), body }
}

fn planned(effect: Effect, known: &Fake) -> Plan {
    plan(Some(&AccountId::new("acct-main")), effect, known).plan
}

fn translator() -> Translator {
    Translator::new(Scope::Account(AccountId::new("acct-main")), "/tmp/rfd.sock")
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

#[test]
fn a_card_goes_to_the_daemon_as_pasted_and_the_contact_is_read_back() {
    let Plan::Call { request, refresh } =
        planned(call(methods::CONTACT_CARD_IMPORT, json!({ "card": "rf-card\nsig" })), &fake())
    else {
        panic!("a call");
    };
    assert_eq!((request.method.as_str(), request.sdk_api), ("contact.card.import", "contact"));
    assert_eq!(request.wire_body(), json!({ "card": "rf-card\nsig" }));
    assert!(request.operation_id.is_some(), "adding a contact is a mutation with a retry key");
    assert_eq!(refresh, vec![Fetch::ContactList, Fetch::first(Paged::Identities)]);
    let Plan::Refuse { code, .. } = planned(call(methods::CONTACT_CARD_IMPORT, json!({})), &fake())
    else {
        panic!("refused");
    };
    assert_eq!(code, FailureCode::Daemon("invalid".to_owned()));
}

#[test]
fn the_exported_card_is_kept_as_the_daemon_wrote_it() {
    let Plan::Call { request, .. } =
        planned(call(methods::CONTACT_CARD_EXPORT, json!({})), &fake())
    else {
        panic!("a call");
    };
    assert_eq!(request.method, "contact.card.export");
    assert!(request.operation_id.is_none(), "exporting the card changes nothing");
    let mut t = translator();
    let answer = json!({ "card": "rf-card-me", "link": "ramflux:contact/me" });
    let events = t.effect_done(methods::CONTACT_CARD_EXPORT, &json!({}), &answer, Timestamp(1));
    assert_eq!(
        events,
        vec![DomainEvent::OwnContactCardUpdated(ContactCard {
            text: "rf-card-me".to_owned(),
            link: Some("ramflux:contact/me".to_owned()),
        })]
    );
    let none = t.effect_done(methods::CONTACT_CARD_EXPORT, &json!({}), &json!({}), Timestamp(1));
    assert!(none.is_empty(), "no card, no event");
}

#[test]
fn a_resolving_daemon_reports_reachability_verification_and_the_claimed_kind() {
    let mut t = translator();
    let _ = t.fetched(
        &Fetch::AccountStatus(AccountId::new("acct-main")),
        &json!({ "local_account_id": "acct-main", "principal_id": "principal-me" }),
        Timestamp(0),
    );
    let record = |target: &str, extra: Value| {
        let mut record = json!({
            "link_id": format!("l-{target}"), "requester_id": "principal-me",
            "target_id": target, "state": "accepted", "blocked": false,
        });
        if let (Value::Object(record), Value::Object(extra)) = (&mut record, extra) {
            record.extend(extra);
        }
        record
    };
    let answer = record(
        "principal-kai",
        json!({ "reachable": true, "verified": false, "kind_asserted": "headless_ai" }),
    );
    let kai = participants(&t.effect_done(
        methods::CONTACT_CARD_IMPORT,
        &json!({}),
        &answer,
        Timestamp(1),
    ));
    assert_eq!(kai.len(), 1);
    assert_eq!(kai[0].reachable, Some(true));
    assert_eq!((kai[0].kind, kai[0].kind_asserted), (ParticipantKind::Agent, true));
    assert_eq!(kai[0].safety, SafetyState::Unverified);
    let again = participants(&t.effect_done(
        methods::CONTACT_CARD_IMPORT,
        &json!({}),
        &answer,
        Timestamp(1),
    ));
    assert_eq!(again, kai, "importing a known card still names the contact");

    let listed = json!({ "contacts": [
        record("principal-lu", json!({ "reachable": false, "verified": true })),
        record("principal-old", json!({})),
    ] });
    let people = participants(&t.fetched(&Fetch::ContactList, &listed, Timestamp(2)).events);
    let lu = people.iter().find(|p| p.id.as_str() == "principal-lu").expect("lu");
    assert_eq!((lu.reachable, lu.safety), (Some(false), SafetyState::Verified));
    let old = people.iter().find(|p| p.id.as_str() == "principal-old").expect("old");
    assert_eq!(
        (old.reachable, old.kind_asserted),
        (None, false),
        "an older daemon reports neither"
    );

    // The identity directory stays the authority for safety and kind.
    let identities = json!({ "identities": [{
        "principal_id": "principal-lu", "kind": "human", "safety_state": "unverified",
        "devices": [],
    }], "next_cursor": null });
    let people = participants(
        &t.fetched(&Fetch::first(Paged::Identities), &identities, Timestamp(3)).events,
    );
    let lu = people.iter().find(|p| p.id.as_str() == "principal-lu").expect("lu");
    assert_eq!(
        (lu.kind, lu.kind_asserted, lu.safety),
        (ParticipantKind::Human, false, SafetyState::Unverified)
    );
}

#[test]
fn a_group_is_created_with_its_conversation_then_each_member_device_added() {
    let body = json!({
        "context_id": "g-new", "title": "Weekend", "members": ["principal-ana", "principal-cy"],
    });
    let Plan::Calls { requests, refresh } = planned(call(methods::GROUP_CREATE, body), &fake())
    else {
        panic!("several calls");
    };
    assert_eq!(refresh, vec![Fetch::first(Paged::Groups), Fetch::first(Paged::Conversations)]);
    let methods_sent: Vec<&str> = requests.iter().map(|r| r.method.as_str()).collect();
    assert_eq!(
        methods_sent,
        ["group.create", "group.member.add", "group.member.add", "group.member.add"]
    );
    let create = requests[0].wire_body();
    assert_eq!(
        (create["conversation_id"].as_str(), create["title"].as_str()),
        (Some("g-new"), Some("Weekend"))
    );
    let typed: LocalBusGroupCreateRequest =
        serde_json::from_value(create.clone()).expect("the SDK type");
    assert!(typed.group_id.starts_with("group-") && typed.group_id.len() > 20, "a fresh group id");
    assert_eq!(typed.creator_id, "device-me");
    assert_eq!(typed.creator_target_delivery_id.as_deref(), Some("target-me"));
    assert!(typed.creator_signing_public_key.is_some(), "this device's verified key");
    let adds: Vec<LocalBusGroupMemberAddRequest> = requests[1..]
        .iter()
        .map(|r| serde_json::from_value(r.wire_body()).expect("the SDK type"))
        .collect();
    let devices: Vec<&str> = adds.iter().map(|a| a.member_id.as_str()).collect();
    assert_eq!(devices, ["device-ana", "device-cy-1", "device-cy-2"]);
    assert!(adds.iter().all(|a| a.group_id == typed.group_id && a.role == "member"));
    assert!(
        adds.iter()
            .all(|a| a.member_principal_commitment.is_some() && a.target_delivery_id.is_some())
    );
    assert!(requests.iter().all(|r| r.operation_id.is_some()), "every step has a retry key");
}

#[test]
fn a_member_without_a_verified_device_stops_the_creation_before_anything_is_sent() {
    let mut known = fake();
    if let Some(cy) = known.0.identities.get_mut("principal-cy") {
        cy.devices.clear();
    }
    let body = json!({ "context_id": "g-new", "title": "Weekend", "members": ["principal-ana", "principal-cy"] });
    let Plan::Refuse { code, message } = planned(call(methods::GROUP_CREATE, body), &known) else {
        panic!("refused");
    };
    assert_eq!(code, FailureCode::UnsupportedByBackend);
    assert!(
        message.starts_with("backend.failure.group_member_route")
            && message.contains("principal-cy"),
        "{message}"
    );
    let existing =
        json!({ "context_id": "thread-team", "title": "Again", "members": ["principal-ana"] });
    assert!(matches!(planned(call(methods::GROUP_CREATE, existing), &fake()), Plan::Refuse { .. }));
}

#[test]
fn a_rename_is_the_group_read_carrying_the_new_name_until_the_directory_lists_it() {
    let rename = json!({ "context_id": "thread-team", "title": "Crew" });
    let Plan::Call { request, refresh } =
        planned(call(methods::CONTEXT_RENAME, rename.clone()), &fake())
    else {
        panic!("a call");
    };
    assert_eq!(request.method, "group.receive");
    let body = request.wire_body();
    assert_eq!(
        (body["group_id"].as_str(), body["conversation_id"].as_str(), body["title"].as_str()),
        (Some("g-team"), Some("thread-team"), Some("Crew"))
    );
    assert_eq!(refresh, vec![Fetch::first(Paged::Conversations)]);
    let direct = json!({ "context_id": "dm-ana", "title": "Ana" });
    let Plan::Refuse { message, .. } = planned(call(methods::CONTEXT_RENAME, direct), &fake())
    else {
        panic!("refused");
    };
    assert_eq!(message, "backend.gap.no_group_link");

    // While the rename is on its way, group calls carry the new name; once
    // the directory lists it, the directory's name is used again.
    let mut t = translator();
    let rows = common::frames()["conversation.directory"].clone();
    let groups = common::frames()["group.list"].clone();
    let _ = t.fetched(&Fetch::first(Paged::Groups), &groups, Timestamp(0));
    let _ = t.fetched(&Fetch::first(Paged::Conversations), &rows, Timestamp(0));
    assert_eq!(t.kept_title(&ContextId::new("thread-team")), Some("Team"));
    t.rename_sent(methods::CONTEXT_RENAME, &rename);
    assert_eq!(t.kept_title(&ContextId::new("thread-team")), Some("Crew"));
    let _ = t.fetched(&Fetch::first(Paged::Conversations), &rows, Timestamp(1));
    assert_eq!(t.kept_title(&ContextId::new("thread-team")), Some("Crew"), "an older listing");
    let mut renamed = rows.clone();
    for row in renamed["conversations"].as_array_mut().into_iter().flatten() {
        if row["id"] == "thread-team" {
            row["title"] = json!("Crew");
        }
    }
    let _ = t.fetched(&Fetch::first(Paged::Conversations), &renamed, Timestamp(2));
    assert_eq!(t.kept_title(&ContextId::new("thread-team")), Some("Crew"));
    t.rename_sent(methods::CONTEXT_RENAME, &json!({ "context_id": "thread-team", "title": "Ops" }));
    let _ = t.effect_failed(
        methods::CONTEXT_RENAME,
        &json!({ "context_id": "thread-team", "title": "Ops" }),
        &FailureCode::Daemon("x".to_owned()),
        "refused",
        Timestamp(3),
    );
    assert_eq!(
        t.kept_title(&ContextId::new("thread-team")),
        Some("Crew"),
        "a refused rename is forgotten"
    );
}
