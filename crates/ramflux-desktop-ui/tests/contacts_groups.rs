// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Adding contacts, sharing this account's card, and making groups in the
//! production window, on a fake daemon that resolves contacts when they are
//! added (see `resolve_fake`):
//!
//! - a principal id is added, shown as unverified (compare safety numbers),
//!   reachable, and a direct conversation starts with it at once;
//! - a contact card (pasted, or imported from a file) adds the contact it
//!   names, its self-declared kind marked; a bad card or an unknown id
//!   shows the daemon's reason under the field;
//! - a contact the daemon cannot reach yet cannot be messaged;
//! - "Your contact card" shows the card as a QR code and text, and copies it;
//! - a new group is created with its name and the chosen contacts, appears
//!   as a conversation and carries messages; its name survives the group
//!   reads; it is renamed in place.

mod common;
mod resolve_fake;
mod sdk_fake;

use common::*;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{MouseButton, TestAppContext, WindowHandle};
use ramflux_desktop_core::{
    ContactLink, ContextId, ContextKind, KindClaim, ParticipantId, PeerKind, RootSource,
    SafetyState,
};
use ramflux_desktop_ui::i18n::tr;
use ramflux_desktop_ui::identity::kind_text;
use ramflux_desktop_ui::route::Page;
use ramflux_desktop_ui::workspace::docks::contacts::add_form::FormNote;
use ramflux_desktop_ui::workspace::docks::contacts::my_card::CardView;
use ramflux_desktop_ui::workspace::new_group::Phase;
use sdk_fake::wait;
use serde_json::json;

type Handle = WindowHandle<gpui_kit::base::Root>;
type ShellEntity = gpui_kit::Entity<ramflux_desktop_ui::Shell>;
type Fake = ramflux_desktop_test::fake_bus::FakeBus;

fn open(cx: &mut TestAppContext, name: &str) -> (Handle, ShellEntity, resolve_fake::Shared, Fake) {
    open_with_home(cx, name, true)
}

fn open_with_home(
    cx: &mut TestAppContext,
    name: &str,
    home: bool,
) -> (Handle, ShellEntity, resolve_fake::Shared, Fake) {
    let (fake, state, path) = resolve_fake::start_with(name, !home);
    let (handle, shell) = open_with(cx, sdk_fake::options(&path));
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the account", |cx| {
        with_store(cx, &shell, |s| s.context(&ContextId::new("dm-ana")).is_some())
    });
    click(cx, handle, "rail-contacts");
    (handle, shell, state, fake)
}

fn click_label(cx: &mut TestAppContext, handle: Handle, label: &str) {
    let label = label.to_owned();
    in_window(cx, handle, move |window, cx| {
        let at = window.find_by_label(&label).bounds().center();
        gpui_kit::test::TestEventExt::pointer_down(window, at, MouseButton::Left, cx);
        gpui_kit::test::TestEventExt::pointer_up(window, at, MouseButton::Left, cx);
    });
}

fn clear(cx: &mut TestAppContext, handle: Handle, id: &'static str) {
    in_window(cx, handle, move |window, cx| {
        window.click(id, cx);
        window.press("secondary-a", cx);
        window.press("backspace", cx);
    });
}

fn label(cx: &mut TestAppContext, handle: Handle, id: &str) -> Option<String> {
    let id = id.to_owned();
    in_window(cx, handle, move |window, _| {
        window.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
}

fn label_in(cx: &mut TestAppContext, handle: Handle, scope: &str, id: &str) -> Option<String> {
    let (scope, id) = (scope.to_owned(), id.to_owned());
    in_window(cx, handle, move |window, _| {
        window.within(scope).try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
}

fn note(cx: &mut TestAppContext, shell: &ShellEntity) -> Option<FormNote> {
    let workspace = workspace(cx, shell);
    cx.update(|cx| workspace.read(cx).contacts_dock().read(cx).add_note().cloned())
}

fn add(cx: &mut TestAppContext, handle: Handle, text: &str) {
    type_into(cx, handle, "contact-add-id", text);
    click(cx, handle, "contact-add");
}

fn contact(
    cx: &mut TestAppContext,
    shell: &ShellEntity,
    id: &str,
) -> Option<ramflux_desktop_core::Participant> {
    let id = ParticipantId::new(id);
    with_store(cx, shell, |s| s.participant(&id).cloned())
}

fn current(cx: &mut TestAppContext, shell: &ShellEntity) -> Option<ContextId> {
    let workspace = workspace(cx, shell);
    cx.update(|cx| workspace.read(cx).current_context().cloned())
}

/// The cards sent with `contact.add {card}`, in order.
fn cards_sent(state: &resolve_fake::Shared) -> Vec<serde_json::Value> {
    seen(state, "contact.add").into_iter().filter_map(|b| b.get("card").cloned()).collect()
}

fn seen(state: &resolve_fake::Shared, method: &str) -> Vec<serde_json::Value> {
    state
        .lock()
        .expect("state")
        .seen
        .iter()
        .filter(|(m, _)| m == method)
        .map(|(_, b)| b.clone())
        .collect()
}

#[gpui_kit::test]
fn a_principal_id_is_added_unverified_and_a_conversation_starts_at_once(cx: &mut TestAppContext) {
    let (handle, shell, state, fake) = open(cx, "f14-add-id");
    type_into(cx, handle, "contact-add-id", "principal-kai");
    assert_eq!(
        label(cx, handle, "contact-add-kind").as_deref(),
        Some(tr("dock.contacts.add.kind_id").as_ref()),
        "before anything is sent the form says an id is trusted on first use"
    );
    click(cx, handle, "contact-add");
    assert_eq!(note(cx, &shell), Some(FormNote::Adding(ParticipantId::new("principal-kai"))));
    wait(cx, "kai reachable", |cx| {
        contact(cx, &shell, "principal-kai")
            .is_some_and(|p| p.reachable == Some(true) && p.display == "Kai")
    });
    let kai = contact(cx, &shell, "principal-kai").expect("kai");
    assert_eq!((kai.link_state(), kai.safety), (ContactLink::Accepted, SafetyState::Unverified));
    assert_eq!(seen(&state, "contact.add")[0]["principal_id"], "principal-kai");
    wait(cx, "the field cleared", |cx| note(cx, &shell).is_none());
    assert_eq!(
        label_in(cx, handle, "contacts-principal-kai", "contact-safety").as_deref(),
        None,
        "the safety line is text, not a labeled control"
    );
    assert!(has_label(cx, handle, &tr("dock.contacts.reachable")));
    // A name lookup's kind is only claimed: the mark says so.
    let claimed_person = kind_text(KindClaim::Asserted(PeerKind::Human)).expect("a kind");
    assert_eq!(
        label_in(cx, handle, "contacts-principal-kai", "contact-kind").as_deref(),
        Some(claimed_person.as_ref())
    );
    assert!(exists(cx, handle, "contacts-principal-kai"));

    // The conversation starts right away: the daemon resolved the devices.
    click_in(cx, handle, "contacts-principal-kai", "message");
    type_into(cx, handle, "composer-input", "Hello Kai");
    click(cx, handle, "composer-send");
    wait(cx, "the started conversation", |cx| current(cx, &shell).is_some());
    let submitted = seen(&state, "message.submit");
    assert_eq!(submitted.len(), 1);
    assert_eq!(submitted[0]["recipient_device_id"], "device-kai");
    assert_eq!(submitted[0]["recipient_principal_commitment"], "cm-principal-kai");
    let id = current(cx, &shell).expect("shown");
    assert_eq!(
        with_store(cx, &shell, |s| s.context(&id).map(|c| c.kind)),
        Some(ContextKind::Direct)
    );
    fake.shutdown();
}

#[gpui_kit::test]
fn cards_add_the_contact_they_name_and_refusals_show_the_daemon_reason(cx: &mut TestAppContext) {
    let (handle, shell, state, fake) = open(cx, "f14-add-card");
    add(cx, handle, "principal-nobody");
    wait(cx, "the node's answer", |cx| matches!(note(cx, &shell), Some(FormNote::Error(_))));
    let Some(FormNote::Error(reason)) = note(cx, &shell) else { unreachable!() };
    // The typed reason, in the user's words (never the daemon's code).
    let unknown = tr("backend.contact.unknown_principal");
    assert!(reason.contains(unknown.as_ref()), "{reason}");

    clear(cx, handle, "contact-add-id");
    add(cx, handle, "ramflux:contact/forged#sig");
    wait(cx, "the card refused", |cx| matches!(note(cx, &shell), Some(FormNote::Error(_))));
    let Some(FormNote::Error(reason)) = note(cx, &shell) else { unreachable!() };
    let forged = tr("backend.contact.card_signature_invalid");
    assert!(reason.contains(forged.as_ref()), "{reason}");

    clear(cx, handle, "contact-add-id");
    type_into(cx, handle, "contact-add-id", "ramflux:contact/principal-bot#sig-bot");
    assert_eq!(
        label(cx, handle, "contact-add-kind").as_deref(),
        Some(tr("dock.contacts.add.kind_card").as_ref())
    );
    click(cx, handle, "contact-add");
    assert_eq!(note(cx, &shell), Some(FormNote::AddingCard));
    wait(cx, "the card's contact", |cx| contact(cx, &shell, "principal-bot").is_some());
    let bot = contact(cx, &shell, "principal-bot").expect("bot");
    assert_eq!(
        (bot.kind, bot.kind_claim, bot.reachable),
        (
            ramflux_desktop_core::ParticipantKind::Agent,
            KindClaim::Asserted(PeerKind::HeadlessAi),
            Some(true)
        )
    );
    let cards: Vec<serde_json::Value> = cards_sent(&state);
    assert_eq!(cards, ["ramflux:contact/forged#sig", "ramflux:contact/principal-bot#sig-bot"]);
    wait(cx, "the field cleared", |cx| note(cx, &shell).is_none());
    let claimed_ai = kind_text(KindClaim::Asserted(PeerKind::HeadlessAi)).expect("a kind");
    assert_eq!(
        label_in(cx, handle, "contacts-principal-bot", "contact-kind").as_deref(),
        Some(claimed_ai.as_ref())
    );

    // A card file is read and added as it is.
    let dir = std::env::temp_dir().join(format!("rf-f14-card-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let file = dir.join("kai.rfcard");
    std::fs::write(&file, "ramflux:contact/principal-kai#sig-kai\n").expect("card file");
    let dock = {
        let workspace = workspace(cx, &shell);
        cx.update(|cx| workspace.read(cx).contacts_dock().clone())
    };
    in_window(cx, handle, |window, cx| {
        dock.update(cx, |dock, cx| dock.import_card_file(&file, window, cx));
    });
    wait(cx, "the file's contact", |cx| contact(cx, &shell, "principal-kai").is_some());
    assert_eq!(cards_sent(&state)[2], "ramflux:contact/principal-kai#sig-kai");
    let missing = dir.join("missing.rfcard");
    in_window(cx, handle, |window, cx| {
        dock.update(cx, |dock, cx| dock.import_card_file(&missing, window, cx));
    });
    assert!(
        matches!(note(cx, &shell), Some(FormNote::Error(_))),
        "an unreadable file is explained"
    );
    let _ = std::fs::remove_dir_all(&dir);
    fake.shutdown();
}

#[gpui_kit::test]
fn a_contact_that_is_not_reachable_cannot_be_messaged_yet(cx: &mut TestAppContext) {
    let (handle, shell, _, fake) = open(cx, "f14-unreachable");
    add(cx, handle, "principal-lu");
    wait(cx, "lu listed", |cx| {
        contact(cx, &shell, "principal-lu").is_some_and(|p| p.reachable == Some(false))
    });
    assert!(has_label(cx, handle, &tr("dock.contacts.unreachable")));
    click_in(cx, handle, "contacts-principal-lu", "message");
    let draft_open = {
        let workspace = workspace(cx, &shell);
        cx.update(|cx| workspace.read(cx).new_conversation().read(cx).is_open())
    };
    assert!(!draft_open, "Message is disabled while the contact is not reachable");
    fake.shutdown();
}

#[gpui_kit::test]
fn the_own_card_is_shown_as_a_qr_code_and_copied(cx: &mut TestAppContext) {
    let (handle, shell, state, fake) = open(cx, "f14-my-card");
    let dock = {
        let workspace = workspace(cx, &shell);
        cx.update(|cx| workspace.read(cx).contacts_dock().clone())
    };
    assert_eq!(cx.update(|cx| dock.read(cx).card_view(cx)), CardView::Hidden);
    click(cx, handle, "contact-card-show");
    wait(cx, "the card", |cx| {
        matches!(cx.update(|cx| dock.read(cx).card_view(cx)), CardView::Ready(_))
    });
    assert_eq!(seen(&state, "contact.card.export").len(), 1);
    assert!(
        has_label(cx, handle, &tr("dock.contacts.card.qr")),
        "the QR code is an image with a name"
    );
    assert_eq!(label(cx, handle, "contact-card-text").as_deref(), Some(resolve_fake::MY_CARD));
    click(cx, handle, "contact-card-copy");
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some(resolve_fake::MY_CARD)
    );
    click(cx, handle, "contact-card-copy-link");
    assert_eq!(
        cx.read_from_clipboard().and_then(|c| c.text()).as_deref(),
        Some(resolve_fake::MY_LINK)
    );
    click(cx, handle, "contact-card-hide");
    assert_eq!(cx.update(|cx| dock.read(cx).card_view(cx)), CardView::Hidden);
    fake.shutdown();
}

#[gpui_kit::test]
fn a_new_group_is_created_with_its_members_named_and_renamed(cx: &mut TestAppContext) {
    let (handle, shell, state, fake) = open(cx, "f14-group");
    add(cx, handle, "principal-kai");
    wait(cx, "kai", |cx| {
        contact(cx, &shell, "principal-kai").is_some_and(|p| p.reachable == Some(true))
    });
    add(cx, handle, "principal-lu");
    wait(cx, "lu", |cx| contact(cx, &shell, "principal-lu").is_some());
    click(cx, handle, "rail-contacts");

    click(cx, handle, "stack-new");
    click_label(cx, handle, &tr("stack.new_group"));
    let form = {
        let workspace = workspace(cx, &shell);
        cx.update(|cx| workspace.read(cx).new_group().clone())
    };
    assert!(cx.update(|cx| form.read(cx).is_open()));
    assert!(exists(cx, handle, "new-group"), "the form replaces the conversation field");
    assert!(!exists(cx, handle, "composer-input"), "no composer while making a group");

    // Refusals before anything is sent.
    click(cx, handle, "new-group-create");
    assert_eq!(
        cx.update(|cx| form.read(cx).phase().clone()),
        Phase::Failed(tr("group.new.error.empty_name"))
    );
    type_into(cx, handle, "new-group-name", "Weekend trip");
    click(cx, handle, "new-group-create");
    assert_eq!(
        cx.update(|cx| form.read(cx).phase().clone()),
        Phase::Failed(tr("group.new.error.no_members"))
    );
    click(cx, handle, "new-group-member-principal-lu");
    assert!(
        cx.update(|cx| form.read(cx).members().is_empty()),
        "an unreachable contact cannot be chosen"
    );
    click(cx, handle, "new-group-member-principal-kai");
    assert_eq!(
        cx.update(|cx| form.read(cx).members().to_vec()),
        vec![ParticipantId::new("principal-kai")]
    );
    assert!(seen(&state, "group.create").is_empty());

    let id = cx.update(|cx| form.read(cx).context_id().cloned()).expect("an id");
    click(cx, handle, "new-group-create");
    wait(cx, "the group shown", |cx| current(cx, &shell) == Some(id.clone()));
    let created = seen(&state, "group.create");
    assert_eq!(created.len(), 1);
    assert_eq!(created[0]["conversation_id"], id.as_str());
    assert_eq!(created[0]["title"], "Weekend trip");
    assert_eq!(created[0]["creator_id"], "device-me");
    let added = seen(&state, "group.member.add");
    assert_eq!(added.len(), 1);
    assert_eq!(
        (added[0]["member_id"].as_str(), added[0]["role"].as_str()),
        (Some("device-kai"), Some("member"))
    );
    let context = with_store(cx, &shell, |s| s.context(&id).cloned()).expect("context");
    assert_eq!((context.kind, context.title.as_str()), (ContextKind::Group, "Weekend trip"));
    assert!(context.participants.contains(&ParticipantId::new("principal-kai")));

    // Messages flow; every group call carries the name, so it stays.
    type_into(cx, handle, "composer-input", "Who brings the tent?");
    click(cx, handle, "composer-send");
    wait(cx, "the group message", |cx| {
        with_store(cx, &shell, |s| s.context_moment_count(&id) == 1)
    });
    for _ in 0..10 {
        advance(cx, 100);
    }
    assert!(seen(&state, "group.send").iter().all(|b| b["title"] == "Weekend trip"));
    assert!(seen(&state, "group.receive").iter().all(|b| b["title"] == "Weekend trip"));
    assert_eq!(
        with_store(cx, &shell, |s| s.context(&id).map(|c| c.title.clone())).as_deref(),
        Some("Weekend trip")
    );

    // Rename in place.
    click(cx, handle, "conversation-more");
    click_label(cx, handle, &tr("group.rename.action"));
    assert!(exists(cx, handle, "conversation-rename-input"));
    clear(cx, handle, "conversation-rename-input");
    click(cx, handle, "conversation-rename-save");
    assert_eq!(
        label(cx, handle, "conversation-rename-status").as_deref(),
        Some(tr("group.new.error.empty_name").as_ref()),
        "a blank name is refused in place"
    );
    type_into(cx, handle, "conversation-rename-input", "Lake trip");
    in_window(cx, handle, |window, cx| window.press("enter", cx));
    wait(cx, "the new name", |cx| {
        with_store(cx, &shell, |s| s.context(&id).is_some_and(|c| c.title == "Lake trip"))
    });
    assert!(!exists(cx, handle, "conversation-rename-input"), "the field closes once renamed");
    for _ in 0..10 {
        advance(cx, 100);
    }
    assert_eq!(
        with_store(cx, &shell, |s| s.context(&id).map(|c| c.title.clone())).as_deref(),
        Some("Lake trip"),
        "later group reads keep it"
    );
    fake.shutdown();
}

#[gpui_kit::test]
fn a_direct_conversation_has_no_rename_and_cancel_leaves_no_group(cx: &mut TestAppContext) {
    let (handle, shell, state, fake) = open(cx, "f14-cancel");
    click(cx, handle, "rail-contacts");
    show_context(cx, handle, &shell, "dm-ana");
    click(cx, handle, "conversation-more");
    assert!(
        !has_label(cx, handle, &tr("group.rename.action")),
        "a direct conversation is named after the contact"
    );
    in_window(cx, handle, |window, cx| window.press("escape", cx));
    let workspace = workspace(cx, &shell);
    in_window(cx, handle, |window, cx| workspace.update(cx, |w, cx| w.open_new_group(window, cx)));
    assert!(
        exists(cx, handle, "new-group-no-contacts"),
        "with no contacts the form says what to do"
    );
    click(cx, handle, "new-group-cancel");
    assert!(!exists(cx, handle, "new-group"));
    assert!(current(cx, &shell).is_some(), "a conversation is shown again");
    assert!(seen(&state, "group.create").is_empty());
    fake.shutdown();
}

#[gpui_kit::test]
fn a_looked_up_identity_is_provisional_until_a_card_replaces_it_after_confirmation(
    cx: &mut TestAppContext,
) {
    let (handle, shell, state, fake) = open(cx, "f14-root-replace");
    add(cx, handle, "principal-kai");
    wait(cx, "kai provisional", |cx| {
        contact(cx, &shell, "principal-kai")
            .is_some_and(|p| p.root_source == Some(RootSource::Lookup))
    });
    wait(cx, "the field cleared", |cx| note(cx, &shell).is_none());
    assert_eq!(
        label_in(cx, handle, "contacts-principal-kai", "contact-root").as_deref(),
        Some(tr("dock.contacts.root.lookup").as_ref())
    );

    // A card naming another root is held until the user compares both.
    clear(cx, handle, "contact-add-id");
    add(cx, handle, "ramflux:contact/principal-kai#other-root");
    wait(cx, "the confirmation", |cx| matches!(note(cx, &shell), Some(FormNote::ConfirmRoot(_))));
    let Some(FormNote::ConfirmRoot(text)) = note(cx, &shell) else { unreachable!() };
    assert!(
        text.contains("cm-principal-kai") && text.contains("cm-other-principal-kai"),
        "both roots are shown: {text}"
    );
    assert_eq!(
        contact(cx, &shell, "principal-kai").and_then(|p| p.root_source),
        Some(RootSource::Lookup),
        "nothing is replaced before the user confirms"
    );
    click(cx, handle, "contact-root-keep");
    assert_eq!(note(cx, &shell), None, "Keep drops the card");
    click(cx, handle, "contact-add");
    wait(cx, "asked again", |cx| matches!(note(cx, &shell), Some(FormNote::ConfirmRoot(_))));
    click(cx, handle, "contact-root-replace");
    wait(cx, "the card's root", |cx| {
        contact(cx, &shell, "principal-kai")
            .is_some_and(|p| p.root_source == Some(RootSource::Card))
    });
    let adds = seen(&state, "contact.add");
    assert_eq!(
        adds.last().map(|b| b["replace_provisional_root"].clone()),
        Some(json!("cm-principal-kai"))
    );
    assert!(
        adds.iter().filter(|b| b.get("replace_provisional_root").is_some()).count() == 1,
        "only the confirmed card names the root it replaces"
    );
    wait(cx, "pinned label", |cx| {
        label_in(cx, handle, "contacts-principal-kai", "contact-root").as_deref()
            == Some(tr("dock.contacts.root.card").as_ref())
    });

    // Verify compares the number first; then the identity is confirmed.
    click_in(cx, handle, "contacts-principal-kai", "verify");
    assert!(exists(cx, handle, "contact-verify-number"));
    assert!(seen(&state, "contact.verify").is_empty(), "opening the comparison changes nothing");
    click_in(cx, handle, "contacts-principal-kai", "verify-confirm");
    wait(cx, "verified", |cx| {
        contact(cx, &shell, "principal-kai")
            .is_some_and(|p| p.root_source == Some(RootSource::Verified))
    });
    assert_eq!(
        seen(&state, "contact.verify")[0]["contact_identity_commitment"],
        "cm-other-principal-kai"
    );
    fake.shutdown();
}

#[gpui_kit::test]
fn without_a_home_node_cards_say_so_and_nothing_is_sent(cx: &mut TestAppContext) {
    let (handle, shell, state, fake) = open_with_home(cx, "f14-no-home", false);
    let dock = {
        let workspace = workspace(cx, &shell);
        cx.update(|cx| workspace.read(cx).contacts_dock().clone())
    };
    wait(cx, "the account's status", |cx| {
        cx.update(|cx| dock.read(cx).card_view(cx)) == CardView::NoHomeNode
    });
    assert_eq!(
        label(cx, handle, "contact-card-status").as_deref(),
        Some(tr("backend.contact.no_home_node").as_ref())
    );
    add(cx, handle, "ramflux:contact/principal-kai#sig-kai");
    assert_eq!(note(cx, &shell), Some(FormNote::Error(tr("backend.contact.no_home_node"))));
    assert!(cards_sent(&state).is_empty(), "no card reaches the daemon");
    assert!(seen(&state, "contact.card.export").is_empty());
    fake.shutdown();
}
