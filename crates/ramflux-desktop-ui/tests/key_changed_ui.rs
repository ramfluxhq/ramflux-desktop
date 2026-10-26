// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A contact whose key changed (or whose identity forked) is refused, not
//! asked about, by the new conversation and the new group forms: the
//! reason is shown, the composer or Create cannot go ahead, and nothing is
//! requested. (The stale case, which only asks, is covered next to each
//! form; adding to an existing group is covered with the members dock.)

mod common;

use common::*;
use gpui_kit::AppContext as _;
use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{MouseButton, TestAppContext};
use ramflux_desktop_core::{ParticipantId, methods};
use ramflux_desktop_ui::i18n::{tr, tr_args};

fn requested(
    cx: &mut TestAppContext,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::Shell>,
) -> usize {
    bodies(cx, shell)
        .iter()
        .filter(|(m, _)| m == methods::CONVERSATION_START || m == methods::GROUP_CREATE)
        .count()
}

#[gpui_kit::test]
fn a_new_conversation_with_a_changed_or_forked_contact_is_refused(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s05_group_safety");
    advance(cx, 500);
    for (who, name) in [("p-kay", "Kay Novak"), ("p-fox", "Fox Lund")] {
        let workspace = workspace(cx, &shell);
        cx.update_window(handle.into(), |_, window, cx| {
            workspace.update(cx, |w, cx| {
                w.open_conversation_with(ParticipantId::new(who), window, cx);
            });
        })
        .expect("window");
        cx.run_until_parked();
        assert!(exists(cx, handle, "new-conversation"), "{who}: the draft opens");
        let refusal = tr_args("conversation.new.error.key_changed", &[("name", name)]);
        assert!(has_label(cx, handle, &refusal), "{who}: the reason is shown: {refusal}");
        assert!(
            !exists(cx, handle, "new-conversation-stale-ack"),
            "{who}: a refusal is not a question"
        );
        assert!(exists(cx, handle, "composer-unavailable"), "{who}: the composer is blocked");
        type_into(cx, handle, "composer-input", "hello");
        click(cx, handle, "composer-send");
        advance(cx, 300);
        assert_eq!(requested(cx, &shell), 0, "{who}: nothing is requested");
    }
}

#[gpui_kit::test]
fn a_new_group_cannot_take_a_changed_or_forked_contact(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s05_group_safety");
    advance(cx, 500);
    click(cx, handle, "stack-new");
    let item = tr("stack.new_group").to_string();
    in_window(cx, handle, move |window, cx| {
        let at = window.find_by_label(&item).bounds().center();
        gpui_kit::test::TestEventExt::pointer_down(window, at, MouseButton::Left, cx);
        gpui_kit::test::TestEventExt::pointer_up(window, at, MouseButton::Left, cx);
    });
    let form = {
        let workspace = workspace(cx, &shell);
        cx.update(|cx| workspace.read(cx).new_group().clone())
    };
    type_into(cx, handle, "new-group-name", "Trip");
    for who in ["p-kay", "p-fox"] {
        let id = format!("new-group-member-{who}");
        let disabled = in_window(cx, handle, {
            let id = id.clone();
            move |window, _| window.find(id).disabled()
        });
        assert_eq!(disabled, Some(true), "{who} cannot be chosen");
        in_window(cx, handle, move |window, cx| window.click(id, cx));
        assert!(
            cx.update(|cx| form.read(cx).members().to_vec()).is_empty(),
            "{who} was not chosen"
        );
    }
    assert!(!exists(cx, handle, "new-group-stale-ack"), "no acknowledgement for a refusal");
    // Create with no member is refused by the policy's own check.
    click(cx, handle, "new-group-create");
    advance(cx, 300);
    assert_eq!(requested(cx, &shell), 0, "nothing is requested");
}
