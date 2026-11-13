// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Source-backed conversation composition, native input and local actions.
mod common;
#[path = "support/desk_conversation.rs"]
mod source_facts;
use common::*;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{Entity, Focusable as _, TestAppContext};
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
fn focus_row(
    cx: &mut TestAppContext,
    handle: gpui_kit::WindowHandle<gpui_kit::base::Root>,
    field: &Entity<Conversation>,
    id: &str,
) {
    let id = MomentId::new(id);
    cx.update(|cx| field.update(cx, |f, cx| f.focus_moment(&id, cx)));
    cx.run_until_parked();
    in_window(cx, handle, |_, _| {});
}

#[gpui_kit::test]
fn own_peer_joined_agent_quote_attachment_and_reactions_use_source_facts(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    rust_i18n::set_locale("en");
    let (handle, shell) = open(cx, "s02_daily");
    load(cx, &shell);
    show_context(cx, handle, &shell, "c-lena");
    let field = field(cx, &shell);
    focus_row(cx, handle, &field, "x-02");
    assert!(
        !exists_in(cx, handle, "moment-x-02", "message-sender"),
        "consecutive same-author rows must suppress repeated identity"
    );
    focus_row(cx, handle, &field, "x-04");
    assert!(
        exists_in(cx, handle, "moment-x-04", "message-menu"),
        "a message action path must remain visible without hover"
    );
    focus_row(cx, handle, &field, "x-05");
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
    focus_row(cx, handle, &field, "x-05");
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
    focus_row(cx, handle, &field, "x-03");
    let selection = cx
        .update(|cx| field.read(cx).message_text_state(&MomentId::new("x-03")).cloned())
        .expect("test: retained selection");
    in_window(cx, handle, |w, cx| {
        let focus = selection.read(cx).focus_handle(cx);
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
            focus_row(cx, handle, &field, "x-03");
            in_window(cx, handle, |w, _| {
                let bubble = w.within("moment-x-03").find("message-bubble").bounds();
                assert!(
                    f32::from(bubble.size.width) >= 200.,
                    "native body controls must retain a usable reading width at {width}"
                );
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

#[gpui_kit::test]
fn untrusted_identity_and_unbroken_quote_attachment_stay_within_compact_bubble(
    cx: &mut TestAppContext,
) {
    let _locale = locale_guard();
    const UNBROKEN_LENGTH: usize = 300;
    let (handle, shell) = open_width(cx, "s02_daily", 640.);
    let mut scenario = source_facts::scenario();
    for (_, event) in &mut scenario.timeline {
        match event {
            DomainEvent::ParticipantUpserted(person) if person.id.as_str() == "p-lena" => {
                person.display = "\u{202e}\u{200b}Ana".into()
            }
            DomainEvent::ObjectUpserted(object) if object.id.as_str() == "o-moodboard" => {
                object.name = Some("f".repeat(UNBROKEN_LENGTH))
            }
            DomainEvent::MomentAdded(moment) if moment.id == MomentId::new("x-01") => {
                if let ramflux_desktop_core::MomentKind::Message(payload) = &mut moment.kind {
                    payload.text = format!("https://example.org/{}", "x".repeat(UNBROKEN_LENGTH));
                }
            }
            _ => {}
        }
    }
    let store = store_handle(cx, &shell);
    cx.update(|cx| store.update(cx, |s, cx| s.load_scenario(&scenario, cx)));
    advance(cx, 1000);
    show_context(cx, handle, &shell, "c-lena");
    let field = field(cx, &shell);
    focus_row(cx, handle, &field, "x-01");
    in_window(cx, handle, |w, _| {
        let row = w.within("moment-x-01");
        assert_eq!(
            row.find("message-avatar").label(),
            Some("A"),
            "invisible controls must not become an avatar initial"
        );
        assert!(
            row.find("message-sender")
                .label()
                .is_some_and(|label| !label.contains('\u{202e}') && !label.contains('\u{200b}')),
            "display labels must strip bidi and invisible identity controls"
        );
    });
    focus_row(cx, handle, &field, "x-05");
    in_window(cx, handle, |w, _| {
        let row = w.within("moment-x-05");
        let bubble = row.find("message-bubble").bounds();
        for part in ["message-reply", "message-attachment"] {
            let bounds = row.find(part).bounds();
            assert!(
                bounds.right() <= bubble.right() && f32::from(bounds.right()) <= 640.,
                "unbroken content must remain inside its reading viewport: {part}"
            );
        }
    });
}

#[gpui_kit::test]
fn drafts_are_isolated_between_contexts_and_erased_on_lock(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 1000);
    click(cx, handle, "stack-c-lena");
    in_window(cx, handle, |w, cx| w.input("private Lena draft", cx));
    let workspace = workspace(cx, &shell);
    let composer = cx.update(|cx| workspace.read(cx).composer().clone());
    click(cx, handle, "stack-c-team");
    assert!(
        cx.update(|cx| composer.read(cx).input().read(cx).value().is_empty()),
        "another conversation must start with its own empty draft"
    );
    in_window(cx, handle, |w, cx| w.press("enter", cx));
    assert!(effects(cx, &shell).is_empty(), "an empty Team draft must not send Lena's text");
    in_window(cx, handle, |w, cx| {
        w.input("Team draft", cx);
        w.press("enter", cx);
    });
    assert_eq!(
        last_body(cx, &shell, methods::SEND_MESSAGE).get("text").and_then(|v| v.as_str()),
        Some("Team draft"),
        "Team must submit only Team's draft"
    );
    show_context(cx, handle, &shell, "c-lena");
    assert_eq!(
        cx.update(|cx| composer.read(cx).input().read(cx).value().to_string()),
        "private Lena draft",
        "returning must restore the correct draft"
    );
    in_window(cx, handle, |w, cx| w.press("secondary-,", cx));
    click(cx, handle, "settings-lock");
    advance(cx, 500);
    assert!(
        cx.update(|cx| composer.read(cx).input().read(cx).value().is_empty()),
        "locking must erase active plaintext"
    );
    let store = store_handle(cx, &shell);
    let scenario =
        ramflux_desktop_test::load_fixture("s02_daily").expect("test: new source session");
    cx.update(|cx| store.update(cx, |s, cx| s.load_scenario(&scenario, cx)));
    advance(cx, 1000);
    show_context(cx, handle, &shell, "c-lena");
    assert!(
        cx.update(|cx| composer.read(cx).input().read(cx).value().is_empty()),
        "a session reset cannot resurrect an earlier plaintext draft"
    );
}

#[gpui_kit::test]
fn selected_plain_body_preserves_all_whitespace_and_unicode_verbatim(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s02_daily");
    const BODY: &str = "\n  a    b\t😀\n\nline\u{202e} text\n\n";
    let mut scenario = source_facts::scenario();
    for (_, event) in &mut scenario.timeline {
        if let DomainEvent::MomentAdded(moment) = event
            && moment.id == MomentId::new("x-03")
            && let ramflux_desktop_core::MomentKind::Message(payload) = &mut moment.kind
        {
            payload.text = BODY.into();
        }
    }
    let store = store_handle(cx, &shell);
    cx.update(|cx| store.update(cx, |s, cx| s.load_scenario(&scenario, cx)));
    advance(cx, 1000);
    show_context(cx, handle, &shell, "c-lena");
    let field = field(cx, &shell);
    focus_row(cx, handle, &field, "x-03");
    let state = cx
        .update(|cx| field.read(cx).message_text_state(&MomentId::new("x-03")).cloned())
        .expect("test: visible native selection");
    in_window(cx, handle, |w, cx| {
        let focus = state.read(cx).focus_handle(cx);
        w.focus(&focus, cx);
    });
    in_window(cx, handle, |w, cx| {
        w.press("secondary-a", cx);
        w.press("secondary-c", cx);
    });
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text())).as_deref(),
        Some(BODY),
        "selection must equal the stored body, including indentation, tabs and terminal blank lines"
    );
    assert_eq!(
        with_store(cx, &shell, |s| s.body(&MomentId::new("x-03")).map(str::to_owned)).as_deref(),
        Some(BODY),
        "the assertion must compare against the actual account body cache"
    );
}

#[gpui_kit::test]
fn selection_cache_is_viewport_bounded_and_append_touches_one_row(cx: &mut TestAppContext) {
    use ramflux_desktop_core::{
        ContextId, Lifecycle, MessagePayload, Moment, MomentKind, ParticipantId, Timestamp,
    };
    let _locale = locale_guard();
    const HISTORY_SIZE: usize = 3000;
    let (handle, shell) = open(cx, "s02_daily");
    let mut scenario = source_facts::scenario();
    scenario.timeline.retain(|(at, event)| {
        *at == 0
            && matches!(
                event,
                DomainEvent::AccountUpdated(_)
                    | DomainEvent::ParticipantUpserted(_)
                    | DomainEvent::SourceStatusChanged(_)
                    | DomainEvent::ListingsLoaded(_)
            )
    });
    let context = serde_json::from_value(
        serde_json::json!({"id":"bulk","kind":"Direct","title":"Bulk history"}),
    )
    .expect("test: source context");
    scenario.timeline.push((0, DomainEvent::ContextUpserted(context)));
    for ix in 0..=HISTORY_SIZE {
        scenario.timeline.push((
            if ix == HISTORY_SIZE { 2000 } else { 0 },
            DomainEvent::MomentAdded(Box::new(Moment {
                id: MomentId::new(format!("bulk-{ix:05}")),
                context_id: ContextId::new("bulk"),
                origin: ParticipantId::new("p-lena"),
                at: Timestamp(0),
                order: None,
                lifecycle: Lifecycle::Done,
                kind: MomentKind::Message(MessagePayload {
                    text: format!("Message {ix}"),
                    ..Default::default()
                }),
            })),
        ));
    }
    scenario.timeline.sort_by_key(|(at, _)| *at);
    let store = store_handle(cx, &shell);
    cx.update(|cx| store.update(cx, |s, cx| s.load_scenario(&scenario, cx)));
    advance(cx, 1000);
    show_context(cx, handle, &shell, "bulk");
    let field = field(cx, &shell);
    in_window(cx, handle, |_, _| {});
    let before = cx.update(|cx| field.read(cx).message_selection_counts());
    assert!(
        before.0 <= 64 && before.1 <= 128,
        "opening 3000 rows must create only viewport selection state: {before:?}"
    );
    assert_eq!(
        cx.update(|cx| field.read(cx).row_count()),
        HISTORY_SIZE,
        "virtualization must retain the full source history"
    );
    advance(cx, 1000);
    let after = cx.update(|cx| field.read(cx).message_selection_counts());
    assert_eq!(after.2, 1, "an appended message must not compare every cached body");
    assert!(
        after.0 <= 64 && after.1.saturating_sub(before.1) <= 16,
        "an append must create only newly visible states: {before:?} -> {after:?}"
    );
}

#[gpui_kit::test]
fn ime_candidate_confirmation_does_not_send_until_next_enter(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 1000);
    show_context(cx, handle, &shell, "c-lena");
    let workspace = workspace(cx, &shell);
    let input = cx.update(|cx| workspace.read(cx).composer().read(cx).input().clone());
    click(cx, handle, "composer-input");
    in_window(cx, handle, |w, cx| {
        let bounds = w.find("composer-input").bounds();
        let handler = gpui_kit::ElementInputHandler::new(bounds, input.clone());
        gpui_kit::test::TestInput::new(handler, w).compose_text(None, "候选", Some(2..2), cx);
    });
    in_window(cx, handle, |w, cx| w.press("enter", cx));
    assert!(
        effects(cx, &shell).is_empty(),
        "Enter confirming marked text must not submit a message"
    );
    in_window(cx, handle, |w, cx| w.press("enter", cx));
    assert_eq!(
        effects(cx, &shell).iter().filter(|m| m.as_str() == methods::SEND_MESSAGE).count(),
        1,
        "the next Enter may submit the committed text exactly once"
    );
}
