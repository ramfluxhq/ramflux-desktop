// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Starting a direct conversation in the production window on a fake
//! daemon: Message on a contact (or the context stack's "+", or the
//! palette) opens a draft; the first message from the composer starts the
//! conversation, which then shows. A contact whose key changed is refused
//! before anything is typed; a missing route or an unverified key comes
//! back as the reason in the draft, with the text kept for another try. A
//! conversation someone else starts appears once the daemon lists it.

mod common;
mod sdk_fake;

use common::*;
use gpui_kit::AppContext as _;
use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{MouseButton, TestAppContext, WindowHandle};
use ramflux_desktop_core::{ContextId, ContextKind, ParticipantId};
use ramflux_desktop_ui::i18n::{tr, tr_args};
use ramflux_desktop_ui::route::Page;
use sdk_fake::wait;
use serde_json::json;

type Handle = WindowHandle<gpui_kit::base::Root>;
type ShellEntity = gpui_kit::Entity<ramflux_desktop_ui::Shell>;

fn open_workspace(
    cx: &mut TestAppContext,
    name: &str,
) -> (Handle, ShellEntity, sdk_fake::Shared, ramflux_desktop_test::fake_bus::FakeBus) {
    let (fake, state, path) = sdk_fake::start(name);
    let (handle, shell) = open_with(cx, sdk_fake::options(&path));
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "contacts and identities", |cx| {
        with_store(cx, &shell, |s| {
            s.participant(&ParticipantId::new("principal-zed")).is_some()
                && s.participant(&ParticipantId::new("principal-dee")).is_some()
                && s.context(&ContextId::new("dm-ana")).is_some()
        })
    });
    (handle, shell, state, fake)
}

fn message_from_contacts(cx: &mut TestAppContext, handle: Handle, who: &str) {
    if !exists(cx, handle, &format!("contacts-{who}")) {
        click(cx, handle, "rail-contacts");
    }
    click_in(cx, handle, &format!("contacts-{who}"), "message");
}

fn draft_open(cx: &mut TestAppContext, shell: &ShellEntity) -> bool {
    let workspace = workspace(cx, shell);
    cx.update(|cx| workspace.read(cx).new_conversation().read(cx).is_open())
}

fn current(cx: &mut TestAppContext, shell: &ShellEntity) -> Option<ContextId> {
    let workspace = workspace(cx, shell);
    cx.update(|cx| workspace.read(cx).current_context().cloned())
}

fn label(cx: &mut TestAppContext, handle: Handle, id: &str) -> Option<String> {
    let id = id.to_owned();
    in_window(cx, handle, move |window, _| {
        window.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
}

fn composer_text(cx: &mut TestAppContext, shell: &ShellEntity) -> String {
    let workspace = workspace(cx, shell);
    cx.update(|cx| workspace.read(cx).composer().read(cx).input().read(cx).value().to_string())
}

fn submits(fake: &ramflux_desktop_test::fake_bus::FakeBus) -> Vec<serde_json::Value> {
    fake.seen().into_iter().filter(|s| s.method == "message.submit").map(|s| s.body).collect()
}

#[gpui_kit::test]
fn the_first_message_starts_the_conversation_and_shows_it(cx: &mut TestAppContext) {
    let (handle, shell, _, fake) = open_workspace(cx, "ui-start-ok");
    message_from_contacts(cx, handle, "principal-bo");
    assert!(draft_open(cx, &shell), "Message opens a draft");
    assert!(exists(cx, handle, "new-conversation"), "the draft replaces the conversation field");
    assert_eq!(current(cx, &shell), None, "no context is shown while drafting");
    assert!(!exists(cx, handle, "composer-unavailable"), "a verified contact can be written to");
    type_into(cx, handle, "composer-input", "Hello Bo");
    click(cx, handle, "composer-send");
    wait(cx, "the started conversation", |cx| {
        !draft_open(cx, &shell) && current(cx, &shell).is_some()
    });
    let id = current(cx, &shell).expect("shown");
    let sent = submits(&fake);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["conversation_id"], id.as_str(), "the draft's id is the conversation's");
    assert_eq!(sent[0]["recipient_device_id"], "device-bo");
    assert_eq!(sent[0]["target_delivery_id"], "target-bo");
    assert_eq!(sent[0]["recipient_principal_commitment"], "cm-bo");
    assert_eq!(id.as_str().len(), 36, "a UUID: {id}");
    let context = with_store(cx, &shell, |s| s.context(&id).cloned()).expect("context");
    assert_eq!(context.kind, ContextKind::Direct);
    assert!(context.participants.contains(&ParticipantId::new("principal-bo")));
    assert_eq!(composer_text(cx, &shell), "", "the composer is cleared once started");
    wait(cx, "the first message", |cx| {
        with_store(cx, &shell, |s| s.context_moment_count(&id) == 1)
    });
    // Message on the same contact now shows the existing conversation.
    show_context(cx, handle, &shell, "dm-ana");
    message_from_contacts(cx, handle, "principal-bo");
    assert!(!draft_open(cx, &shell));
    assert_eq!(current(cx, &shell), Some(id));
    fake.shutdown();
}

#[gpui_kit::test]
fn a_contact_with_a_changed_key_is_refused_before_typing(cx: &mut TestAppContext) {
    let (handle, shell, _, fake) = open_workspace(cx, "ui-start-key");
    message_from_contacts(cx, handle, "principal-cy");
    assert!(draft_open(cx, &shell));
    let reason = tr_args("conversation.new.error.key_changed", &[("name", "principal-cy")]);
    assert_eq!(label(cx, handle, "new-conversation-status").as_deref(), Some(reason.as_ref()));
    assert!(exists(cx, handle, "composer-unavailable"), "the composer says why too");
    type_into(cx, handle, "composer-input", "hi");
    click(cx, handle, "composer-send");
    advance(cx, 300);
    assert!(submits(&fake).is_empty(), "nothing is sent");
    assert!(draft_open(cx, &shell));
    fake.shutdown();
}

#[gpui_kit::test]
fn a_missing_route_or_unverified_key_comes_back_in_the_draft(cx: &mut TestAppContext) {
    let (handle, shell, _, fake) = open_workspace(cx, "ui-start-route");
    for (who, name, key) in [
        ("principal-zed", "principal-zed", "backend.failure.start_no_identity"),
        ("principal-dee", "Dee", "backend.failure.start_unverified_key"),
    ] {
        message_from_contacts(cx, handle, who);
        assert!(draft_open(cx, &shell));
        in_window(cx, handle, |window, cx| {
            window.click("composer-input", cx);
            window.press("secondary-a", cx);
            window.press("backspace", cx);
        });
        type_into(cx, handle, "composer-input", "first words");
        click(cx, handle, "composer-send");
        let expected =
            tr_args("conversation.new.failed", &[("reason", &tr_args(key, &[("name", name)]))]);
        wait(cx, "the reason", |cx| {
            label(cx, handle, "new-conversation-status").as_deref() == Some(expected.as_ref())
        });
        assert!(draft_open(cx, &shell), "the draft stays open");
        assert_eq!(composer_text(cx, &shell), "first words", "the text is kept for another try");
        assert!(!exists(cx, handle, "composer-unavailable"), "it can be sent again");
    }
    assert!(submits(&fake).is_empty(), "no route was guessed");
    fake.shutdown();
}

#[gpui_kit::test]
fn the_stack_plus_and_the_palette_open_the_draft(cx: &mut TestAppContext) {
    let (handle, shell, _, fake) = open_workspace(cx, "ui-start-entry");
    click(cx, handle, "stack-new");
    let item = tr_args("stack.new_with", &[("name", "principal-bo")]).to_string();
    in_window(cx, handle, move |window, cx| {
        let at = window.find_by_label(&item).bounds().center();
        window.pointer_down(at, MouseButton::Left, cx);
        window.pointer_up(at, MouseButton::Left, cx);
    });
    assert!(draft_open(cx, &shell), "the stack's + opened a draft with Bo");
    let target = {
        let workspace = workspace(cx, &shell);
        cx.update(|cx| workspace.read(cx).new_conversation().read(cx).draft().cloned())
    };
    assert_eq!(target.map(|d| d.participant), Some(ParticipantId::new("principal-bo")));
    // The palette's contact row does the same (and shows Ana's existing
    // conversation instead of a draft).
    let workspace = workspace(cx, &shell);
    cx.update_window(handle.into(), |_, window, cx| {
        workspace.update(cx, |w, cx| {
            w.open_conversation_with(ParticipantId::new("principal-ana"), window, cx);
        });
    })
    .expect("window");
    cx.run_until_parked();
    assert!(!draft_open(cx, &shell));
    assert_eq!(current(cx, &shell), Some(ContextId::new("dm-ana")));
    assert!(!tr("stack.new").is_empty());
    fake.shutdown();
}

#[gpui_kit::test]
fn a_conversation_the_daemon_lists_appears_without_a_restart(cx: &mut TestAppContext) {
    let (handle, shell, state, fake) = open_workspace(cx, "ui-start-k1");
    let new = ContextId::new("dm-from-zoe");
    assert!(with_store(cx, &shell, |s| s.context(&new).is_none()));
    state.lock().expect("state").conversations.push(json!({
        "id": "dm-from-zoe", "archived": false, "hidden": false, "unread_count": 1,
        "is_unread": true, "muted": false, "disappearing": null, "kind": "direct",
        "title": null, "participants": ["principal-me", "principal-zoe"], "group_id": null,
        "direct_route": null, "self_principal_id": "principal-me",
    }));
    fake.push("acct-main", "conversation.changed", json!({ "conversation_id": "dm-from-zoe" }));
    wait(cx, "the listed conversation", |cx| with_store(cx, &shell, |s| s.context(&new).is_some()));
    show_context(cx, handle, &shell, "dm-from-zoe");
    assert_eq!(current(cx, &shell), Some(new));
    fake.shutdown();
}
