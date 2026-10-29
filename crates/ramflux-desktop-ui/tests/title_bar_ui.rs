// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The title bar never loses its controls (DESK-PIN-03 L10): a long failure
//! in the status strip is cut with an ellipsis (its full text stays in the
//! accessibility label), and the palette, Needs-me, theme and settings
//! controls stay inside the window, down to the application's minimum size,
//! in English and in Chinese.

mod common;
mod sdk_fake;

use std::sync::Arc;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use ramflux_desktop_core::{Intent, IntentKind, RequestAnswer, selectors};
use ramflux_desktop_ui::i18n::{self, tr};
use ramflux_desktop_ui::route::Page;
use sdk_fake::wait;
use serde_json::json;

const CONTROLS: [&str; 4] = ["open-palette", "needs-me", "toggle-theme", "open-settings"];

/// Opens a `width` px wide window whose status strip shows the longest
/// refusal text (a failed trust check) and returns the strip's failure text.
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
    requests.fail_decisions(Some(("TransportSecurity", "node certificate pin mismatch")));
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
    let long = tr("backend.failure.transport_security").to_string();
    assert!(full.contains(&long), "the longest refusal text: {full}");
    let (shown, bar, controls) = in_window(cx, handle, |window, _| {
        let shown =
            window.within("status-failure").find("status-failure-value").label().map(str::to_owned);
        let bar = window.find("title-bar").bounds();
        let controls: Vec<_> =
            CONTROLS.iter().map(|id| (*id, window.try_find(*id).map(|e| e.bounds()))).collect();
        (shown, bar, controls)
    });
    // The whole failure stays in the label even where the chip is cut.
    assert_eq!(shown.as_deref(), Some(full.as_str()), "{locale} {width}");
    for (id, bounds) in controls {
        let bounds = bounds.unwrap_or_else(|| panic!("{id} is shown at {width} px ({locale})"));
        assert!(
            bounds.right() <= bar.right() && bounds.left() >= bar.left(),
            "{id} is inside the title bar at {width} px ({locale}): {bounds:?} in {bar:?}"
        );
    }
}

#[gpui_kit::test]
fn a_long_failure_leaves_the_controls_at_1100_px_in_both_languages(cx: &mut TestAppContext) {
    let _guard = locale_guard();
    check(cx, "en", 1_100.);
    check(cx, "zh-CN", 1_100.);
    cx.update(|cx| i18n::select_locale("en", cx));
}

#[gpui_kit::test]
fn a_long_failure_leaves_the_controls_at_the_minimum_size_in_both_languages(
    cx: &mut TestAppContext,
) {
    let _guard = locale_guard();
    check(cx, "en", ramflux_desktop_ui::MIN_WINDOW_SIZE.0);
    check(cx, "zh-CN", ramflux_desktop_ui::MIN_WINDOW_SIZE.0);
    cx.update(|cx| i18n::select_locale("en", cx));
}
