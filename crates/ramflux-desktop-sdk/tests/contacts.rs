// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Contact effects against the daemon's `contact.*` methods.
//!
//! Request bodies are checked against the product SDK's own request types
//! at the pinned revision (`LocalBusContactAddRequest`,
//! `LocalBusContactLinkRequest`, `LocalBusContactRemoveRequest`): each wire
//! body deserializes into the type and has exactly its fields. The fake
//! daemon answers like the daemon's friend-link store does (the same
//! record fields, the same transitions, the same error codes and messages).

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use common::*;
use ramflux_desktop_core::{
    AccountId, ContactLink, ContextId, ContextKind, Effect, FailureCode, Listing, ParticipantId,
    ParticipantKind, RootSource, Source as _, Store, methods,
};
use ramflux_desktop_sdk::plan::contact::REMOVE_SCOPE;
use ramflux_desktop_sdk::plan::fetch::{Fetch, Paged};
use ramflux_desktop_sdk::plan::{Known, KnownLink, Plan, plan};
use ramflux_sdk::{
    LocalBusContactAddRequest, LocalBusContactLinkRequest, LocalBusContactRemoveRequest,
};
use serde_json::{Value, json};

const ME: &str = "principal-me";

struct Fake {
    principal: Option<&'static str>,
    links: BTreeMap<&'static str, KnownLink>,
}

fn link(id: &str, requester: &str, target: &str, state: ContactLink) -> KnownLink {
    KnownLink {
        link_id: id.to_owned(),
        requester_id: requester.to_owned(),
        target_id: target.to_owned(),
        state,
    }
}

impl Fake {
    fn new() -> Self {
        let mut links = BTreeMap::new();
        links.insert("principal-ana", link("l1", ME, "principal-ana", ContactLink::Accepted));
        links.insert("principal-noor", link("l2", "principal-noor", ME, ContactLink::Incoming));
        links.insert("principal-ivo", link("l3", ME, "principal-ivo", ContactLink::Blocked));
        Self { principal: Some(ME), links }
    }
}

impl Known for Fake {
    fn context_kind(&self, _: &ContextId) -> Option<ContextKind> {
        None
    }
    fn self_device(&self) -> Option<&str> {
        Some("device-me")
    }
    fn daemon_grant_id(&self, _: &str) -> Option<&str> {
        None
    }
    fn self_principal(&self) -> Option<&str> {
        self.principal
    }
    fn contact_link(&self, participant: &str) -> Option<&KnownLink> {
        self.links.get(participant)
    }
}

fn call(method: &str, body: Value) -> Effect {
    Effect::Call { method: method.to_owned(), body }
}

fn on(participant: &str) -> Value {
    json!({ "participant_id": participant, "proposed_by": "Human", "confirmed_by": null })
}

/// (daemon method, wire body, refresh) of a planned call.
fn mapped(effect: Effect, known: &Fake) -> (String, Value, Vec<Fetch>) {
    match plan(Some(&AccountId::new("acct-main")), effect, known).plan {
        Plan::Call { request, refresh } => {
            assert_eq!(request.account.as_deref(), Some("acct-main"));
            assert_eq!(request.sdk_api, "contact");
            (request.method.clone(), request.wire_body(), refresh)
        }
        other => panic!("expected a call, got {other:?}"),
    }
}

fn refused(effect: Effect, known: &Fake) -> (FailureCode, String) {
    match plan(Some(&AccountId::new("acct-main")), effect, known).plan {
        Plan::Refuse { code, message } => (code, message),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// The body has exactly the SDK type's fields and deserializes into it.
fn sdk_shaped<T: serde::de::DeserializeOwned + serde::Serialize>(body: &Value) -> T {
    let typed: T = serde_json::from_value(body.clone()).expect("the SDK request type");
    assert_eq!(&serde_json::to_value(&typed).expect("json"), body, "no extra or missing field");
    typed
}

#[test]
fn every_contact_method_builds_the_sdk_request() {
    let known = Fake::new();
    let (m, body, refresh) = mapped(call(methods::CONTACT_ADD, on("principal-kai")), &known);
    assert_eq!(m, "contact.add");
    // The contact's identity follows the link: both are read again.
    assert_eq!(refresh, vec![Fetch::ContactList, Fetch::first(Paged::Identities)]);
    // Exactly the principal id: the daemon resolves it and derives the link
    // (it refuses any other field next to a selector).
    assert_eq!(body, json!({ "principal_id": "principal-kai" }));
    let (_, body, _) = mapped(call(methods::CONTACT_ADD, on("principal-ana")), &known);
    assert_eq!(body, json!({ "principal_id": "principal-ana" }));

    // Accept is the daemon's local form: the pending link's own ids.
    let (m, body, refresh) = mapped(call(methods::CONTACT_ACCEPT, on("principal-noor")), &known);
    assert_eq!(
        (m.as_str(), refresh),
        ("contact.accept", vec![Fetch::ContactList, Fetch::first(Paged::Identities)])
    );
    let accept: LocalBusContactAddRequest = sdk_shaped(&body);
    assert_eq!(
        (accept.link_id.as_str(), accept.requester_id.as_str(), accept.target_id.as_str()),
        ("l2", "principal-noor", ME)
    );
    // A moment-based answer names the requester as `from`.
    let (_, from_moment, _) = mapped(
        call(methods::CONTACT_ACCEPT, json!({ "moment_id": "m-1", "from": "principal-noor" })),
        &known,
    );
    assert_eq!(from_moment, body);

    for (method, participant, daemon, link_id) in [
        (methods::CONTACT_REJECT, "principal-noor", "contact.reject", "l2"),
        (methods::CONTACT_BLOCK, "principal-ana", "contact.block", "l1"),
        (methods::CONTACT_UNBLOCK, "principal-ivo", "contact.unblock", "l3"),
    ] {
        let (m, body, refresh) = mapped(call(method, on(participant)), &known);
        assert_eq!((m.as_str(), refresh), (daemon, vec![Fetch::ContactList]));
        let request: LocalBusContactLinkRequest = sdk_shaped(&body);
        assert_eq!(request.link_id, link_id);
    }
    let (m, body, _) = mapped(call(methods::CONTACT_REMOVE, on("principal-ana")), &known);
    assert_eq!(m, "contact.remove");
    let remove: LocalBusContactRemoveRequest = sdk_shaped(&body);
    assert_eq!((remove.link_id.as_str(), remove.scope.as_str()), ("l1", REMOVE_SCOPE));
    assert_eq!(REMOVE_SCOPE, "me", "the reference CLI's default scope");
}

#[test]
fn contact_effects_the_daemon_cannot_take_are_refused_before_sending() {
    let known = Fake::new();
    let invalid = |(code, message): (FailureCode, String)| {
        assert!(matches!(code, FailureCode::Daemon(c) if c == "invalid"), "{message}");
        message
    };
    let unknown = invalid(refused(call(methods::CONTACT_BLOCK, on("principal-zoe")), &known));
    assert!(unknown.contains("no link"));
    let not_pending = invalid(refused(call(methods::CONTACT_ACCEPT, on("principal-ana")), &known));
    assert!(not_pending.contains("no pending request"));
    let own = invalid(refused(call(methods::CONTACT_ADD, on(ME)), &known));
    assert!(own.contains("own id"));
    let anonymous = Fake { principal: None, ..Fake::new() };
    let no_identity = invalid(refused(call(methods::CONTACT_ADD, on("principal-kai")), &anonymous));
    assert!(no_identity.contains("not known yet"));
    invalid(refused(call(methods::CONTACT_ADD, json!({})), &known));

    // A contact request travels to the contact's device: no route is listed.
    let (code, message) = refused(call(methods::CONTACT_REQUEST, on("principal-kai")), &known);
    assert_eq!(code, FailureCode::UnsupportedByBackend);
    assert_eq!(message, "backend.gap.contact_route");
}

/// One row of the daemon's friend-link store, as `contact.list` lists it.
fn record(id: &str, requester: &str, target: &str, state: &str, blocked: bool) -> Value {
    json!({
        "link_id": id,
        "requester_id": requester,
        "target_id": target,
        "state": state,
        "remove_scope": null,
        "blocked": blocked,
        "capability_revoked_at": null,
    })
}

const NO_ROWS: &str = "storage error: sqlite error: Query returned no rows";
const REJECTED: &str = "storage error: authorization rejected";
const LOCKED: &str = "account database is not unlocked";

/// A fake daemon whose friend links change like the daemon's store does.
fn friend_daemon(links: Arc<Mutex<Vec<Value>>>) -> impl Fn(&str, Option<&str>, &Value) -> Reply {
    move |method, _, body| {
        let mut links = links.lock().expect("links");
        let id = body["link_id"].as_str().unwrap_or_default().to_owned();
        let position = links.iter().position(|l| l["link_id"] == id.as_str());
        let error = |message: &str| Reply::Error("ValidationFailed", message.to_owned());
        // A link the daemon lost meanwhile (its row is gone).
        if id == "l-ghost" && method != "contact.list" {
            return error(NO_ROWS);
        }
        let set = |links: &mut Vec<Value>, state: &str, blocked: bool| match position {
            Some(ix) => {
                links[ix]["state"] = json!(state);
                links[ix]["blocked"] = json!(blocked);
                Reply::Ok(links[ix].clone())
            }
            None => error(NO_ROWS),
        };
        match method {
            "contact.list" => Reply::Ok(json!({ "contacts": links.clone() })),
            "contact.add" => {
                let principal = body["principal_id"].as_str().unwrap_or_default();
                if body.as_object().is_none_or(|b| b.len() != 1) || principal.is_empty() {
                    return Reply::Refused {
                        code: "ValidationFailed",
                        message: "invalid_card".to_owned(),
                        reason_code: "invalid_card",
                        details: None,
                    };
                }
                if principal == "principal-locked" {
                    return error(LOCKED);
                }
                let contact = json!({
                    "principal_id": principal, "display_name": null, "reachable": true,
                    "verified": false, "kind_asserted": "human", "root_commitment": "root-1",
                    "root_source": "lookup", "node": null,
                });
                // A listed row is the link record joined with the contact.
                let mut row =
                    record(&format!("link-{principal}"), ME, principal, "accepted", false);
                if let (Value::Object(row), Value::Object(fields)) = (&mut row, contact.clone()) {
                    row.extend(fields);
                }
                links.push(row);
                Reply::Ok(json!({ "contact": contact }))
            }
            "contact.accept" => {
                let row = record(
                    &id,
                    body["requester_id"].as_str().unwrap_or_default(),
                    body["target_id"].as_str().unwrap_or_default(),
                    "accepted",
                    false,
                );
                match position {
                    Some(ix) => links[ix] = row.clone(),
                    None => links.push(row.clone()),
                }
                Reply::Ok(row)
            }
            "contact.reject" => match position {
                Some(ix) if links[ix]["state"] == "pending" => set(&mut links, "rejected", false),
                _ => error(REJECTED),
            },
            "contact.block" => set(&mut links, "blocked", true),
            "contact.unblock" => set(&mut links, "accepted", false),
            "contact.remove" => {
                if !matches!(body["scope"].as_str(), Some("me" | "own_devices" | "both")) {
                    return error(REJECTED);
                }
                set(&mut links, "removed", false)
            }
            other => fixture(other),
        }
    }
}

fn link_of(store: &Store, id: &str) -> Option<ContactLink> {
    store.participant(&ParticipantId::new(id)).map(|p| p.link_state())
}

#[test]
fn contact_actions_reach_the_daemon_and_its_records_update_the_contacts() {
    let path = socket_path("contacts");
    let links = Arc::new(Mutex::new(vec![
        record("l1", ME, "principal-ana", "accepted", false),
        record("l2", "principal-noor", ME, "pending", false),
        record("l-ghost", "principal-ghost", ME, "pending", false),
    ]));
    let fake = FakeBus::start(&path, friend_daemon(links));
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("contacts listed", |s, _| {
        s.is_listed(Listing::Contacts) && link_of(s, "principal-noor").is_some()
    });
    assert_eq!(link_of(&account.store, "principal-ana"), Some(ContactLink::Accepted));
    assert_eq!(link_of(&account.store, "principal-noor"), Some(ContactLink::Incoming));
    let noor = account.store.participant(&ParticipantId::new("principal-noor")).cloned();
    assert!(noor.is_some_and(|p| !p.contact), "an incoming request is not a contact yet");

    let frame = |method: &str| {
        fake.seen().into_iter().rev().find(|s| s.method == method).expect("frame").body
    };
    let submit = |fed: &mut Fed, method: &str, participant: &str| {
        fed.source.submit(call(method, on(participant)));
    };

    submit(&mut account, methods::CONTACT_ADD, "principal-kai");
    account.until("kai added", |s, _| {
        link_of(s, "principal-kai") == Some(ContactLink::Accepted)
            && s.participant(&ParticipantId::new("principal-kai")).is_some_and(|p| p.contact)
    });
    assert_eq!(frame("contact.add"), json!({ "principal_id": "principal-kai" }));
    let kai = account.store.participant(&ParticipantId::new("principal-kai")).cloned();
    let kai = kai.expect("kai");
    // A name lookup is provisional until a card pins it or numbers match.
    assert_eq!(
        (kai.reachable, kai.root_source, kai.kind, kai.kind_asserted),
        (Some(true), Some(RootSource::Lookup), ParticipantKind::Human, true)
    );

    submit(&mut account, methods::CONTACT_ACCEPT, "principal-noor");
    account
        .until("noor accepted", |s, _| link_of(s, "principal-noor") == Some(ContactLink::Accepted));
    let _: LocalBusContactAddRequest = sdk_shaped(&frame("contact.accept"));

    submit(&mut account, methods::CONTACT_BLOCK, "principal-ana");
    account.until("ana blocked", |s, _| link_of(s, "principal-ana") == Some(ContactLink::Blocked));
    assert!(
        account.store.participant(&ParticipantId::new("principal-ana")).is_some_and(|p| !p.contact)
    );
    submit(&mut account, methods::CONTACT_UNBLOCK, "principal-ana");
    account
        .until("ana unblocked", |s, _| link_of(s, "principal-ana") == Some(ContactLink::Accepted));
    submit(&mut account, methods::CONTACT_REMOVE, "principal-ana");
    account.until("ana removed", |s, _| link_of(s, "principal-ana") == Some(ContactLink::Removed));
    let remove: LocalBusContactRemoveRequest = sdk_shaped(&frame("contact.remove"));
    assert_eq!(remove.scope, "me");
    assert!(account.store.failures().is_empty(), "{:?}", account.store.failures());

    // Daemon refusals are failed effects with the daemon's code and reason.
    submit(&mut account, methods::CONTACT_REJECT, "principal-ana");
    account.until("reject refused", |s, _| s.failures().len() == 1);
    let failure = account.store.failures()[0].clone();
    assert_eq!(
        (failure.method.as_str(), failure.code, failure.message.as_str()),
        (methods::CONTACT_REJECT, FailureCode::Daemon("ValidationFailed".into()), REJECTED),
        "only a pending request can be rejected"
    );
    submit(&mut account, methods::CONTACT_ADD, "principal-locked");
    account.until("add refused", |s, _| s.failures().len() == 2);
    assert_eq!(account.store.failures()[1].method, methods::CONTACT_ADD);
    assert!(account.store.failures()[1].message.contains("not unlocked"));
    for (n, method) in [
        methods::CONTACT_ACCEPT,
        methods::CONTACT_REJECT,
        methods::CONTACT_BLOCK,
        methods::CONTACT_UNBLOCK,
        methods::CONTACT_REMOVE,
    ]
    .into_iter()
    .enumerate()
    {
        submit(&mut account, method, "principal-ghost");
        account.until("ghost refused", |s, _| s.failures().len() == 3 + n);
        let failure = account.store.failures()[2 + n].clone();
        assert_eq!(failure.method, method);
        assert_eq!(failure.code, FailureCode::Daemon("ValidationFailed".into()));
        assert_eq!(failure.message, NO_ROWS);
    }
    assert_eq!(link_of(&account.store, "principal-ghost"), Some(ContactLink::Incoming));

    // A contact request never reaches the daemon.
    let before = fake.seen().len();
    submit(&mut account, methods::CONTACT_REQUEST, "principal-kai");
    account.until("request refused", |s, _| s.failures().len() == 8);
    assert_eq!(account.store.failures()[7].code, FailureCode::UnsupportedByBackend);
    assert!(fake.seen()[before..].iter().all(|s| s.method != "contact.request"));
    fake.shutdown();
}

#[test]
fn an_incoming_request_recorded_later_appears_on_the_next_poll() {
    let path = socket_path("incoming");
    let links = Arc::new(Mutex::new(vec![record("l1", ME, "principal-ana", "accepted", false)]));
    let fake = FakeBus::start(&path, friend_daemon(links.clone()));
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("listed", |s, _| link_of(s, "principal-ana").is_some());
    // The daemon records a request it received (a `friend.requested` event
    // applied by a receive), without pushing anything.
    links.lock().expect("links").push(record("l9", "principal-sam", ME, "pending", false));
    account
        .until("request polled", |s, _| link_of(s, "principal-sam") == Some(ContactLink::Incoming));
    fake.shutdown();
}

/// DESK-F17: a contact added by id (a name lookup) carries no name and no
/// kind: the daemon answers `display_name: null` and `kind_asserted: null`
/// (only a contact card supplies either, and the identity directory has no
/// remote kind). It is shown by its short id, its kind stays unknown and a
/// task is not addressed to it on a guess. A principal whose card was
/// pinned earlier is answered with the card's name and asserted kind.
#[test]
fn a_contact_added_by_id_is_shown_by_its_short_id_and_its_kind_stays_unknown() {
    const ORBIT: &str = "principal-c786a534839ad12fef6dd00033a374ef";
    const CARDED: &str = "principal-0a1b2c3d4e5f60718293a4b5c6d7e8f9";
    let path = socket_path("contacts-unnamed");
    let links: Arc<Mutex<Vec<Value>>> = Arc::default();
    let fake = FakeBus::start(&path, move |method, _, body| {
        let mut links = links.lock().expect("links");
        match method {
            "contact.list" => Reply::Ok(json!({ "contacts": links.clone() })),
            "contact.add" => {
                let principal = body["principal_id"].as_str().unwrap_or_default();
                let (name, kind) = if principal == CARDED {
                    (json!("Ledger"), json!("headless_ai"))
                } else {
                    (Value::Null, Value::Null)
                };
                let contact = json!({
                    "principal_id": principal, "display_name": name, "reachable": true,
                    "verified": false, "kind_asserted": kind, "root_commitment": "root-1",
                    "root_source": "lookup", "node": null,
                });
                let mut row =
                    record(&format!("link-{principal}"), ME, principal, "accepted", false);
                if let (Value::Object(row), Value::Object(fields)) = (&mut row, contact.clone()) {
                    row.extend(fields);
                }
                links.push(row);
                Reply::Ok(json!({ "contact": contact }))
            }
            other => fixture(other),
        }
    });
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("contacts listed", |s, _| s.is_listed(Listing::Contacts));
    for principal in [ORBIT, CARDED] {
        account.source.submit(call(methods::CONTACT_ADD, on(principal)));
    }
    let shown = |s: &Store, id: &str| s.participant(&ParticipantId::new(id)).cloned();
    account.until("both added", |s, _| {
        [ORBIT, CARDED].iter().all(|id| shown(s, id).is_some_and(|p| p.contact))
    });
    let orbit = shown(&account.store, ORBIT).expect("orbit");
    assert_eq!(orbit.display, "principal-c786a534\u{2026}", "the short id, not the whole id");
    assert!(!orbit.named());
    assert_eq!(orbit.kind, ParticipantKind::Unknown, "a lookup says nothing about the kind");
    let carded = shown(&account.store, CARDED).expect("carded");
    assert_eq!(carded.display, "Ledger", "the name the card gave");
    assert!(carded.named());
    assert_eq!((carded.kind, carded.kind_asserted), (ParticipantKind::Agent, true));
    fake.shutdown();
}
