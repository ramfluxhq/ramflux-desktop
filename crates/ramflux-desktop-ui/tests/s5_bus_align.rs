// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Production root over the fake daemon: local readiness and networking titles.
mod common;
use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use ramflux_desktop_core::{AccountId, DomainEvent, EffectFailure, FailureCode, Networking};
use ramflux_desktop_sdk::SdkConfig;
use ramflux_desktop_test::fake_bus::{FakeBus, Reply, socket_path};
use ramflux_desktop_ui::{
    i18n::{select_locale, tr},
    shell::{ShellOptions, SourceChoice},
    workspace::PrefsStore,
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn fixture(cx: &mut TestAppContext, locked: bool) -> (FakeBus, common::Opened) {
    fixture_with_authority(cx, locked, false)
}

fn fixture_with_authority(
    cx: &mut TestAppContext,
    locked: bool,
    authority_blocked: bool,
) -> (FakeBus, common::Opened) {
    let path = socket_path("s5-ui");
    let frames: std::collections::BTreeMap<String, Value> = serde_json::from_str(include_str!(
        "../../ramflux-desktop-sdk/tests/fixtures/rfd_frames.json"
    ))
    .expect("fixture");
    let opened = Arc::new(AtomicBool::new(!locked));
    let fake = FakeBus::start(&path, move |method, _, _| {
        let unlocked = opened.load(Ordering::SeqCst);
        match method {
            "account.list" => {
                Reply::Ok(json!({"accounts":if unlocked {vec!["acct-main"]} else {vec![]},
                "account_details":[{"id":"acct-main","label":"Work","state":if unlocked {"unlocked"} else {"locked"},"networking":"networking_pending","authority_blocked":authority_blocked}]}))
            }
            "account.unlock" => {
                opened.store(true, Ordering::SeqCst);
                Reply::Ok(
                    json!({"local_account_id":"acct-main","state":"unlocked","networking":"networking_pending"}),
                )
            }
            "account.status" => {
                let mut status = frames.get(method).cloned().unwrap_or(json!({}));
                status["networking"] =
                    json!(if authority_blocked { "networking_pending" } else { "live" });
                status["authority_blocked"] = json!(authority_blocked);
                status["networking_snapshot"] = json!({"state":"networking_pending","phase":"connecting","generation":1,"cause":null,"retry_after_ms":null});
                Reply::Ok(status)
            }
            _ => Reply::Ok(frames.get(method).cloned().unwrap_or(json!({}))),
        }
    });
    fake.enable_stream();
    let mut config = SdkConfig::new(&path);
    config.call_api = false;
    config.backup_api = false;
    let options = ShellOptions {
        source: SourceChoice::Rfd(config),
        scenario: String::new(),
        start_clock: true,
        wall_clock: false,
        prefs: PrefsStore::in_memory(),
        launch: ramflux_desktop_ui::launch::LaunchOptions::in_memory(),
    };
    (fake, open_with(cx, options))
}

#[gpui_kit::test]
fn unlock_opens_account_on_networking_pending(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    cx.update(|cx| select_locale("en", cx));
    let (fake, (handle, shell)) = fixture(cx, true);
    wait(cx, "unlock form", |cx| exists(cx, handle, "unlock"));
    type_into(cx, handle, "passphrase", "river stone lantern");
    click(cx, handle, "unlock");
    wait_for_workspace(cx, &shell);
    wait(cx, "connecting header", |cx| has_label(cx, handle, &tr("network_header.connecting")));
    assert!(with_store(cx, &shell, |s| s
        .account(&AccountId::new("acct-main"))
        .is_some_and(|a| a.lock_state == ramflux_desktop_core::LockState::Unlocked
            && a.networking == Networking::Pending)));
    assert!(!exists(cx, handle, "networking-banner"));
    let unlocks: Vec<_> =
        fake.seen().into_iter().filter(|r| r.method == "account.unlock").collect();
    assert_eq!(unlocks.len(), 1);
    fake.shutdown();
}

fn title(cx: &mut TestAppContext, handle: gpui_kit::WindowHandle<gpui_kit::base::Root>) -> String {
    in_window(cx, handle, |window, _| {
        window.find("workspace-networking-title").label().expect("title label").to_owned()
    })
}

fn event(reason: Value, session: &str) -> Value {
    json!({"account_id":"acct-main", "state":"live", "node_binding":{"node_id":"node-one","key":"identity"},
        "networking_reason":reason, "session":{"state":session,"phase":"backoff","generation":2,
        "cause":{"kind":"node_unreachable","retry_after_ms":5000},"retry_after_ms":5000}})
}

#[gpui_kit::test]
fn header_shows_connecting_until_live_or_typed_reason(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (fake, (handle, shell)) = fixture(cx, false);
    wait_for_workspace(cx, &shell);
    wait(cx, "stream", |_| fake.streaming("acct-main"));
    for locale in ["en", "zh-CN"] {
        cx.update(|cx| select_locale(locale, cx));
        assert_eq!(
            tr("network_header.connecting").as_ref(),
            if locale == "en" { "Connecting…" } else { "\u{6b63}\u{5728}\u{8fde}\u{63a5}…" }
        );
        let mut cases = vec![
            (json!({"kind":"no_node_selected"}), "network_header.choose_node", false),
            (json!({"kind":"node_custody_unavailable"}), "network_header.choose_custody", false),
            (json!({"kind":"custody_expired"}), "network_header.renew", false),
            (
                json!({"kind":"node_busy","retry_after_ms":5000,"capacity":{"scope":"global_daily","resets_at_unix":9000}}),
                "network_header.busy",
                true,
            ),
            (
                json!({"kind":"node_unreachable","retry_after_ms":5000}),
                "network_header.unreachable",
                true,
            ),
        ];
        for (code, key, transient) in [
            ("busy", "busy", true),
            ("capacity", "capacity", true),
            ("persistence_unavailable", "persistence", true),
            ("session_required", "session_required", true),
            ("session_expired", "session_expired", true),
            ("unsupported", "update", false),
            ("update_required", "update", false),
            ("authority_unavailable", "authority", false),
            ("principal_id_taken", "identity", false),
            ("challenge_unknown", "restart", false),
            ("challenge_expired", "restart", false),
            ("replay", "restart", false),
            ("invalid_input", "refused", false),
            ("binding_refused", "refused", false),
        ] {
            let full_key = match key {
                "busy" => "network_header.busy",
                "capacity" => "network_header.capacity",
                "persistence" => "network_header.persistence",
                "session_required" => "network_header.session_required",
                "session_expired" => "network_header.session_expired",
                "update" => "network_header.update",
                "authority" => "network_header.authority",
                "identity" => "network_header.identity",
                "restart" => "network_header.restart",
                "refused" => "network_header.refused",
                _ => unreachable!(),
            };
            cases.push((json!({"kind":"registration_refused","code":code}), full_key, transient));
        }
        for (reason, key, transient) in cases {
            // A terminal custody cause must beat the simultaneous transient resident cause.
            let mut body = event(reason.clone(), "networking_pending");
            if transient {
                body["session"]["cause"] = reason.clone();
            }
            fake.publish("acct-main", "account.networking.changed", body);
            let expected_reason: ramflux_desktop_core::NetworkingReason =
                serde_json::from_value(reason.clone()).expect("typed reason");
            wait(cx, "reason applied", |cx| {
                with_store(cx, &shell, |s| {
                    s.account(&AccountId::new("acct-main"))
                        .and_then(|a| a.networking_reason.as_ref())
                        .is_some_and(|r| {
                            r.kind == expected_reason.kind && r.code == expected_reason.code
                        })
                })
            });
            let expected = tr(if transient { "network_header.connecting" } else { key });
            wait(cx, "rendered title", |cx| title(cx, handle) == expected.as_ref());
            if transient {
                let detail = in_window(cx, handle, |window, _| {
                    window.find("workspace-networking-detail").label().expect("detail").to_owned()
                });
                assert!(detail.contains(tr(key).as_ref()), "{locale}: {reason}: {detail}");
                assert!(detail.contains('5'), "retry hint retained");
                assert!(!detail.contains(['{', '"']) && !detail.contains("_daily"));
                if reason.get("capacity").is_some() {
                    assert!(detail.contains(tr("network_header.global_daily").as_ref()));
                    assert!(!detail.contains("9000"));
                }
            }
            assert!(!exists(cx, handle, "networking-banner"));
        }
        for (refusal, key) in [
            ("home_fork", "network_header.fork"),
            ("capacity_refused", "network_header.capacity"),
            ("commit_outcome_unknown", "backend.failure.reconcile"),
            ("lease_conflict", "network_header.migration"),
        ] {
            let migration = json!({"kind":"registration_refused", "code":{"home_migration":{"reason":refusal,"nack":{"code":"busy","detail":"lease"}}}});
            fake.publish(
                "acct-main",
                "account.networking.changed",
                event(migration, "networking_pending"),
            );
            wait(cx, "migration detail rendered", |cx| {
                let t = title(cx, handle);
                t == tr(key).as_ref() && !t.contains(['{', '"']) && !t.contains(refusal)
            });
        }
        let mut blocked = event(Value::Null, "live");
        blocked["state"] = json!("networking_pending");
        blocked["networking_reason"] =
            json!({"kind":"registration_refused","code":"authority_unavailable"});
        fake.publish("acct-main", "account.networking.changed", blocked);
        wait(cx, "authority beats live session", |cx| {
            title(cx, handle) == tr("network_header.authority").as_ref()
        });
        // No cause and a Live event restore the normal workspace title.
        let mut live = event(Value::Null, "live");
        live["session"]["cause"] = Value::Null;
        fake.publish("acct-main", "account.networking.changed", live);
        wait(cx, "live title", |cx| title(cx, handle) == tr("app.name").as_ref());
    }
    // Typed authority/reconciliation failures also outrank a transient session.
    let account = with_store(cx, &shell, |s| {
        s.account(&AccountId::new("acct-main")).expect("account").clone()
    });
    for (code, key) in [
        ("CustodyForkFenced", "network_header.fork"),
        ("AuthorityUnavailable", "network_header.authority"),
        ("OperationReconciliationRequired", "backend.failure.reconcile"),
    ] {
        let mut pending = account.clone();
        pending.networking = Networking::Pending;
        let mut scenario = ramflux_desktop_test::Scenario {
            name: "typed-title".into(),
            description: String::new(),
            epoch: ramflux_desktop_core::Timestamp(0),
            timeline: vec![
                (
                    0,
                    DomainEvent::SourceStatusChanged(ramflux_desktop_core::SourceStatus::Connected),
                ),
                (0, DomainEvent::AccountUpdated(pending)),
                (
                    0,
                    DomainEvent::AccountNetworkingFailed {
                        account_id: AccountId::new("acct-main"),
                        code: FailureCode::Daemon(code.into()),
                    },
                ),
            ],
            responses: vec![],
            companions: vec![],
        };
        // An unrelated failure on another account cannot replace this title.
        let mut other = account.clone();
        other.id = AccountId::new("acct-other");
        other.lock_state = ramflux_desktop_core::LockState::Locked;
        let more = ramflux_desktop_test::Scenario {
            name: "other-account-failure".into(),
            description: String::new(),
            epoch: ramflux_desktop_core::Timestamp(0),
            timeline: vec![
                (0, DomainEvent::AccountUpdated(other)),
                (
                    0,
                    DomainEvent::AccountNetworkingFailed {
                        account_id: AccountId::new("acct-other"),
                        code: FailureCode::Daemon("CustodyForkFenced".into()),
                    },
                ),
                (
                    0,
                    DomainEvent::EffectFailed(EffectFailure {
                        method: "context.list".into(),
                        code: FailureCode::Timeout,
                        message: "unrelated timeout".into(),
                    }),
                ),
            ],
            responses: vec![],
            companions: vec![],
        };
        scenario.timeline.extend(more.timeline);
        let store = store_handle(cx, &shell);
        cx.update(|cx| store.update(cx, |store, cx| store.load_scenario(&scenario, cx)));
        cx.run_until_parked();
        advance(cx, 100);
        assert_eq!(title(cx, handle), tr(key).as_ref());
    }
    cx.update(|cx| select_locale("en", cx));
    fake.shutdown();
}

#[gpui_kit::test]
fn authority_restored_event_returns_blocked_header_to_live(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    cx.update(|cx| select_locale("en", cx));
    let (fake, (handle, shell)) = fixture_with_authority(cx, false, true);
    wait_for_workspace(cx, &shell);
    wait(cx, "blocked status header and stream", |cx| {
        fake.streaming("acct-main")
            && title(cx, handle) == tr("network_header.authority").as_ref()
            && with_store(cx, &shell, |s| {
                s.account(&AccountId::new("acct-main")).is_some_and(|a| a.authority_blocked)
            })
    });
    let mut restored = event(Value::Null, "live");
    restored["session"]["cause"] = Value::Null;
    assert!(restored.get("authority_blocked").is_none(), "product event has no flag");
    fake.publish("acct-main", "account.networking.changed", restored);
    wait(cx, "authority event restores Live header", |cx| {
        title(cx, handle) == tr("app.name").as_ref()
            && with_store(cx, &shell, |s| {
                s.account(&AccountId::new("acct-main"))
                    .is_some_and(|a| !a.authority_blocked && a.networking == Networking::Live)
            })
    });
    fake.shutdown();
}

/// Runs bridge threads without advancing GPUI's clock. A delivery must be
/// applied by the completion waiter, rather than by the 50 ms render ticker.
fn without_time(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) {
    let limit = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !done(cx) {
        assert!(std::time::Instant::now() < limit, "completion did not wake UI");
        std::thread::sleep(std::time::Duration::from_millis(2));
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn ui_wakes_on_bridge_event_within_one_frame(cx: &mut TestAppContext) {
    use ramflux_desktop_core::{CallId, Source, Timestamp, methods};
    use ramflux_desktop_sdk::SdkHub;
    use std::{
        future::Future,
        pin::pin,
        task::{Context, Poll, Wake, Waker},
    };
    struct Count(std::sync::atomic::AtomicUsize);
    impl Wake for Count {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let _locale = locale_guard();
    let (fake, (handle, shell)) = fixture(cx, false);
    wait_for_workspace(cx, &shell);
    wait(cx, "stream", |_| fake.streaming("acct-main"));
    let store = store_handle(cx, &shell);
    cx.update(|cx| store.update(cx, |store, _| store.stop_clock()));
    let before = cx.executor().now();
    let mut live = event(Value::Null, "live");
    live["session"]["cause"] = Value::Null;
    fake.publish("acct-main", "account.networking.changed", live);
    without_time(cx, |cx| title(cx, handle) == tr("app.name").as_ref());
    assert_eq!(before, cx.executor().now(), "zero-time delivery renders in one drain");

    // A separate background scope shares the hub notification even if its
    // completion carries no domain event. Register both before and after send.
    let hub = SdkHub::start(SdkConfig::new(
        cx.update(|cx| store.read(cx).live_socket().expect("socket").to_owned()),
    ))
    .expect("hub");
    let mut background = hub.account_source(AccountId::new("background"));
    let wake = hub.wake();
    let counter = Arc::new(Count(std::sync::atomic::AtomicUsize::new(0)));
    let waker = Waker::from(counter.clone());
    let mut context = Context::from_waker(&waker);
    // Settle attach before isolating the media-only completion.
    without_time(cx, |_| {
        background
            .drain(Timestamp(0))
            .iter()
            .any(|e| matches!(e.event, DomainEvent::CapabilitiesUpdated(_)))
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    background.drain(Timestamp(0));
    let mut pending = pin!(wake.notified());
    if pending.as_mut().poll(&mut context).is_ready() {
        pending.set(wake.notified());
    }
    assert!(pending.as_mut().poll(&mut context).is_pending());
    background.request_call_media(&CallId::new("background-call"));
    without_time(cx, |_| counter.0.load(Ordering::SeqCst) > 0);
    assert!(matches!(pending.as_mut().poll(&mut context), Poll::Ready(())));
    assert_eq!(
        background.take_call_media().expect("media-only answer").0,
        CallId::new("background-call")
    );
    assert!(background.take_call_media().is_none());

    // A send before waiter registration must remain ready too.
    wake.notify();
    assert!(pin!(wake.notified()).as_mut().poll(&mut context).is_ready());

    // The daemon's phrase channel uses the same signal and keeps one-shot
    // ownership/wiping: a phrase never becomes a replayable domain event.
    let path = socket_path("s5-wake-phrase");
    let custody = Arc::new(ramflux_desktop_test::fake_custody::FakeCustody::new());
    let phrase_fake = FakeBus::start_with(
        &path,
        |method, _, _| match method {
            "daemon.status" => Reply::Ok(json!({"born_v3_creation":true})),
            "account.list" => Reply::Ok(json!({"accounts":[],"account_details":[]})),
            _ => Reply::Ok(json!({})),
        },
        custody,
    );
    let phrase_hub = SdkHub::start(SdkConfig::new(path)).expect("phrase hub");
    let mut daemon = ramflux_desktop_ui::app::AccountSession::live(
        "daemon",
        Box::new(phrase_hub.daemon_source()),
    );
    daemon.submit(ramflux_desktop_core::Effect::Call {
        method: methods::CUSTODY_BEGIN.into(),
        body: json!({"display_name":"Alex","principal_kind":"human"}),
    });
    without_time(cx, |_| {
        daemon.advance(0);
        daemon
            .store()
            .ceremony()
            .is_some_and(|c| c.phase == ramflux_desktop_core::CeremonyPhase::Prepared)
    });
    daemon.submit(ramflux_desktop_core::Effect::Call {
        method: methods::CUSTODY_SHOW_ONCE.into(),
        body: json!({}),
    });
    let mut phrase = None;
    without_time(cx, |_| {
        daemon.advance(0);
        phrase = daemon.take_recovery_phrase();
        phrase.is_some()
    });
    assert!(daemon.take_recovery_phrase().is_none(), "phrase moves once");
    assert_eq!(phrase_hub.phrase_audit().taken(), 1);
    drop(phrase);
    phrase_fake.shutdown();
    fake.shutdown();
}
