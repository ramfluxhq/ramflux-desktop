// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A long failure stays readable (DESK-PIN-03 L10): the status banner at the
//! top of the content area cuts it with an ellipsis (its full text stays in
//! the accessibility label) and stays inside the window, down to the
//! application's minimum size, in English and in Chinese. Only what needs
//! attention is in the banner (design B38): the neutral facts are not shown
//! and the error's value takes the room.

mod common;
mod sdk_fake;

use std::sync::Arc;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use ramflux_desktop_core::{Intent, IntentKind, RequestAnswer, selectors};
use ramflux_desktop_ui::i18n;
use ramflux_desktop_ui::route::Page;
use serde_json::json;

/// The longest refusal text the strip can show: the node lookup refused
/// without a reason (`backend.failure.local_network_maybe`, 208 characters
/// in English and 92 in Chinese, against 146 and 42 for a failed trust check).
const LONGEST: &str = r#"backend.failure.local_network_maybe {"host":"ramflux-node-01.local"}"#;

/// Opens a `width` px wide window whose status strip shows the longest
/// refusal text and returns the strip's failure text.
fn open_with_long_failure(
    cx: &mut TestAppContext,
    locale: &str,
    width: f32,
) -> (gpui_kit::WindowHandle<gpui_kit::base::Root>, String) {
    let (fake, state, path) = sdk_fake::start(&format!("ui-title-bar-{locale}-{width}"));
    fake.enable_stream();
    let fake = Arc::new(fake);
    let requests = state.lock().expect("state").requests.clone();
    let bus = Arc::downgrade(&fake);
    requests.set_publisher(Box::new(move |account, topic, body| {
        if let Some(bus) = bus.upgrade() {
            bus.publish(account, topic, body);
        }
    }));
    let id = requests.incoming("acct-main", "principal-new", None);
    cx.update(|cx| {
        gpui_kit::init(cx);
        ramflux_desktop_ui::init(cx);
        i18n::select_locale(locale, cx);
    });
    let (handle, shell) = open_at(cx, sdk_fake::options(&path), width, 860.);
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    fake.publish("acct-main", "contact.changed", json!({}));
    wait(cx, "the request", |cx| {
        with_store(cx, &shell, |s| !selectors::pending_contact_requests(s).is_empty())
    });
    requests.fail_decisions(Some(("ValidationFailed", LONGEST)));
    let store = store_handle(cx, &shell);
    cx.update(|cx| {
        store.update(cx, |handle, cx| {
            handle.dispatch(
                Intent::human(IntentKind::AnswerContactRequest {
                    request_id: id,
                    answer: RequestAnswer::Accept,
                }),
                cx,
            );
        });
    });
    wait(cx, "the refusal", |cx| with_store(cx, &shell, |s| !s.failures().is_empty()));
    let full = with_store(cx, &shell, |s| s.failures().last().map(i18n::failure))
        .expect("a failure")
        .to_string();
    // Keep the daemon alive for the test's duration.
    std::mem::forget(fake);
    (handle, full)
}

fn check(cx: &mut TestAppContext, locale: &str, width: f32) {
    let (handle, full) = open_with_long_failure(cx, locale, width);
    assert!(full.contains("ramflux-node-01.local"), "the longest refusal text: {full}");
    let (shown, window_width) = in_window(cx, handle, |window, _| {
        let shown =
            window.within("status-failure").find("status-failure-value").label().map(str::to_owned);
        (shown, window.viewport_size().width)
    });
    // The error's value is cut with an ellipsis inside the strip, never by
    // the strip's edge; the neutral chips gave way to it.
    let (value, strip, neutral) = in_window(cx, handle, |window, _| {
        (
            window.find("status-failure-value").bounds(),
            window.find("status-strip").bounds(),
            ["status-device", "status-projection"]
                .iter()
                .filter(|id| window.try_find(**id).is_some())
                .count(),
        )
    });
    assert!(
        value.right() <= strip.right() && value.left() >= strip.left(),
        "the error value ends inside the strip at {width} px ({locale}): {value:?} in {strip:?}"
    );
    assert!(value.size.width > gpui_kit::px(200.), "the error has room: {value:?}");
    assert_eq!(neutral, 0, "neutral chips make room for the error at {width} px ({locale})");
    // The whole failure stays in the label even where the chip is cut.
    assert_eq!(shown.as_deref(), Some(full.as_str()), "{locale} {width}");
    assert!(
        strip.left() >= gpui_kit::px(0.) && strip.right() <= window_width,
        "the banner stays inside the window at {width} px ({locale}): {strip:?}"
    );
}

#[gpui_kit::test]
fn a_long_failure_stays_readable_at_1100_px_in_both_languages(cx: &mut TestAppContext) {
    let _guard = locale_guard();
    check(cx, "en", 1_100.);
    check(cx, "zh-CN", 1_100.);
    cx.update(|cx| i18n::select_locale("en", cx));
}

#[gpui_kit::test]
fn the_error_keeps_its_room_at_desktop_widths_in_both_languages(cx: &mut TestAppContext) {
    let _guard = locale_guard();
    for width in [900., 1_280.] {
        check(cx, "en", width);
        check(cx, "zh-CN", width);
    }
    cx.update(|cx| i18n::select_locale("en", cx));
}

#[gpui_kit::test]
fn a_long_failure_stays_inside_the_window_at_the_minimum_size_in_both_languages(
    cx: &mut TestAppContext,
) {
    let _guard = locale_guard();
    check(cx, "en", ramflux_desktop_ui::MIN_WINDOW_SIZE.0);
    check(cx, "zh-CN", ramflux_desktop_ui::MIN_WINDOW_SIZE.0);
    cx.update(|cx| i18n::select_locale("en", cx));
}
