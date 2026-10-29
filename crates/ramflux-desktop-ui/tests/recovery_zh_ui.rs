// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The recovery screens in zh-CN. The interface language is process-wide, so
//! this test has a binary of its own: next to the others (which run on
//! parallel threads and compare what is painted with `tr(...)`) it would
//! change their language between a render and an assertion.

mod common;

use common::*;
use gpui_kit::{AppContext as _, TestAppContext};
use ramflux_desktop_core::{
    Account, AccountId, AccountKind, DomainEvent, LockState, Networking, SecureSessions,
};
use ramflux_desktop_test::mock_recovery::DEMO_ANCHOR;
use ramflux_desktop_ui::i18n::{tr, tr_args};

const PASSPHRASE: &str = "river stone lantern";

#[gpui_kit::test]
fn the_screens_speak_chinese_when_the_interface_does(cx: &mut TestAppContext) {
    let _guard = locale_guard();
    let (handle, shell) = open(cx, "s13_born_v3");
    cx.update_window(handle.into(), |_, window, _| window.activate_window()).expect("window");
    cx.run_until_parked();
    advance(cx, 2_500);
    let store = store_handle(cx, &shell);
    cx.update(|cx| {
        store.update(cx, |store, _| {
            let mock = store.mock_mut().expect("a mock");
            mock.ceremony_mut().recovery_mut().allow("acct-r", "Alex", PASSPHRASE);
            mock.inject(
                0,
                DomainEvent::AccountUpdated(Account {
                    id: AccountId::new("acct-r"),
                    label: "Alex".to_owned(),
                    kind: AccountKind::HumanInteractive,
                    node_id: None,
                    device_id: None,
                    lock_state: LockState::Locked,
                    self_participant_id: None,
                    secure_sessions: SecureSessions::Ready,
                    networking: Networking::Live,
                }),
            );
        });
    });
    advance(cx, 300);
    cx.update(|cx| ramflux_desktop_ui::i18n::select_locale("zh-CN", cx));
    advance(cx, 100);
    click(cx, handle, "account-recover-acct-r");
    let intro = tr_args("recovery.intro.title", &[("name", "Alex")]).to_string();
    assert!(!intro.is_ascii(), "{intro}");
    assert!(has_label(cx, handle, &intro));
    assert!(has_label(cx, handle, &tr("recovery.intro.warning")));
    click(cx, handle, "recovery-continue");
    advance(cx, 300);
    let shown = tr_args("recovery.fingerprint.shown", &[("value", &grouped())]).to_string();
    assert!(!shown.is_ascii(), "{shown}");
    assert!(has_label(cx, handle, &shown));
    cx.update(|cx| ramflux_desktop_ui::i18n::select_locale("en", cx));
}

fn grouped() -> String {
    DEMO_ANCHOR
        .chars()
        .collect::<Vec<_>>()
        .chunks(4)
        .map(|g| g.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}
