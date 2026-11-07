// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The conversation view follows the store's stream order, and the span
//! boundaries render (S10c c03 r2): a row inserted by order key lands in
//! the middle, 4,096 spans render with bounded markers, and a span past the
//! text refuses the whole message on screen.

mod common;
mod sdk_fake;

use common::*;
use ramflux_desktop_core::{ContextId, MomentId};
use ramflux_desktop_test::v1_fixtures::{
    entity, identity, message_update, service_update, text_message,
};
use ramflux_desktop_ui::route::Page;
use ramflux_protocol::message::{EntityKind, ServiceAction};

const DM: &str = "dm-ana";

fn open_dm(
    cx: &mut gpui_kit::TestAppContext,
    name: &str,
) -> (
    ramflux_desktop_test::fake_bus::FakeBus,
    std::path::PathBuf,
    gpui_kit::WindowHandle<gpui_kit::base::Root>,
    gpui_kit::Entity<ramflux_desktop_ui::Shell>,
) {
    let (fake, _state, path) = sdk_fake::start(name);
    let (handle, shell) = open_with(cx, sdk_fake::options(&path));
    cx.update(|cx| ramflux_desktop_ui::i18n::select_locale("en", cx));
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the conversation", |cx| {
        with_store(cx, &shell, |s| s.context(&ContextId::new(DM)).is_some())
    });
    show_context(cx, handle, &shell, DM);
    (fake, path, handle, shell)
}

fn id(n: u8) -> MomentId {
    MomentId::new(format!("msg:{DM}:{}", format!("{n:02x}").repeat(4)))
}

#[gpui_kit::test]
fn a_row_inserted_by_order_key_lands_between_its_neighbours(cx: &mut gpui_kit::TestAppContext) {
    let _locale = locale_guard();
    let (fake, path, _handle, shell) = open_dm(cx, "s10c-stream-order");
    for (n, hlc) in [(1_u8, 100_u64), (2, 200)] {
        let update = message_update(
            identity(DM, &[n; 4], "dev-ana", hlc, hlc),
            text_message(&format!("message {n}"), Vec::new()),
        );
        assert!(sdk_fake::push_generated(&path, update));
    }
    wait(cx, "both messages", |cx| with_store(cx, &shell, |s| s.moment(&id(2)).is_some()));
    advance(cx, 200);
    // A service row with a clock between the two arrives last.
    let pin = service_update(
        identity(DM, &[3; 4], "dev-ana", 150, 150),
        ServiceAction::ServicePinChanged,
        &[],
        None,
    );
    assert!(sdk_fake::push_generated(&path, pin));
    wait(cx, "the pin row", |cx| with_store(cx, &shell, |s| s.moment(&id(3)).is_some()));
    advance(cx, 200);
    let shown: Vec<String> = rows(cx, &shell)
        .into_iter()
        .filter(|row| [1_u8, 2, 3].iter().any(|n| id(*n).to_string() == *row))
        .collect();
    assert_eq!(
        shown,
        vec![id(1).to_string(), id(3).to_string(), id(2).to_string()],
        "displayed order follows the stream order (hlc 100, 150, 200)"
    );
    fake.shutdown();
}

#[gpui_kit::test]
fn four_thousand_ninety_six_spans_render_with_bounded_markers(cx: &mut gpui_kit::TestAppContext) {
    let _locale = locale_guard();
    let (fake, path, handle, shell) = open_dm(cx, "s10c-spans-4096");
    let spans = vec![entity(0, 1, EntityKind::EntityItalic); 4_096];
    let update = message_update(identity(DM, &[7; 4], "dev-ana", 70, 7), text_message("x", spans));
    assert!(sdk_fake::push_generated(&path, update));
    wait(cx, "the message", |cx| with_store(cx, &shell, |s| s.moment(&id(7)).is_some()));
    let row = format!("moment-{}", id(7));
    wait(cx, "the first marker", |cx| exists_in(cx, handle, &row, "message-span-0"));
    assert!(exists_in(cx, handle, &row, "message-span-63"), "markers up to the bound");
    assert!(!exists_in(cx, handle, &row, "message-span-64"), "no marker beyond the bound");
    fake.shutdown();
}

#[gpui_kit::test]
fn a_span_past_the_text_refuses_the_whole_message_on_screen(cx: &mut gpui_kit::TestAppContext) {
    let _locale = locale_guard();
    let (fake, path, handle, shell) = open_dm(cx, "s10c-spans-refused");
    // Three scalars of text, a span of four.
    let update = message_update(
        identity(DM, &[8; 4], "dev-ana", 80, 8),
        text_message("abc", vec![entity(0, 4, EntityKind::EntityBold)]),
    );
    assert!(sdk_fake::push_generated(&path, update));
    wait(cx, "the refusal shown", |cx| {
        has_label(
            cx,
            handle,
            "This message uses formatting this app cannot show, so its text is not displayed.",
        )
    });
    with_store(cx, &shell, |s| {
        assert!(s.bodies().get(&id(8)).is_none(), "no partial text in the store");
    });
    fake.shutdown();
}
