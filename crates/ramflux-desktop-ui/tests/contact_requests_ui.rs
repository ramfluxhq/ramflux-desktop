// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI integration tests of contact requests in the contacts dock, driving
//! the production window on the fake daemon with the CONTACT-02 request
//! API (contract-first): a pending request with its note, held messages and
//! safety number; Accept, Decline and Block (through the hold) reaching the
//! daemon by request id; the auto-accept setting; this account's own
//! request on a contact (Send again while it waits, a decline shown as
//! such, a re-request during the decline's cooldown explained in the add
//! form); and a daemon without requests.

mod common;
mod sdk_fake;

use std::sync::Arc;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use ramflux_desktop_core::{RequestPolicy, SyncMode};
use ramflux_desktop_test::fake_contact_requests::FakeContactRequests;
use ramflux_desktop_ui::i18n::tr_args;
use ramflux_desktop_ui::route::Page;
use ramflux_desktop_ui::workspace::docks::contacts::add_form::FormNote;
use sdk_fake::wait;
use serde_json::json;

const ACCOUNT: &str = "acct-main";

struct World {
    fake: Arc<ramflux_desktop_test::fake_bus::FakeBus>,
    requests: Arc<FakeContactRequests>,
    handle: gpui_kit::WindowHandle<gpui_kit::base::Root>,
    shell: gpui_kit::Entity<ramflux_desktop_ui::Shell>,
}

fn open_requests(cx: &mut TestAppContext, name: &str, available: bool) -> World {
    let (fake, state, path) = sdk_fake::start(name);
    fake.enable_stream();
    let fake = Arc::new(fake);
    let requests = state.lock().expect("state").requests.clone();
    requests.set_available(available);
    let bus = Arc::downgrade(&fake);
    requests.set_publisher(Box::new(move |account, topic, body| {
        if let Some(bus) = bus.upgrade() {
            bus.publish(account, topic, body);
        }
    }));
    let (handle, shell) = open_sized(cx, sdk_fake::options(&path), 1_100.);
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the stream", |cx| {
        with_store(cx, &shell, |s| s.sync_status().is_some_and(|x| x.mode == SyncMode::Stream))
    });
    click(cx, handle, "rail-contacts");
    assert!(exists(cx, handle, "dock-content-contacts"));
    World { fake, requests, handle, shell }
}

fn row(peer: &str) -> String {
    format!("contacts-request-{peer}")
}

fn label_in(
    cx: &mut TestAppContext,
    world: &World,
    scope: &str,
    id: &'static str,
) -> Option<String> {
    let scope = scope.to_owned();
    in_window(cx, world.handle, move |window, _| {
        window.within(scope).try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
}

fn sent(world: &World, method: &str) -> Vec<serde_json::Value> {
    world.fake.seen().into_iter().filter(|s| s.method == method).map(|s| s.body).collect()
}

#[gpui_kit::test]
fn a_request_shows_its_note_held_messages_and_safety_number_and_is_accepted(
    cx: &mut TestAppContext,
) {
    let world = open_requests(cx, "ui-request-accept", true);
    let id = world.requests.incoming(ACCOUNT, "principal-new", Some("we met at the meetup"));
    world.requests.hold(&id, "hi, it's me from the meetup");
    // The event carries the count; the preview comes with the list read.
    wait(cx, "the request row", |cx| {
        exists(cx, world.handle, &row("principal-new"))
            && label_in(cx, &world, &row("principal-new"), "request-held")
                .is_some_and(|held| held.contains("hi, it's me from the meetup"))
    });
    assert!(exists_in(cx, world.handle, &row("principal-new"), "request-note"));
    assert!(exists_in(cx, world.handle, &row("principal-new"), "request-safety"));
    let held = label_in(cx, &world, &row("principal-new"), "request-held").unwrap_or_default();
    assert!(held.contains("hi, it's me from the meetup"), "{held}");
    assert!(exists(cx, world.handle, "contacts-auto-accept"));

    click_in(cx, world.handle, &row("principal-new"), "request-accept");
    wait(cx, "accepted by the daemon", |_| {
        world.requests.row(&id).is_some_and(|r| r["state"] == "accepted")
    });
    assert_eq!(sent(&world, "contact.accept"), [json!({ "request_id": id })]);
    wait(cx, "the row gone", |cx| !exists(cx, world.handle, &row("principal-new")));
    // The held message was delivered into the conversation.
    assert_eq!(world.requests.delivered().len(), 1);
}

#[gpui_kit::test]
fn decline_and_block_through_the_hold_reach_the_daemon(cx: &mut TestAppContext) {
    let world = open_requests(cx, "ui-request-decline", true);
    let declined = world.requests.incoming(ACCOUNT, "principal-old", None);
    let blocked = world.requests.incoming(ACCOUNT, "principal-spam", None);
    wait(cx, "both rows", |cx| {
        exists(cx, world.handle, &row("principal-old"))
            && exists(cx, world.handle, &row("principal-spam"))
    });
    click_in(cx, world.handle, &row("principal-old"), "request-decline");
    wait(cx, "declined", |_| {
        world.requests.row(&declined).is_some_and(|r| r["state"] == "declined")
    });

    click_in(cx, world.handle, &row("principal-spam"), "request-block");
    advance(cx, 100);
    assert!(sent(&world, "contact.block").is_empty(), "nothing before the hold");
    hold_confirm(cx, world.handle);
    wait(cx, "blocked", |_| world.requests.row(&blocked).is_some_and(|r| r["state"] == "blocked"));
    assert_eq!(sent(&world, "contact.block"), [json!({ "request_id": blocked })]);
}

#[gpui_kit::test]
fn the_auto_accept_setting_is_switched(cx: &mut TestAppContext) {
    let world = open_requests(cx, "ui-request-policy", true);
    wait(cx, "the setting", |cx| {
        with_store(cx, &world.shell, |s| s.request_policy() == Some(RequestPolicy::Manual))
    });
    click(cx, world.handle, "contacts-auto-accept");
    wait(cx, "auto accept", |cx| {
        with_store(cx, &world.shell, |s| s.request_policy() == Some(RequestPolicy::AutoAccept))
    });
    assert_eq!(world.requests.policy(ACCOUNT), "auto_accept");
    // A request now never waits in the dock.
    world.requests.incoming(ACCOUNT, "principal-new", None);
    advance(cx, 300);
    assert!(!exists(cx, world.handle, &row("principal-new")));
}

#[gpui_kit::test]
fn a_contact_says_this_accounts_request_waits_for_them(cx: &mut TestAppContext) {
    let world = open_requests(cx, "ui-request-outgoing", true);
    let id = world.requests.outgoing(ACCOUNT, "principal-ana");
    world.fake.publish(ACCOUNT, "contact.changed", json!({}));
    wait(cx, "the waiting note", |cx| {
        exists(cx, world.handle, "contacts-principal-ana")
            && label_in(cx, &world, "contacts-principal-ana", "contact-request").is_some()
    });
    let note = label_in(cx, &world, "contacts-principal-ana", "contact-request").unwrap();
    let ana = ramflux_desktop_core::ParticipantId::new("principal-ana");
    let name = with_store(cx, &world.shell, move |s| s.participant(&ana).unwrap().display.clone());
    assert_eq!(note, tr_args("dock.contacts.request.waiting", &[("name", &name)]).as_ref());
    world.requests.peer_answers(&id, "accepted");
    wait(cx, "the note gone", |cx| {
        label_in(cx, &world, "contacts-principal-ana", "contact-request").is_none()
    });
}

#[gpui_kit::test]
fn a_waiting_request_is_sent_again_on_request_and_a_decline_cools_down(cx: &mut TestAppContext) {
    let world = open_requests(cx, "ui-request-again", true);
    let id = world.requests.outgoing(ACCOUNT, "principal-ana");
    world.fake.publish(ACCOUNT, "contact.changed", json!({}));
    wait(cx, "Send again", |cx| {
        exists_in(cx, world.handle, "contacts-principal-ana", "contact-request-again")
    });
    click_in(cx, world.handle, "contacts-principal-ana", "contact-request-again");
    wait(cx, "sent again", |_| world.requests.sent(&id) == 2);
    assert_eq!(sent(&world, "contact.add"), [json!({ "principal_id": "principal-ana" })]);

    // Ana declines: the row says so quietly, and offers nothing to resend.
    world.requests.peer_answers(&id, "declined");
    let ana = ramflux_desktop_core::ParticipantId::new("principal-ana");
    let name = with_store(cx, &world.shell, move |s| s.participant(&ana).unwrap().display.clone());
    let declined = tr_args("dock.contacts.request.declined", &[("name", &name)]).to_string();
    wait(cx, "the decline", |cx| {
        label_in(cx, &world, "contacts-principal-ana", "contact-request").as_deref()
            == Some(declined.as_str())
    });
    assert!(!exists_in(cx, world.handle, "contacts-principal-ana", "contact-request-again"));
    advance(cx, 300);
    assert_eq!(sent(&world, "contact.add").len(), 1, "no automatic retry");

    // Asking again in the add form during the cooldown: a plain answer.
    type_into(cx, world.handle, "contact-add-id", "principal-ana");
    click(cx, world.handle, "contact-add");
    let note = |cx: &mut TestAppContext| {
        let workspace = workspace(cx, &world.shell);
        cx.update(|cx| workspace.read(cx).contacts_dock().read(cx).add_note().cloned())
    };
    wait(cx, "the cooldown", |cx| matches!(note(cx), Some(FormNote::Error(_))));
    let Some(FormNote::Error(reason)) = note(cx) else { unreachable!() };
    let cooldown = tr_args("backend.contact.request_cooldown", &[("name", &name)]);
    assert!(reason.contains(cooldown.as_ref()), "{reason}");
}

#[gpui_kit::test]
fn without_requests_on_the_daemon_nothing_is_offered(cx: &mut TestAppContext) {
    let world = open_requests(cx, "ui-request-none", false);
    advance(cx, 300);
    assert!(!exists(cx, world.handle, "contacts-auto-accept"));
    assert!(with_store(cx, &world.shell, |s| s.request_policy().is_none()));
}
