// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Pinned contexts persist in the desktop settings file and come back, in
//! pin order, after a restart.

mod common;

use common::*;
use ramflux_desktop_core::{AccountId, ContextId, Effect, Source as _, methods};
use ramflux_desktop_sdk::SdkHub;
use ramflux_desktop_sdk::pins::PinStore;
use serde_json::json;

fn pin(method: &str, context: &str) -> Effect {
    Effect::Call { method: method.to_owned(), body: json!({ "context_id": context }) }
}

#[test]
fn pins_persist_across_a_restart_in_pin_order() {
    let path = socket_path("pins");
    let dir = std::env::temp_dir().join(format!("rf-pins-it-{}", std::process::id()));
    let settings = dir.join("desktop-settings.json");
    let account = AccountId::new("acct-main");
    let fake = FakeBus::start(&path, |method, _, _| fixture(method));
    let start = || {
        SdkHub::start(ramflux_desktop_sdk::SdkConfig {
            settings_path: Some(settings.clone()),
            ..config(&path)
        })
        .expect("bridge")
    };

    let hub = start();
    let mut first = Fed::new(hub.account_source(account.clone()));
    first.until("daemon pin adopted", |s, _| s.pin_order() == [ContextId::new("dm-ana")]);
    first.source.submit(pin(methods::CONTEXT_PIN, "g-team"));
    first.source.submit(pin(methods::CONTEXT_UNPIN, "dm-ana"));
    first.until("pins changed", |s, _| s.pin_order() == [ContextId::new("g-team")]);
    let store = PinStore::new(Some(settings.clone()));
    assert_eq!(store.load(&account), Some(vec![ContextId::new("g-team")]));
    drop(first);
    drop(hub);

    let hub = start();
    let mut second = Fed::new(hub.account_source(account.clone()));
    second.until("restored", |s, _| s.pin_order() == [ContextId::new("g-team")]);
    second.settle(300);
    assert_eq!(
        second.store.pin_order(),
        [ContextId::new("g-team")],
        "the desktop's own record wins over the daemon's pin"
    );
    second.source.submit(pin(methods::CONTEXT_PIN, "dm-ana"));
    second.until("pinned again", |s, _| s.pin_order().len() == 2);
    assert_eq!(
        store.load(&account),
        Some(vec![ContextId::new("g-team"), ContextId::new("dm-ana")]),
        "pin order is kept"
    );
    fake.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}
