// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI integration tests of the contacts dock: the add-contact form (checks,
//! the daemon's answer, its failure), incoming requests, the contact menu
//! (block, remove through the hold), blocked contacts, and the palette entry.

mod common;

use common::*;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{MouseButton, TestAppContext, WindowHandle};
use ramflux_desktop_core::{ContactLink, ParticipantId, methods};
use ramflux_desktop_ui::actions::OpenPalette;
use ramflux_desktop_ui::i18n::tr;
use ramflux_desktop_ui::workspace::docks::contacts::add_form::FormNote;

type Handle = WindowHandle<gpui_kit::base::Root>;

fn open_contacts(cx: &mut TestAppContext) -> (Handle, gpui_kit::Entity<ramflux_desktop_ui::Shell>) {
    let (handle, shell) = open(cx, "s04_safety");
    advance(cx, 1_000);
    click(cx, handle, "rail-contacts");
    assert!(exists(cx, handle, "dock-content-contacts"));
    (handle, shell)
}

fn note(
    cx: &mut TestAppContext,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::Shell>,
) -> Option<FormNote> {
    let workspace = workspace(cx, shell);
    cx.update(|cx| workspace.read(cx).contacts_dock().read(cx).add_note().cloned())
}

fn field(cx: &mut TestAppContext, shell: &gpui_kit::Entity<ramflux_desktop_ui::Shell>) -> String {
    let workspace = workspace(cx, shell);
    cx.update(|cx| workspace.read(cx).contacts_dock().read(cx).add_value(cx))
}

fn link(
    cx: &mut TestAppContext,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::Shell>,
    id: &str,
) -> Option<ContactLink> {
    let id = ParticipantId::new(id);
    with_store(cx, shell, move |s| s.participant(&id).map(|p| p.link_state()))
}

fn replace_field(cx: &mut TestAppContext, handle: Handle, text: &str) {
    in_window(cx, handle, |window, cx| {
        window.click("contact-add-id", cx);
        window.press("secondary-a", cx);
        window.press("backspace", cx);
    });
    type_into(cx, handle, "contact-add-id", text);
}

fn click_label(cx: &mut TestAppContext, handle: Handle, label: &str) {
    let label = label.to_owned();
    in_window(cx, handle, move |window, cx| {
        let at = window.find_by_label(&label).bounds().center();
        gpui_kit::test::TestEventExt::pointer_down(window, at, MouseButton::Left, cx);
        gpui_kit::test::TestEventExt::pointer_up(window, at, MouseButton::Left, cx);
    });
}

#[gpui_kit::test]
fn the_form_checks_the_id_before_anything_is_sent(cx: &mut TestAppContext) {
    let (handle, shell) = open_contacts(cx);
    click(cx, handle, "contact-add");
    assert_eq!(note(cx, &shell), Some(FormNote::Error(tr("dock.contacts.add.error.empty"))));
    type_into(cx, handle, "contact-add-id", "two words");
    click(cx, handle, "contact-add");
    assert_eq!(note(cx, &shell), Some(FormNote::Error(tr("dock.contacts.add.error.spaces"))));
    assert!(exists(cx, handle, "contact-add-note"), "the reason is shown next to the field");
    replace_field(cx, handle, "p-mira");
    assert_eq!(note(cx, &shell), None, "editing the field clears what was said");
    click(cx, handle, "contact-add");
    assert_eq!(note(cx, &shell), Some(FormNote::Error(tr("dock.contacts.add.error.exists"))));
    replace_field(cx, handle, "p-ivo");
    click(cx, handle, "contact-add");
    assert_eq!(note(cx, &shell), Some(FormNote::Error(tr("dock.contacts.add.error.blocked"))));
    replace_field(cx, handle, "p-sam");
    click(cx, handle, "contact-add");
    assert_eq!(note(cx, &shell), Some(FormNote::Error(tr("dock.contacts.add.error.incoming"))));
    assert!(
        !effects(cx, &shell).contains(&methods::CONTACT_ADD.to_owned()),
        "nothing reaches the source"
    );
}

#[gpui_kit::test]
fn adding_a_contact_sends_contact_add_and_the_contact_appears(cx: &mut TestAppContext) {
    let (handle, shell) = open_contacts(cx);
    type_into(cx, handle, "contact-add-id", "  principal-kai ");
    in_window(cx, handle, |window, cx| window.press("enter", cx));
    let body = last_body(cx, &shell, methods::CONTACT_ADD);
    assert_eq!(body["participant_id"], "principal-kai", "trimmed");
    assert_eq!(body["proposed_by"], "Human");
    assert!(body["confirmed_by"].is_null(), "adding needs no hold");
    assert!(!strip_open(cx, &shell));
    assert_eq!(note(cx, &shell), Some(FormNote::Adding(ParticipantId::new("principal-kai"))));
    assert_eq!(field(cx, &shell), "  principal-kai ", "the input is kept while waiting");
    advance(cx, 200);
    assert_eq!(link(cx, &shell, "principal-kai"), Some(ContactLink::Accepted));
    assert!(exists(cx, handle, "contacts-principal-kai"), "listed as a contact");
    assert_eq!(field(cx, &shell), "", "cleared once added");
    assert_eq!(note(cx, &shell), None);
    let store = store_handle(cx, &shell);
    let path = cx.update(|cx| {
        store.read(cx).inspector().root_intents().find_map(|entry| match entry {
            ramflux_desktop_ui::app::InspectorEntry::UiEvent {
                path,
                kind: "AddContactRequested",
                ..
            } => Some(path.clone()),
            _ => None,
        })
    });
    assert_eq!(path.as_deref(), Some("Workspace > Panel(dock.contacts)"));
}

#[gpui_kit::test]
fn a_failed_add_shows_the_daemon_reason_under_the_field(cx: &mut TestAppContext) {
    let (handle, shell) = open_contacts(cx);
    type_into(cx, handle, "contact-add-id", "principal-locked");
    click(cx, handle, "contact-add");
    assert_eq!(last_body(cx, &shell, methods::CONTACT_ADD)["participant_id"], "principal-locked");
    advance(cx, 200);
    let Some(FormNote::Error(text)) = note(cx, &shell) else {
        panic!("an error under the field, got {:?}", note(cx, &shell))
    };
    assert!(text.contains("account database is not unlocked"), "{text}");
    assert_eq!(field(cx, &shell), "principal-locked", "the input is kept to retry");
    assert_eq!(link(cx, &shell, "principal-locked"), None);
}

#[gpui_kit::test]
fn an_incoming_request_is_accepted_from_the_dock(cx: &mut TestAppContext) {
    let (handle, shell) = open_contacts(cx);
    assert!(exists(cx, handle, "contacts-incoming-p-sam"));
    click_in(cx, handle, "contacts-incoming-p-sam", "accept");
    let body = last_body(cx, &shell, methods::CONTACT_ACCEPT);
    assert_eq!(body["participant_id"], "p-sam");
    assert!(body["confirmed_by"].is_null());
    advance(cx, 200);
    assert_eq!(link(cx, &shell, "p-sam"), Some(ContactLink::Accepted));
    assert!(!exists(cx, handle, "contacts-incoming-p-sam"));
    assert!(exists(cx, handle, "contacts-p-sam"));
}

#[gpui_kit::test]
fn an_incoming_request_is_rejected_from_the_dock(cx: &mut TestAppContext) {
    let (handle, shell) = open_contacts(cx);
    click_in(cx, handle, "contacts-incoming-p-sam", "reject");
    assert_eq!(last_body(cx, &shell, methods::CONTACT_REJECT)["participant_id"], "p-sam");
    advance(cx, 200);
    assert_eq!(link(cx, &shell, "p-sam"), Some(ContactLink::Rejected));
    assert!(!exists(cx, handle, "contacts-incoming-p-sam"));
    assert!(!exists(cx, handle, "contacts-p-sam"), "a rejected request is not a contact");
}

#[gpui_kit::test]
fn blocking_from_the_contact_menu_needs_the_hold(cx: &mut TestAppContext) {
    let (handle, shell) = open_contacts(cx);
    click_in(cx, handle, "contacts-p-tomas", "contact-actions");
    click_label(cx, handle, &tr("dock.contacts.block"));
    assert!(strip_open(cx, &shell), "blocking needs the hold");
    assert!(!effects(cx, &shell).contains(&methods::CONTACT_BLOCK.to_owned()));
    hold_confirm(cx, handle);
    let body = last_body(cx, &shell, methods::CONTACT_BLOCK);
    assert_eq!(body["participant_id"], "p-tomas");
    assert_eq!(body["confirmed_by"]["gesture"], "Hold");
    advance(cx, 200);
    assert_eq!(link(cx, &shell, "p-tomas"), Some(ContactLink::Blocked));
    assert!(exists(cx, handle, "contacts-blocked-p-tomas"), "listed under Blocked");
    assert!(!exists(cx, handle, "contacts-p-tomas"));
    let audited = with_store(cx, &shell, |s| {
        s.audit().iter().any(|a| a.action == methods::CONTACT_BLOCK && a.subject == "p-tomas")
    });
    assert!(audited, "the confirmed block is audited");
}

#[gpui_kit::test]
fn a_blocked_contact_is_unblocked_or_removed(cx: &mut TestAppContext) {
    let (handle, shell) = open_contacts(cx);
    assert!(exists(cx, handle, "contacts-blocked-p-ivo"));
    click_in(cx, handle, "contacts-blocked-p-ivo", "remove");
    assert!(strip_open(cx, &shell), "removing needs the hold");
    hold_confirm(cx, handle);
    let body = last_body(cx, &shell, methods::CONTACT_REMOVE);
    assert_eq!(body["participant_id"], "p-ivo");
    assert_eq!(body["confirmed_by"]["gesture"], "Hold");
    advance(cx, 200);
    assert_eq!(link(cx, &shell, "p-ivo"), Some(ContactLink::Removed));
    assert!(!exists(cx, handle, "contacts-blocked-p-ivo"));
}

#[gpui_kit::test]
fn unblocking_needs_no_hold(cx: &mut TestAppContext) {
    let (handle, shell) = open_contacts(cx);
    click_in(cx, handle, "contacts-blocked-p-ivo", "unblock");
    assert!(!strip_open(cx, &shell));
    let body = last_body(cx, &shell, methods::CONTACT_UNBLOCK);
    assert_eq!(body["participant_id"], "p-ivo");
    advance(cx, 200);
    assert_eq!(link(cx, &shell, "p-ivo"), Some(ContactLink::Accepted));
    assert!(exists(cx, handle, "contacts-p-ivo"));
    // Removing an accepted contact goes through its menu and the hold.
    click_in(cx, handle, "contacts-p-ivo", "contact-actions");
    click_label(cx, handle, &tr("dock.contacts.remove"));
    assert!(strip_open(cx, &shell));
    hold_confirm(cx, handle);
    assert_eq!(last_body(cx, &shell, methods::CONTACT_REMOVE)["participant_id"], "p-ivo");
}

#[gpui_kit::test]
fn the_palette_opens_the_add_contact_field(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s04_safety");
    advance(cx, 1_000);
    in_window(cx, handle, |window, cx| window.dispatch_action(Box::new(OpenPalette), cx));
    let query = tr("palette.add_contact").to_string();
    in_window(cx, handle, move |window, cx| window.input(&query, cx));
    in_window(cx, handle, |window, cx| window.press("enter", cx));
    assert!(!cx.update(|cx| shell.read(cx).is_palette_open()));
    assert!(exists(cx, handle, "dock-content-contacts"), "the contacts dock is open");
    let workspace = workspace(cx, &shell);
    let focused = in_window(cx, handle, move |window, cx| {
        workspace.read(cx).contacts_dock().read(cx).add_focused(window, cx)
    });
    assert!(focused, "the add-contact field has focus");
    in_window(cx, handle, |window, cx| window.input("principal-kai", cx));
    in_window(cx, handle, |window, cx| window.press("enter", cx));
    assert_eq!(last_body(cx, &shell, methods::CONTACT_ADD)["participant_id"], "principal-kai");
    assert!(effects(cx, &shell).iter().all(|m| m != methods::CONTEXT_PIN), "only the add");
}

#[gpui_kit::test]
fn a_contact_can_be_unblocked_and_blocked_again(cx: &mut TestAppContext) {
    // CONTACT-BLOCK-01: unblock, block again and unblock again are three
    // decisions the daemon takes in turn. The second block needs the hold
    // like the first, and each row reads as what it is.
    let (handle, shell) = open_contacts(cx);
    let blocked_row = "contacts-blocked-p-ivo";
    for round in 1..=2 {
        let label = in_window(cx, handle, |window, _| {
            window.within(blocked_row).find("unblock").label().map(str::to_owned)
        });
        assert_eq!(label.as_deref(), Some(tr("dock.contacts.unblock").as_ref()), "round {round}");
        click_in(cx, handle, blocked_row, "unblock");
        advance(cx, 200);
        assert_eq!(link(cx, &shell, "p-ivo"), Some(ContactLink::Accepted), "unblock {round}");
        assert!(exists(cx, handle, "contacts-p-ivo"), "back among the contacts");
        assert!(!exists(cx, handle, blocked_row));

        if round == 1 {
            click_in(cx, handle, "contacts-p-ivo", "contact-actions");
            click_label(cx, handle, &tr("dock.contacts.block"));
            assert!(strip_open(cx, &shell), "blocking again needs the hold");
            hold_confirm(cx, handle);
            advance(cx, 200);
            assert_eq!(link(cx, &shell, "p-ivo"), Some(ContactLink::Blocked), "blocked again");
            assert!(exists(cx, handle, blocked_row), "listed under Blocked again");
        }
    }
    let blocks = with_store(cx, &shell, |s| {
        s.audit().iter().filter(|a| a.action == methods::CONTACT_BLOCK).count()
    });
    assert_eq!(blocks, 1, "the second block is audited");
}
