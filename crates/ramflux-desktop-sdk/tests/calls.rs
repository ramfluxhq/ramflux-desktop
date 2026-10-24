// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Voice calls through the SDK source and a fake daemon with the CALL-01
//! call API (contract-first): placing, answering, declining and hanging up
//! reach the daemon as `call.*` requests; `call.changed` events (stream or
//! push) drive the store's state machine (outgoing and incoming ringing,
//! connecting, active, ended with its reason, including timeout, busy and
//! answered elsewhere); the media session reaches the engine through the
//! source, never the store; and without the call API every call method is
//! declared unsupported with its reason.

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use common::*;
use ramflux_desktop_core::{
    AccountId, AttentionReason, Call, CallEndReason, CallId, CallMedia, CallOutcome, CallState,
    ContextId, EventEnvelope, FailureCode, Intent, IntentError, IntentKind, Lifecycle, MomentKind,
    Source, Store, SyncMode, Timestamp, handle_intent, methods, selectors,
};
use ramflux_desktop_sdk::SdkConfig;
use ramflux_desktop_test::fake_calls::FakeCalls;
use serde_json::{Value, json};

const ACCOUNT: &str = "acct-main";
const DM: &str = "dm-ana";

struct World {
    fake: Arc<FakeBus>,
    calls: Arc<FakeCalls>,
    path: PathBuf,
}

impl World {
    fn start(name: &str, stream: bool) -> Self {
        let path = socket_path(&format!("calls-{name}"));
        let calls = Arc::new(FakeCalls::default());
        calls.conversation(DM, "principal-ana");
        let handler = calls.clone();
        let fake = Arc::new(FakeBus::start(&path, move |method, account, body| {
            handler.handle(method, account, body).unwrap_or_else(|| fixture(method))
        }));
        if stream {
            fake.enable_stream();
        }
        let bus: Weak<FakeBus> = Arc::downgrade(&fake);
        calls.set_publisher(Box::new(move |account, topic, body| {
            if let Some(bus) = bus.upgrade() {
                bus.publish(account, topic, body.clone());
                bus.push(account, topic, body);
            }
        }));
        Self { fake, calls, path }
    }

    fn config(&self, call_api: bool) -> SdkConfig {
        SdkConfig { call_api, ..config(&self.path) }
    }

    fn requests(&self, method: &str) -> Vec<Value> {
        self.fake.seen().into_iter().filter(|s| s.method == method).map(|s| s.body).collect()
    }

    fn stop(self) {
        if let Ok(fake) = Arc::try_unwrap(self.fake) {
            fake.shutdown();
        }
    }
}

/// An account source on `world`, live on the stream (or polling with the
/// push subscription open).
fn live(world: &World, call_api: bool, stream: bool) -> Fed {
    let hub = ramflux_desktop_sdk::SdkHub::start(world.config(call_api)).expect("bridge");
    let mut account = Fed::new(hub.account_source(AccountId::new(ACCOUNT)));
    let mode = if stream { SyncMode::Stream } else { SyncMode::Polling };
    account.until("the account is live", |s, _| {
        s.context(&ContextId::new(DM)).is_some()
            && s.sync_status().is_some_and(|sync| sync.mode == mode)
    });
    if stream {
        assert!(world.fake.streaming(ACCOUNT));
    } else {
        account.until("the push subscription", |_, _| world.fake.subscribed(ACCOUNT));
    }
    account
}

fn dispatch(account: &mut Fed, kind: IntentKind) {
    let effects =
        handle_intent(&account.store, Intent::human(kind), Timestamp(0)).expect("allowed");
    for effect in effects {
        account.source.submit(effect);
    }
}

fn the_call(store: &Store) -> Option<&Call> {
    store.calls().max_by_key(|c| (c.started_at, c.id.clone()))
}

fn state_is(state: CallState) -> impl Fn(&Store, &[EventEnvelope]) -> bool {
    move |store, _| the_call(store).is_some_and(|c| c.state == state)
}

/// Asks the source for the media session of `call` and waits for it.
fn media(account: &mut Fed, call: &CallId) -> Result<CallMedia, String> {
    account.source.request_call_media(call);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        account.pump();
        if let Some((id, answer)) = account.source.take_call_media() {
            assert_eq!(&id, call);
            return answer;
        }
        assert!(Instant::now() < deadline, "timed out waiting for the media session");
        std::thread::sleep(Duration::from_millis(15));
    }
}

#[test]
fn an_outgoing_call_rings_connects_carries_media_and_hangs_up() {
    let world = World::start("outgoing", true);
    let mut account = live(&world, true, true);
    assert!(account.store.unsupported_reason(methods::CALL_INVITE).is_none());

    dispatch(&mut account, IntentKind::StartCall { context_id: ContextId::new(DM) });
    account.until("ringing out", state_is(CallState::OutgoingRinging));
    assert_eq!(world.requests("call.invite")[0]["conversation_id"], DM);
    let call = the_call(&account.store).unwrap().clone();
    assert_eq!(call.peer.as_str(), "principal-ana");
    assert_eq!(call.caller.as_str(), "principal-me");
    let record = account.store.moment(&Call::moment_id(&call.id)).expect("record").clone();
    assert_eq!(record.context_id.as_str(), DM);
    assert_eq!(record.lifecycle, Lifecycle::InProgress);
    assert!(matches!(record.kind, MomentKind::Call(_)));
    // One call at a time.
    assert_eq!(
        handle_intent(
            &account.store,
            Intent::human(IntentKind::StartCall { context_id: ContextId::new(DM) }),
            Timestamp(0)
        )
        .unwrap_err(),
        IntentError::CallInProgress
    );

    world.calls.peer_answers(call.id.as_str());
    account.until("connecting", state_is(CallState::Connecting));

    let session = media(&mut account, &call.id).expect("media session");
    assert_eq!(session.endpoint, "203.0.113.7:3478");
    assert_eq!(session.suite, 0x0005);
    assert_eq!((session.send.kid(), session.receive.kid()), (1, 2));
    assert_eq!(session.send.base_key().len(), 32);
    assert_ne!(session.send.base_key(), session.receive.base_key());
    let token: Value = serde_json::from_slice(&session.token).expect("opaque token as sent");
    assert!(token["allocation"].as_str().unwrap().ends_with("/caller"));
    // The fake takes the media session as media flowing.
    account.until("active", state_is(CallState::Active));
    // The keys never entered the store or an event.
    let printed = format!("{:?}", account.envelopes);
    let encoded = ramflux_protocol::encode_base64url(session.send.base_key());
    assert!(!printed.contains(&encoded));

    dispatch(&mut account, IntentKind::HangUpCall { call_id: call.id.clone() });
    account.until("ended", state_is(CallState::Ended(CallEndReason::HungUp)));
    let ended = the_call(&account.store).unwrap();
    assert!(matches!(ended.outcome(), Some(CallOutcome::Answered { .. })));
    assert_eq!(
        account.store.moment(&Call::moment_id(&call.id)).unwrap().lifecycle,
        Lifecycle::Done
    );
    world.stop();
}

#[test]
fn an_incoming_call_rings_here_and_is_answered_or_declined() {
    let world = World::start("incoming", true);
    let mut account = live(&world, true, true);

    let first = CallId::new(world.calls.incoming(ACCOUNT, DM));
    account.until("ringing in", state_is(CallState::IncomingRinging));
    let ringing_attention = |store: &Store| {
        selectors::attention(store)
            .into_iter()
            .filter(|item| item.reason == AttentionReason::IncomingCall)
            .count()
    };
    assert_eq!(ringing_attention(&account.store), 1);
    assert_eq!(selectors::ringing_calls(&account.store)[0].id, first);
    // No media before the call is answered.
    assert!(media(&mut account, &first).is_err());

    dispatch(&mut account, IntentKind::AnswerCall { call_id: first.clone() });
    account.until("connecting", state_is(CallState::Connecting));
    assert_eq!(world.requests("call.answer")[0]["call_id"], first.as_str());
    assert_eq!(ringing_attention(&account.store), 0);
    let session = media(&mut account, &first).expect("media");
    assert_eq!((session.send.kid(), session.receive.kid()), (2, 1));
    account.until("active", state_is(CallState::Active));
    world.calls.peer_hangs_up(first.as_str());
    account.until("ended", state_is(CallState::Ended(CallEndReason::HungUp)));

    world.calls.advance(Duration::from_secs(60));
    let second = CallId::new(world.calls.incoming(ACCOUNT, DM));
    account.until("ringing again", |s, _| {
        s.call(&second).is_some_and(|c| c.state == CallState::IncomingRinging)
    });
    dispatch(&mut account, IntentKind::RejectCall { call_id: second.clone() });
    account.until("declined", |s, _| {
        s.call(&second).is_some_and(|c| c.state == CallState::Ended(CallEndReason::Rejected))
    });
    assert_eq!(account.store.call(&second).unwrap().outcome(), Some(CallOutcome::Declined));
    // Each call is one record in the conversation.
    let records = account
        .store
        .context_moments(&ContextId::new(DM))
        .filter(|m| matches!(m.kind, MomentKind::Call(_)))
        .count();
    assert_eq!(records, 2);
    world.stop();
}

#[test]
fn timeout_busy_and_answered_elsewhere_end_calls_with_their_reason() {
    let world = World::start("endings", true);
    let mut account = live(&world, true, true);
    let wait_end = |account: &mut Fed, id: &CallId, reason: CallEndReason| {
        let id = id.clone();
        account.until("ended", move |s, _| {
            s.call(&id).is_some_and(|c| c.state == CallState::Ended(reason))
        });
    };

    // Nobody answers an incoming call within 45 s: missed.
    let missed = CallId::new(world.calls.incoming(ACCOUNT, DM));
    account.until("ringing", state_is(CallState::IncomingRinging));
    world.calls.advance(Duration::from_secs(45));
    wait_end(&mut account, &missed, CallEndReason::Timeout);
    assert_eq!(account.store.call(&missed).unwrap().outcome(), Some(CallOutcome::Missed));

    // Another device answers: not a missed call here.
    let elsewhere = CallId::new(world.calls.incoming(ACCOUNT, DM));
    account.until("ringing", state_is(CallState::IncomingRinging));
    world.calls.answered_elsewhere(elsewhere.as_str());
    wait_end(&mut account, &elsewhere, CallEndReason::AnsweredElsewhere);
    assert_eq!(
        account.store.call(&elsewhere).unwrap().outcome(),
        Some(CallOutcome::AnsweredElsewhere)
    );

    // The peer is busy.
    dispatch(&mut account, IntentKind::StartCall { context_id: ContextId::new(DM) });
    account.until("ringing out", state_is(CallState::OutgoingRinging));
    let busy = the_call(&account.store).unwrap().id.clone();
    world.calls.peer_busy(busy.as_str());
    wait_end(&mut account, &busy, CallEndReason::Busy);
    assert_eq!(account.store.call(&busy).unwrap().outcome(), Some(CallOutcome::Busy));

    // Nobody answers an outgoing call: no answer.
    world.calls.advance(Duration::from_secs(1));
    dispatch(&mut account, IntentKind::StartCall { context_id: ContextId::new(DM) });
    account.until("ringing out", |s, _| {
        s.calls().filter(|c| c.state == CallState::OutgoingRinging).count() == 1
    });
    let unanswered = selectors::current_call(&account.store).unwrap().id.clone();
    // While it rings, another incoming call finds this account busy.
    let while_busy = CallId::new(world.calls.incoming(ACCOUNT, DM));
    wait_end(&mut account, &while_busy, CallEndReason::Busy);
    assert_eq!(account.store.call(&while_busy).unwrap().outcome(), Some(CallOutcome::Missed));
    world.calls.advance(Duration::from_secs(45));
    wait_end(&mut account, &unanswered, CallEndReason::Timeout);
    assert_eq!(account.store.call(&unanswered).unwrap().outcome(), Some(CallOutcome::NoAnswer));
    assert!(!selectors::call_under_way(&account.store));
    world.stop();
}

#[test]
fn without_the_stream_calls_change_through_the_push_subscription() {
    let world = World::start("push", false);
    let mut account = live(&world, true, false);
    let id = CallId::new(world.calls.incoming(ACCOUNT, DM));
    account.until("ringing", state_is(CallState::IncomingRinging));
    world.calls.peer_hangs_up(id.as_str());
    account.until("cancelled by the caller", state_is(CallState::Ended(CallEndReason::HungUp)));
    assert_eq!(account.store.call(&id).unwrap().outcome(), Some(CallOutcome::Missed));
    world.stop();
}

#[test]
fn without_the_call_api_calls_are_unsupported_with_a_reason() {
    let world = World::start("no-api", true);
    let mut account = live(&world, false, true);
    assert_eq!(
        account.store.unsupported_reason(methods::CALL_INVITE),
        Some(ramflux_desktop_sdk::capabilities::reason::CALLS)
    );
    for method in [methods::CALL_ANSWER, methods::CALL_REJECT, methods::CALL_HANGUP] {
        assert!(account.store.unsupported_reason(method).is_some(), "{method}");
    }
    assert!(matches!(
        handle_intent(
            &account.store,
            Intent::human(IntentKind::StartCall { context_id: ContextId::new(DM) }),
            Timestamp(0)
        ),
        Err(IntentError::UnsupportedByBackend { method: "call.invite", .. })
    ));
    // An effect that reaches the source anyway fails without a request.
    account.source.submit(ramflux_desktop_core::Effect::Call {
        method: methods::CALL_INVITE.to_owned(),
        body: json!({ "context_id": DM, "proposed_by": "Human", "confirmed_by": null }),
    });
    account.until("refused", |s, _| {
        s.failures().iter().any(|f| {
            f.method == methods::CALL_INVITE && f.code == FailureCode::UnsupportedByBackend
        })
    });
    assert_eq!(
        media(&mut account, &CallId::new("call-x")).unwrap_err(),
        ramflux_desktop_sdk::capabilities::reason::CALLS
    );
    assert!(world.requests("call.invite").is_empty());
    assert!(world.requests("call.media.session").is_empty());
    // The stream does not ask a daemon without calls for call events.
    let subscribed = world.requests("bus.subscribe").pop().expect("subscribed");
    assert!(!subscribed["topics"].as_array().unwrap().iter().any(|t| t == "call.changed"));
    world.stop();
}
