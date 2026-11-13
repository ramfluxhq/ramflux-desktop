// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Source-backed conversation composition, native input and local actions.
mod common;
#[path = "support/desk_conversation.rs"]
mod source_facts;
use common::*;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{Entity, TestAppContext};
use ramflux_desktop_core::{DomainEvent, MomentId, methods};
use ramflux_desktop_ui::{Shell, workspace::Conversation};

fn load(cx: &mut TestAppContext, shell: &Entity<Shell>) {
    let scenario = source_facts::scenario();
    let store = store_handle(cx, shell);
    cx.update(|cx| store.update(cx, |s, cx| s.load_scenario(&scenario, cx)));
    advance(cx, 1000);
    cx.update(|cx| ramflux_desktop_ui::i18n::select_locale("en", cx));
    cx.run_until_parked();
}
fn field(cx: &mut TestAppContext, shell: &Entity<Shell>) -> Entity<Conversation> {
    let workspace = workspace(cx, shell);
    cx.update(|cx| workspace.read(cx).conversation().clone())
}
fn focus_row(cx: &mut TestAppContext, field: &Entity<Conversation>, id: &str) {
    let id = MomentId::new(id);
    cx.update(|cx| field.update(cx, |f, cx| f.focus_moment(&id, cx)));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn own_peer_joined_agent_quote_attachment_and_reactions_use_source_facts(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    rust_i18n::set_locale("en");
    let (handle, shell) = open(cx, "s02_daily");
    load(cx, &shell);
    show_context(cx, handle, &shell, "c-lena");
    let field = field(cx, &shell);
    focus_row(cx, &field, "x-02");
    assert!(
        !exists_in(cx, handle, "moment-x-02", "message-sender"),
        "consecutive same-author rows must suppress repeated identity"
    );
    focus_row(cx, &field, "x-04");
    assert!(
        exists_in(cx, handle, "moment-x-04", "message-menu"),
        "a message action path must remain visible without hover"
    );
    focus_row(cx, &field, "x-05");
    assert!(
        exists_in(cx, handle, "moment-x-05", "message-reply"),
        "source quote must compose inside the message"
    );
    assert!(
        exists_in(cx, handle, "moment-x-05", "message-attachment"),
        "source attachment must retain its transfer facts"
    );
    assert!(
        exists_in(cx, handle, "moment-x-05", "message-reactions"),
        "source reactions must render without inventing a mutation"
    );
    click_in(cx, handle, "moment-x-05", "message-reply");
    assert_eq!(
        cx.update(|cx| field.read(cx).highlighted().cloned()),
        Some(MomentId::new("x-01")),
        "quote navigation must locate the actual source message"
    );
    advance(cx, 1000);
    focus_row(cx, &field, "x-05");
    assert!(
        has_label(cx, handle, "Original message deleted"),
        "a target deletion must invalidate cached excerpts in dependent rows"
    );
    assert!(
        cx.update(|cx| field.read(cx).message_text_state(&MomentId::new("x-01")).is_none()),
        "deleted text cannot retain a selection participant"
    );
    assert!(
        effects(cx, &shell).is_empty(),
        "viewing and locating messages must not mutate the source"
    );
}

#[gpui_kit::test]
fn native_selection_and_message_menu_copy_literal_text_without_mutating(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    rust_i18n::set_locale("en");
    let (handle, shell) = open(cx, "s02_daily");
    load(cx, &shell);
    show_context(cx, handle, &shell, "c-lena");
    let field = field(cx, &shell);
    focus_row(cx, &field, "x-03");
    let selection = cx
        .update(|cx| field.read(cx).message_text_state(&MomentId::new("x-03")).cloned())
        .expect("test: retained selection");
    in_window(cx, handle, |w, cx| {
        let focus = selection.read(cx).focus_handle().clone();
        w.focus(&focus, cx);
    });
    in_window(cx, handle, |w, cx| {
        w.press("secondary-a", cx);
        w.press("secondary-c", cx);
    });
    let copied = cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(
        copied.as_deref(),
        Some("Agreed. <img src='never'> & **literal** 😀"),
        "native selection must copy literal peer content, never interpreted markup"
    );
    click_in(cx, handle, "moment-x-03", "message-menu");
    in_window(cx, handle, |w, _| {
        assert!(
            !w.find_all_by_label("Edit message").is_empty(),
            "owned message menu must name the unavailable command"
        );
    });
    in_window(cx, handle, |w, cx| {
        let center = w.find_by_label("Edit message").bounds().center();
        w.pointer_down(center, gpui_kit::MouseButton::Left, cx);
        w.pointer_up(center, gpui_kit::MouseButton::Left, cx);
    });
    assert!(
        has_label(cx, handle, "Edit message"),
        "a disabled mutation must leave its menu open without activation"
    );
    in_window(cx, handle, |w, cx| w.press("escape", cx));
    assert!(
        in_window(cx, handle, |w, _| w.within("moment-x-03").find("message-menu").focused()
            == Some(true)),
        "Escape must restore the menu trigger"
    );
    click_in(cx, handle, "moment-x-03", "message-menu");
    in_window(cx, handle, |w, cx| {
        let target = w.find_by_label("Copy text").bounds().center();
        w.pointer_down(target, gpui_kit::MouseButton::Left, cx);
        w.pointer_up(target, gpui_kit::MouseButton::Left, cx);
    });
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text())).as_deref(),
        Some("Agreed. <img src='never'> & **literal** 😀"),
        "menu copy must read the current account body"
    );
    click_in(cx, handle, "moment-x-03", "message-menu");
    cx.update(|cx| cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string("sentinel".into())));
    advance(cx, 2000);
    // The popup was opened against an earlier snapshot. Its callback must
    // revalidate current content, rather than copying a captured plaintext body.
    in_window(cx, handle, |w, cx| {
        if let Some(copy) = w.try_find_by_label("Copy text") {
            let center = copy.bounds().center();
            w.pointer_down(center, gpui_kit::MouseButton::Left, cx);
            w.pointer_up(center, gpui_kit::MouseButton::Left, cx);
        }
    });
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text())).as_deref(),
        Some("sentinel"),
        "a popup opened before deletion must not copy erased text"
    );
    assert!(
        cx.update(|cx| field.read(cx).message_text_state(&MomentId::new("x-03")).is_none()),
        "a tombstone must drop the old native selection owner"
    );
    assert!(
        effects(cx, &shell).is_empty(),
        "copy and disabled changes cannot submit business effects"
    );
}

#[gpui_kit::test]
fn one_multiline_composer_preserves_drafts_and_enter_send_policy(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    rust_i18n::set_locale("en");
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 1000);
    show_context(cx, handle, &shell, "c-lena");
    click(cx, handle, "composer-input");
    in_window(cx, handle, |w, cx| {
        w.input("First line", cx);
        w.press("shift-enter", cx);
        w.input("第二行 😀", cx);
    });
    assert!(effects(cx, &shell).is_empty(), "Shift+Enter must insert a line without submitting");
    show_context(cx, handle, &shell, "c-team");
    show_context(cx, handle, &shell, "c-lena");
    let workspace = workspace(cx, &shell);
    let composer = cx.update(|cx| workspace.read(cx).composer().clone());
    assert_eq!(
        cx.update(|cx| composer.read(cx).input().read(cx).value().to_string()),
        "First line\n第二行 😀",
        "existing draft text must survive navigation"
    );
    in_window(cx, handle, |w, cx| w.press("enter", cx));
    let body = last_body(cx, &shell, methods::SEND_MESSAGE);
    assert_eq!(
        body.get("text").and_then(|v| v.as_str()),
        Some("First line\n第二行 😀"),
        "plain Enter must submit the existing context-scoped send intent once"
    );
    assert_eq!(
        effects(cx, &shell).iter().filter(|m| m.as_str() == methods::SEND_MESSAGE).count(),
        1,
        "one composer must submit one message"
    );
}

#[gpui_kit::test]
fn long_literal_bodies_fit_reading_width_in_both_themes_and_all_shell_widths(
    cx: &mut TestAppContext,
) {
    let _locale = locale_guard();
    rust_i18n::set_locale("en");
    const PARAGRAPHS: usize = 12;
    for width in [640., 800., 1024., 1440.] {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            let (handle, shell) = open_width(cx, "s02_daily", width);
            let mut scenario = source_facts::scenario();
            for (_, event) in &mut scenario.timeline {
                if let DomainEvent::MomentAdded(moment) = event
                    && moment.id == MomentId::new("x-03")
                    && let ramflux_desktop_core::MomentKind::Message(payload) = &mut moment.kind
                {
                    payload.text = "Verify daylight samples before confirming. 先核对资料。 😀\n"
                        .repeat(PARAGRAPHS);
                }
            }
            let store = store_handle(cx, &shell);
            cx.update(|cx| {
                store.update(cx, |s, cx| s.load_scenario(&scenario, cx));
                Theme::change(mode, None, cx);
            });
            advance(cx, 1000);
            show_context(cx, handle, &shell, "c-lena");
            let field = field(cx, &shell);
            focus_row(cx, &field, "x-03");
            in_window(cx, handle, |w, _| {
                let bubble = w.within("moment-x-03").find("message-bubble").bounds();
                assert!(
                    f32::from(bubble.size.width) <= 600.,
                    "long text must respect its reading measure at {width}"
                );
                assert!(
                    f32::from(bubble.origin.x) >= 0. && f32::from(bubble.right()) <= width + 1.,
                    "bubble must remain within the viewport at {width}"
                );
                assert!(
                    w.find("composer-input").bounds().size.height > gpui_kit::px(0.),
                    "long rows cannot displace the single composer"
                );
            });
        }
    }
}
