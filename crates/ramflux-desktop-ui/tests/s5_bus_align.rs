// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Production root over the fake daemon: local readiness and networking titles.
mod common;
use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
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
    fixture_with_calls(cx, locked, authority_blocked, false)
}

#[derive(Default)]
struct MediaLatch {
    entered: AtomicBool,
    released: std::sync::Mutex<bool>,
    changed: std::sync::Condvar,
}
impl MediaLatch {
    fn wait(&self) {
        self.entered.store(true, Ordering::SeqCst);
        let mut released = self.released.lock().expect("media latch");
        while !*released {
            released = self.changed.wait(released).expect("media latch");
        }
    }
    fn release(&self) {
        *self.released.lock().expect("media latch") = true;
        self.changed.notify_all();
    }
}

fn fixture_with_calls(
    cx: &mut TestAppContext,
    locked: bool,
    authority_blocked: bool,
    call_api: bool,
) -> (FakeBus, common::Opened) {
    fixture_with_media(cx, locked, authority_blocked, call_api, None, false)
}

fn fixture_with_media(
    cx: &mut TestAppContext,
    locked: bool,
    authority_blocked: bool,
    call_api: bool,
    media: Option<Arc<MediaLatch>>,
    background: bool,
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
            "call.media.session" if media.is_some() => {
                media.as_ref().expect("media latch").wait();
                Reply::Error("ValidationFailed", "media-latch".into())
            }
            "account.list" if background => Reply::Ok(
                json!({"accounts":["acct-main","background"],"account_details":[{"id":"acct-main","state":"unlocked","networking":"live"},{"id":"background","state":"unlocked","networking":"live"}]}),
            ),
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
    config.call_api = call_api;
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
fn without_time(cx: &mut TestAppContext, done: impl FnMut(&mut TestAppContext) -> bool) {
    without_time_named(cx, "completion", done);
}

fn without_time_named(
    cx: &mut TestAppContext,
    name: &str,
    mut done: impl FnMut(&mut TestAppContext) -> bool,
) {
    let limit = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !done(cx) {
        assert!(std::time::Instant::now() < limit, "{name} did not wake UI");
        std::thread::sleep(std::time::Duration::from_millis(2));
        cx.run_until_parked();
    }
}

/// Registers a fresh waiter: consumes any stale notification, and returns the
/// waiter whose poll answered pending. That poll is the registration, so a
/// notification arriving later is delivered to it and none can slip between a
/// consuming poll and a separate pending assertion. `late(n)` runs just
/// before poll `n` (test hook for the interleaving).
fn register_waiter<F: std::future::Future<Output = ()>>(
    fresh: impl FnMut() -> F,
    context: &mut std::task::Context<'_>,
    late: &mut dyn FnMut(u32),
) -> std::pin::Pin<Box<F>> {
    register_waiter_within(fresh, context, late, std::time::Duration::from_secs(10))
}

/// `register_waiter` with a diagnostic deadline: a source that is ready on
/// every poll fails the test in bounded time instead of spinning outside the
/// hang guards. Pending stays the only success condition.
fn register_waiter_within<F: std::future::Future<Output = ()>>(
    mut fresh: impl FnMut() -> F,
    context: &mut std::task::Context<'_>,
    late: &mut dyn FnMut(u32),
    limit: std::time::Duration,
) -> std::pin::Pin<Box<F>> {
    let deadline = std::time::Instant::now() + limit;
    for attempt in 0.. {
        let mut waiter = Box::pin(fresh());
        late(attempt);
        if waiter.as_mut().poll(context).is_pending() {
            return waiter;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "waiter did not reach pending: the source was ready on all {} polls",
            attempt + 1
        );
    }
    unreachable!("the loop returns on the first pending poll")
}

struct CountedWake(std::sync::atomic::AtomicUsize);
impl std::task::Wake for CountedWake {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// The interleaving behind the CI failure: one stale notification is consumed
/// by the first poll, and a late one lands before the next poll. The
/// registration still completes with a pending waiter that a later send wakes.
#[test]
fn waiter_registration_survives_late_notification() {
    use ramflux_desktop_core::source::SourceWake;
    let wake = SourceWake::default();
    wake.notify(); // stale: consumed by the first poll
    let counter = Arc::new(CountedWake(std::sync::atomic::AtomicUsize::new(0)));
    let waker = std::task::Waker::from(counter.clone());
    let mut context = std::task::Context::from_waker(&waker);
    let mut pending = register_waiter(
        || wake.notified(),
        &mut context,
        // Before poll 1 (the one after the reset) a hub thread notifies again.
        &mut |attempt| {
            if attempt == 1 {
                wake.notify();
            }
        },
    );
    assert!(pending.as_mut().poll(&mut context).is_pending());
    wake.notify();
    assert_eq!(counter.0.load(Ordering::SeqCst), 1, "the registered waiter is woken once");
}

/// A source that is ready on every poll cannot spin the registration forever:
/// it fails with a diagnostic once the deadline passes.
#[test]
#[should_panic(expected = "waiter did not reach pending")]
fn registration_fails_in_bounded_time_on_an_always_ready_source() {
    use ramflux_desktop_core::source::SourceWake;
    let wake = SourceWake::default();
    let counter = Arc::new(CountedWake(std::sync::atomic::AtomicUsize::new(0)));
    let waker = std::task::Waker::from(counter);
    let mut context = std::task::Context::from_waker(&waker);
    let started = std::time::Instant::now();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        register_waiter_within(
            || wake.notified(),
            &mut context,
            &mut |_| wake.notify(), // a notification before every poll
            std::time::Duration::from_millis(200),
        )
    }));
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "failure is bounded");
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

/// The previous shape (single reset, then assert pending) loses to the same
/// interleaving, which is why `register_waiter` loops.
#[test]
#[should_panic(expected = "is_pending")]
fn single_reset_registration_loses_to_late_notification() {
    use ramflux_desktop_core::source::SourceWake;
    let wake = SourceWake::default();
    let counter = Arc::new(CountedWake(std::sync::atomic::AtomicUsize::new(0)));
    let waker = std::task::Waker::from(counter);
    let mut context = std::task::Context::from_waker(&waker);
    wake.notify();
    let mut pending = std::pin::pin!(wake.notified());
    if pending.as_mut().poll(&mut context).is_ready() {
        pending.set(wake.notified());
    }
    wake.notify(); // the late notification, exactly between reset and the poll
    assert!(pending.as_mut().poll(&mut context).is_pending());
}

#[gpui_kit::test]
fn ui_wakes_on_bridge_event_within_one_frame(cx: &mut TestAppContext) {
    use ramflux_desktop_core::{CallId, Source, Timestamp};
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
    let media_latch = Arc::new(MediaLatch::default());
    let (fake, (handle, shell)) =
        fixture_with_media(cx, false, false, false, Some(media_latch.clone()), false);
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
            .any(|e| matches!(&e.event, DomainEvent::CapabilitiesUpdated(c) if c.unsupported_reason("call.invite").is_none()))
    });
    background.drain(Timestamp(0));
    // Late attach notifications may still arrive from the hub threads.
    drop(register_waiter(|| wake.notified(), &mut context, &mut |_| {}));
    background.request_call_media(&CallId::new("background-call"));
    without_time(cx, |_| media_latch.entered.load(Ordering::SeqCst));
    // Attach may have consumed the earlier waiter. Re-register under the
    // reply latch; never clear the counter after registering this waiter.
    counter.0.store(0, Ordering::SeqCst);
    let mut pending = register_waiter(|| wake.notified(), &mut context, &mut |_| {});
    media_latch.release();
    let mut media_answer = None;
    without_time_named(cx, "latched media completion", |_| {
        if media_answer.is_none() {
            media_answer = background.take_call_media();
        }
        media_answer.is_some() && counter.0.load(Ordering::SeqCst) > 0
    });
    assert!(matches!(pending.as_mut().poll(&mut context), Poll::Ready(())));
    assert_eq!(media_answer.expect("media-only answer").0, CallId::new("background-call"));
    assert!(background.take_call_media().is_none());

    // A send before waiter registration must remain ready too.
    wake.notify();
    assert!(pin!(wake.notified()).as_mut().poll(&mut context).is_ready());

    late_phrase_handoff(cx);
    fake.shutdown();
}

#[gpui_kit::test]
fn visible_render_clock_stops_and_restarts(cx: &mut TestAppContext) {
    use gpui_kit::WindowVisibility;
    use ramflux_desktop_core::{CallId, Intent, IntentKind, methods};
    let _locale = locale_guard();
    let (fake, (handle, shell)) = fixture_with_calls(cx, false, false, true);
    wait_for_workspace(cx, &shell);
    wait(cx, "stream", |_| fake.streaming("acct-main"));
    let store = store_handle(cx, &shell);
    let ticking = |cx: &mut TestAppContext| cx.update(|cx| store.read(cx).render_clock_running());
    assert!(!ticking(cx), "settled idle has no render clock");
    let idle_time = cx.update(|cx| store.read(cx).session().expect("session").clock_ms());
    advance(cx, 10_000);
    assert_eq!(idle_time, cx.update(|cx| store.read(cx).session().expect("session").clock_ms()));

    let calls = ramflux_desktop_test::fake_calls::FakeCalls::new("127.0.0.1:1");
    calls.conversation("dm-ana", "principal-ana");
    let Reply::Ok(row) = calls
        .handle("call.invite", Some("acct-main"), &json!({"conversation_id":"dm-ana"}))
        .expect("invite")
    else {
        panic!("invite")
    };
    let id = row["call_id"].as_str().expect("id").to_owned();
    calls.peer_answers(&id);
    fake.publish("acct-main", "call.changed", calls.row(&id).expect("row"));
    without_time(cx, |cx| with_store(cx, &shell, |s| s.call(&CallId::new(&id)).is_some()));
    assert!(ticking(cx), "visible call starts time rendering");
    advance(cx, 1_000);
    let visible_time = cx.update(|cx| store.read(cx).session().expect("session").clock_ms());
    assert!(visible_time > idle_time);
    cx.simulate_window_visibility_change(handle.into(), WindowVisibility::Hidden);
    cx.run_until_parked();
    assert!(!ticking(cx), "occluded call stops rendering");
    advance(cx, 2_000);
    assert_eq!(visible_time, cx.update(|cx| store.read(cx).session().expect("session").clock_ms()));
    cx.simulate_window_visibility_change(handle.into(), WindowVisibility::Visible);
    cx.run_until_parked();
    assert!(ticking(cx), "visible call restarts rendering");
    advance(cx, 1_000);
    assert!(cx.update(|cx| store.read(cx).session().expect("session").clock_ms()) > visible_time);
    calls.peer_hangs_up(&id);
    fake.publish("acct-main", "call.changed", calls.row(&id).expect("ended"));
    without_time(cx, |cx| !ticking(cx));

    // A retry hint is rendered from a monotonic deadline, not a repeated
    // copy of the original five-second duration.
    fake.publish(
        "acct-main",
        "account.networking.changed",
        event(json!({"kind":"node_busy","retry_after_ms":5000}), "networking_pending"),
    );
    without_time(cx, |cx| ticking(cx));
    advance(cx, 1_000);
    assert_eq!(
        cx.update(|cx| store
            .read(cx)
            .session()
            .expect("session")
            .networking_retry_remaining(&AccountId::new("acct-main"))),
        Some(4000)
    );
    advance(cx, 4_000);
    assert!(!ticking(cx), "expired render-only hint leaves idle stopped");
    fake.shutdown();

    // A live creation deadline survives occlusion and a stopped render clock.
    let path = socket_path("s5-hidden-logical");
    let custody = Arc::new(ramflux_desktop_test::fake_custody::FakeCustody::new());
    let expiry_fake = FakeBus::start_with(
        &path,
        |method, _, _| match method {
            "daemon.status" => Reply::Ok(json!({"born_v3_creation":true})),
            "account.list" => Reply::Ok(json!({"accounts":[],"account_details":[]})),
            _ => Reply::Ok(json!({})),
        },
        custody,
    );
    let mut options = ShellOptions::for_scenario("", true);
    options.source = SourceChoice::Rfd(SdkConfig::new(path));
    let (expiry_window, expiry_shell) = open_with(cx, options);
    let expiry_store = store_handle(cx, &expiry_shell);
    wait(cx, "creation capability", |cx| {
        with_store(cx, &expiry_shell, |s| s.capabilities().born_v3_creation)
    });
    cx.update(|cx| {
        expiry_store.update(cx, |store, cx| {
            store.dispatch(Intent::human(IntentKind::BeginCeremony { label: "Alex".into() }), cx);
        })
    });
    without_time(cx, |cx| {
        with_store(cx, &expiry_shell, |s| s.ceremony().is_some_and(|c| c.is_active()))
    });
    cx.simulate_window_visibility_change(expiry_window.into(), WindowVisibility::Hidden);
    cx.run_until_parked();
    assert!(!cx.update(|cx| expiry_store.read(cx).render_clock_running()));
    let remaining = cx.update(|cx| {
        let session = expiry_store.read(cx).session().expect("daemon");
        session.store().ceremony().expect("ceremony").expires_at.saturating_since(session.now())
    });
    advance(cx, remaining + 1);
    assert!(!cx.update(|cx| expiry_store.read(cx).render_clock_running()));
    assert!(
        cx.update(|cx| expiry_store.read(cx).session().expect("daemon").now())
            >= with_store(cx, &expiry_shell, |s| s.ceremony().expect("ceremony").expires_at)
    );
    // The view's expiry handling has run without any visible rendering tick.
    assert!(
        cx.update(|cx| expiry_shell.read(cx).entry().read(cx).ceremony().read(cx).expired_seen())
    );
    expiry_fake.shutdown();

    // Hold progress uses its own visible gesture timer. Hiding cancels the
    // gesture; showing lets a new hold complete normally.
    let (hold_window, hold_shell) = open(cx, "s03_intervention");
    advance(cx, 1_000);
    click_in(cx, hold_window, "moment-m-approve", "approve");
    in_window(cx, hold_window, |window, cx| window.key_down("space", false, cx));
    advance(cx, 400);
    let strip = cx.update(|cx| hold_shell.read(cx).root().read(cx).confirm_strip().clone());
    assert!(cx.update(|cx| strip.read(cx).progress()) > 0.1);
    cx.simulate_window_visibility_change(hold_window.into(), WindowVisibility::Hidden);
    cx.run_until_parked();
    advance(cx, 2_000);
    assert_eq!(cx.update(|cx| strip.read(cx).progress()), 0.0);
    assert!(effects(cx, &hold_shell).is_empty());
    cx.simulate_window_visibility_change(hold_window.into(), WindowVisibility::Visible);
    cx.run_until_parked();
    hold_confirm(cx, hold_window);
    assert_eq!(effects(cx, &hold_shell), [methods::GRANT_APPROVE]);
}

fn late_phrase_handoff(cx: &mut TestAppContext) {
    use ramflux_desktop_core::{CeremonyPhase, Intent, IntentKind};
    let path = socket_path("s5-late-phrase");
    let custody = Arc::new(ramflux_desktop_test::fake_custody::FakeCustody::new());
    let fake = FakeBus::start_with(
        &path,
        |method, _, _| {
            Reply::Ok(match method {
                "daemon.status" => json!({"born_v3_creation":true}),
                "account.list" => json!({"accounts":[],"account_details":[]}),
                _ => json!({}),
            })
        },
        custody,
    );
    let gate = Arc::new(ramflux_desktop_sdk::test_support::PhraseHandoffGate::default());
    let mut config = SdkConfig::new(path);
    config.phrase_handoff_gate = Some(gate.clone());
    let hub = ramflux_desktop_sdk::SdkHub::start(config.clone()).expect("phrase hub");
    let mut options = ShellOptions::for_scenario("", false);
    options.source = SourceChoice::Rfd(config);
    let (_window, shell) = open_with(cx, options);
    let store = store_handle(cx, &shell);
    cx.update(|cx| store.update(cx, |s, cx| s.attach_live(hub.clone(), cx)));
    without_time(cx, |cx| with_store(cx, &shell, |s| s.capabilities().born_v3_creation));
    cx.update(|cx| {
        store.update(cx, |s, cx| {
            s.stop_clock();
            s.dispatch(Intent::human(IntentKind::BeginCeremony { label: "Alex".into() }), cx);
        })
    });
    without_time(cx, |cx| {
        with_store(cx, &shell, |s| s.ceremony().is_some_and(|c| c.phase == CeremonyPhase::Prepared))
    });
    cx.update(|cx| {
        store.update(cx, |s, cx| {
            s.dispatch(Intent::human(IntentKind::ShowRecoveryPhrase), cx);
        })
    });
    without_time(cx, |cx| {
        gate.waiting()
            && with_store(cx, &shell, |s| {
                s.ceremony().is_some_and(|c| c.phase == CeremonyPhase::PhraseReady)
            })
    });
    let panel = cx.update(|cx| shell.read(cx).entry().read(cx).ceremony().clone());
    assert_eq!(cx.update(|cx| panel.read(cx).held().words()), 0);
    let before = cx.executor().now();
    gate.release();
    without_time(cx, |cx| {
        cx.update(|cx| panel.read(cx).held().words()) == ramflux_desktop_core::PHRASE_WORDS
    });
    assert_eq!(hub.phrase_audit().taken(), 1);
    // An unrelated repaint may revisit PhraseReady but never takes again.
    cx.update(|cx| cx.refresh_windows());
    cx.run_until_parked();
    assert_eq!(hub.phrase_audit().taken(), 1);
    assert_eq!(before, cx.executor().now());
    assert!(!cx.update(|cx| store.read(cx).render_clock_running()));
    fake.shutdown();
}

#[gpui_kit::test]
fn background_account_media_drains_without_render_ticks(cx: &mut TestAppContext) {
    use ramflux_desktop_core::CallId;
    let _locale = locale_guard();
    let latch = Arc::new(MediaLatch::default());
    let (fake, (_window, shell)) =
        fixture_with_media(cx, false, false, true, Some(latch.clone()), true);
    wait_for_workspace(cx, &shell);
    let store = store_handle(cx, &shell);
    without_time(cx, |cx| {
        cx.update(|cx| store.read(cx).accounts().count()) == 2 && fake.streaming("acct-main")
    });
    cx.update(|cx| store.update(cx, |s, _| s.stop_clock()));
    let calls = ramflux_desktop_test::fake_calls::FakeCalls::new("127.0.0.1:1");
    calls.conversation("dm-ana", "principal-ana");
    let Reply::Ok(row) = calls
        .handle("call.invite", Some("acct-main"), &json!({"conversation_id":"dm-ana"}))
        .expect("invite")
    else {
        panic!("invite")
    };
    let id = row["call_id"].as_str().expect("id").to_owned();
    calls.peer_answers(&id);
    fake.publish("acct-main", "call.changed", calls.row(&id).expect("row"));
    without_time(cx, |_| latch.entered.load(Ordering::SeqCst));
    cx.update(|cx| {
        store.update(cx, |s, cx| s.set_active_account(&AccountId::new("background"), cx))
    });
    let before = cx.executor().now();
    latch.release();
    without_time(cx, |cx| {
        cx.update(|cx| {
            store.read(cx).call_audio().is_some_and(|a| {
                a.call_id == CallId::new(&id) && a.error.as_deref() == Some("media-latch")
            })
        })
    });
    assert_eq!(
        cx.update(|cx| store.read(cx).active_account().cloned()),
        Some(AccountId::new("background"))
    );
    assert_eq!(before, cx.executor().now());
    assert!(!cx.update(|cx| store.read(cx).render_clock_running()));
    fake.shutdown();
}

#[gpui_kit::test]
fn hidden_scenario_playback_stops_render_ticks(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (window, shell) = open(cx, "s02_daily");
    let store = store_handle(cx, &shell);
    advance(cx, 100);
    cx.simulate_window_visibility_change(window.into(), gpui_kit::WindowVisibility::Hidden);
    cx.run_until_parked();
    assert!(!cx.update(|cx| store.read(cx).render_clock_running()));
    let before = cx.update(|cx| store.read(cx).session().expect("session").clock_ms());
    advance(cx, 500);
    assert_eq!(before, cx.update(|cx| store.read(cx).session().expect("session").clock_ms()));
}

#[gpui_kit::test]
fn hidden_waiting_send_expires_without_render_ticks(cx: &mut TestAppContext) {
    use gpui_kit::WindowVisibility;
    use ramflux_desktop_core::{Intent, IntentKind, MomentKind, Receipt};
    use ramflux_desktop_sdk::SdkHub;
    use std::time::Duration;
    let _locale = locale_guard();
    let path = socket_path("s5-hidden-send");
    let frames: std::collections::BTreeMap<String, Value> = serde_json::from_str(include_str!(
        "../../ramflux-desktop-sdk/tests/fixtures/rfd_frames.json"
    ))
    .expect("frames");
    let fake = FakeBus::start(&path, move |method, _, _| {
        if method == "message.submit" {
            return Reply::Refused {
                code: "NetworkingPending",
                message: "arbitrary wording".into(),
                reason_code: "pending",
                details: Some(json!({"networking":{"state":"networking_pending"}})),
            };
        }
        Reply::Ok(frames.get(method).cloned().unwrap_or(json!({})))
    });
    fake.enable_stream();
    let mut config = SdkConfig::new(path);
    config.call_api = false;
    config.backup_api = false;
    config.pause_test_time = true;
    let hub = SdkHub::start(config.clone()).expect("hub");
    let mut options = ShellOptions::for_scenario("", true);
    options.source = SourceChoice::Rfd(config);
    let (window, shell) = open_with(cx, options);
    let store = store_handle(cx, &shell);
    cx.update(|cx| store.update(cx, |store, cx| store.attach_live(hub.clone(), cx)));
    wait_for_workspace(cx, &shell);
    show_context(cx, window, &shell, "dm-ana");
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.dispatch(
                Intent::human(IntentKind::SendMessage {
                    context_id: ramflux_desktop_core::ContextId::new("dm-ana"),
                    text: "Hidden expiry".into(),
                }),
                cx,
            );
        })
    });
    let has = |cx: &mut TestAppContext, wanted: Receipt| {
        with_store(cx, &shell, |s| {
            s.moments().any(|m| matches!(&m.kind, MomentKind::Message(p) if p.receipt == wanted))
        })
    };
    without_time(cx, |cx| {
        with_store(cx, &shell, |s| {
            s.moments().any(|m| matches!(&m.kind, MomentKind::Message(p) if matches!(p.receipt, Receipt::WaitingForNode { .. })))
        })
    });
    cx.simulate_window_visibility_change(window.into(), WindowVisibility::Hidden);
    cx.run_until_parked();
    assert!(!cx.update(|cx| store.read(cx).render_clock_running()));
    let elapsed = cx.update(|cx| store.read(cx).session().expect("session").clock_ms());
    hub.advance_test_time(Duration::from_secs(599));
    without_time(cx, |cx| {
        fake.seen().iter().filter(|r| r.method == "message.submit").count() >= 2
            && with_store(cx, &shell, |s| {
                s.moments().any(|m| matches!(&m.kind, MomentKind::Message(p) if matches!(p.receipt, Receipt::WaitingForNode { .. })))
            })
    });
    assert!(!has(cx, Receipt::Failed { retryable: true }), "not before ten minutes");
    hub.advance_test_time(Duration::from_secs(1));
    without_time(cx, |cx| has(cx, Receipt::Failed { retryable: true }));
    assert_eq!(
        elapsed,
        cx.update(|cx| store.read(cx).session().expect("session").clock_ms()),
        "no render ticks drove expiry"
    );
    assert!(!cx.update(|cx| store.read(cx).render_clock_running()));
    cx.simulate_window_visibility_change(window.into(), WindowVisibility::Visible);
    cx.run_until_parked();
    assert!(exists(cx, window, "message-retry"));
    let failed = with_store(cx, &shell, |s| {
        s.moments().find(|m| matches!(&m.kind, MomentKind::Message(p) if p.receipt == (Receipt::Failed { retryable: true }))).expect("failed moment").clone()
    });
    assert_eq!(failed.lifecycle, ramflux_desktop_core::Lifecycle::Failed);
    in_window(cx, window, |window, _| {
        let label =
            window.find(format!("moment-{}", failed.id)).label().expect("row label").to_owned();
        assert!(label.contains(tr("lifecycle.failed").as_ref()));
        assert!(!label.contains(tr("lifecycle.in_progress").as_ref()));
        assert!(
            window.find("message-retry").bounds().size.width < gpui_kit::px(160.),
            "Retry stays compact inside the receipt row"
        );
    });
    fake.shutdown();
}

#[gpui_kit::test]
fn sent_text_has_done_lifecycle_and_no_in_progress_label(cx: &mut TestAppContext) {
    use ramflux_desktop_core::{Intent, IntentKind, Lifecycle, MomentKind, Receipt};
    let _locale = locale_guard();
    let (fake, (window, shell)) = fixture(cx, false);
    wait_for_workspace(cx, &shell);
    show_context(cx, window, &shell, "dm-ana");
    let store = store_handle(cx, &shell);
    cx.update(|cx| {
        store.update(cx, |store, cx| {
            store.dispatch(
                Intent::human(IntentKind::SendMessage {
                    context_id: ramflux_desktop_core::ContextId::new("dm-ana"),
                    text: "Completed text".into(),
                }),
                cx,
            );
        })
    });
    without_time(cx, |cx| {
        with_store(cx, &shell, |s| {
            s.moments().any(|m| {
                s.body(&m.id) == Some("Completed text")
                    && matches!(&m.kind, MomentKind::Message(p) if p.receipt == Receipt::Sent)
            })
        })
    });
    let sent = with_store(cx, &shell, |s| {
        s.moments().find(|m| s.body(&m.id) == Some("Completed text")).expect("sent").clone()
    });
    assert_eq!(sent.lifecycle, Lifecycle::Done);
    in_window(cx, window, |window, _| {
        let label =
            window.find(format!("moment-{}", sent.id)).label().expect("row label").to_owned();
        assert!(label.contains(tr("lifecycle.done").as_ref()));
        assert!(!label.contains(tr("lifecycle.in_progress").as_ref()));
    });
    fake.shutdown();
}

#[gpui_kit::test]
fn attachment_fallback_and_paused_state_are_localized(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    for locale in ["en", "zh-CN"] {
        cx.update(|cx| select_locale(locale, cx));
        let (fake, (window, shell)) = fixture(cx, false);
        wait_for_workspace(cx, &shell);
        show_context(cx, window, &shell, "dm-ana");
        wait(cx, "paused attachment row", |cx| {
            with_store(cx, &shell, |s| {
                s.object(&ramflux_desktop_core::ObjectId::new("attachment:m2:0")).is_some()
            }) && with_store(cx, &shell, |s| {
                s.moments().any(|m| {
                    matches!(&m.kind,
                ramflux_desktop_core::MomentKind::Message(p)
                if p.attachment.as_ref().is_some_and(|id| id.as_str() == "attachment:m2:0"))
                })
            })
        });
        let scope = with_store(cx, &shell, |s| {
            let moment = s
                .moments()
                .find(|m| {
                    matches!(&m.kind,
                ramflux_desktop_core::MomentKind::Message(p)
                if p.attachment.as_ref().is_some_and(|id| id.as_str() == "attachment:m2:0"))
                })
                .expect("attachment moment");
            format!("moment-{}", moment.id)
        });
        let label = in_window(cx, window, |window, _| {
            window
                .within(scope)
                .find("message-attachment")
                .label()
                .expect("attachment label")
                .to_owned()
        });
        assert!(label.contains(tr("moment.message.attachment").as_ref()), "{locale}: {label}");
        assert!(label.contains(tr("object.paused").as_ref()), "{locale}: {label}");
        assert!(!label.contains("attachment:m2:0"), "{locale}: {label}");
        assert!(!label.contains("paused"), "{locale}: {label}");
        fake.shutdown();
    }
    cx.update(|cx| select_locale("en", cx));
}
