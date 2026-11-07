// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Typed v1 call controls (S10c c08b): all 13 actions project to the call
//! model, with who-may-send, replay and no-revival checks, and no key
//! material in anything emitted.

mod common;
mod sdk_fake;

use common::*;
use gpui_kit::TestAppContext;
use ramflux_desktop_core::{
    AccountId, Call, CallEndReason as End, CallId, CallState as State, ContextId, DomainEvent,
};
use ramflux_desktop_sdk::Scope;
use ramflux_desktop_sdk::translate::Translator;
use ramflux_desktop_sdk::translate::v1::call::{
    MAX_CALLS, Signal, generations, set_self_principal, signal_of,
};
use ramflux_desktop_sdk::translate::v1::candidate::apply;
use ramflux_desktop_test::v1_call_fixtures::{CallSpec, SECRET_KEY, action, all_actions, rekey};
use ramflux_desktop_test::v1_fixtures::identity;
use ramflux_protocol::message::MessageUpdate;

const DM: &str = "dm-ana";
const ME: &str = "principal-me";
const ANA: &str = "principal-ana";

/// A call where `caller` rings `callee`; the local side is `ME`.
fn spec(caller: &str, callee: &str) -> CallSpec {
    CallSpec::new(DM, "call-1", caller, callee)
}

fn control(spec: &CallSpec, sender: &str, seq: u64, name: &str) -> MessageUpdate {
    let n = u8::try_from(seq).expect("small");
    spec.control(
        identity(DM, &[n; 4], "dev-x", seq * 10, seq),
        sender,
        seq,
        1_000 + i64::from(n),
        action(name),
    )
}

fn translator() -> Translator {
    let mut t = Translator::new(Scope::Account(AccountId::new("acct-main")), "/tmp/s10c-call.sock");
    set_self_principal(&mut t, ME);
    t
}

/// Applies the controls in order; the call states they report.
fn states(t: &mut Translator, updates: &[MessageUpdate]) -> Vec<State> {
    calls(t, updates).into_iter().map(|c| c.state).collect()
}

fn calls(t: &mut Translator, updates: &[MessageUpdate]) -> Vec<Call> {
    apply(t, updates)
        .events
        .into_iter()
        .filter_map(|event| match event {
            DomainEvent::CallUpserted(call) => Some(call),
            _ => None,
        })
        .collect()
}

#[test]
fn v1_call_projection_all_actions() {
    // The wire action maps to its own signal: thirteen rows, none shared.
    let expected = [
        ("invite", Signal::Invite),
        ("ringing", Signal::Ringing),
        ("answer", Signal::Answer),
        ("ack", Signal::Ack),
        ("ready", Signal::Ready),
        ("reject", Signal::Reject),
        ("busy", Signal::Busy),
        ("hangup", Signal::Hangup),
        ("timeout", Signal::Timeout),
        ("failed", Signal::Failed),
        ("answered_elsewhere", Signal::AnsweredElsewhere),
        ("rekey", Signal::Rekey { generation: 1 }),
        ("rekey_ack", Signal::RekeyAck { generation: 1 }),
    ];
    let actions = all_actions();
    assert_eq!(actions.len(), 13);
    for ((name, wire), (row, signal)) in actions.iter().zip(expected) {
        assert_eq!(*name, row);
        assert_eq!(signal_of(wire), Some(signal), "{row}");
    }
    let mut seen = Vec::new();
    for (_, wire) in &actions {
        let signal = signal_of(wire);
        assert!(!seen.contains(&signal), "two actions share {signal:?}");
        seen.push(signal);
    }
    assert_eq!(signal_of(&ramflux_protocol::message::CallAction { value: None }), None);

    // Each action's effect on the call, as the callee (incoming) sees it.
    let incoming = spec(ANA, ME);
    let invite = || control(&incoming, ANA, 1, "invite");
    let answered = || control(&incoming, ME, 2, "answer");
    let rows: [(&str, Vec<MessageUpdate>, Vec<State>); 11] = [
        ("invite", vec![invite()], vec![State::IncomingRinging]),
        (
            "ringing",
            vec![invite(), control(&incoming, ME, 2, "ringing")],
            vec![State::IncomingRinging],
        ),
        ("answer", vec![invite(), answered()], vec![State::IncomingRinging, State::Connecting]),
        (
            "ack",
            vec![invite(), answered(), control(&incoming, ANA, 2, "ack")],
            vec![State::IncomingRinging, State::Connecting],
        ),
        (
            "ready",
            vec![invite(), answered(), control(&incoming, ANA, 2, "ready")],
            vec![State::IncomingRinging, State::Connecting, State::Active],
        ),
        (
            "reject",
            vec![invite(), control(&incoming, ME, 2, "reject")],
            vec![State::IncomingRinging, State::Ended(End::Declined)],
        ),
        (
            "busy",
            vec![invite(), control(&incoming, ME, 2, "busy")],
            vec![State::IncomingRinging, State::Ended(End::Busy)],
        ),
        (
            "hangup",
            vec![invite(), control(&incoming, ANA, 2, "hangup")],
            vec![State::IncomingRinging, State::Ended(End::Missed)],
        ),
        (
            "timeout",
            vec![invite(), control(&incoming, ANA, 2, "timeout")],
            vec![State::IncomingRinging, State::Ended(End::Missed)],
        ),
        (
            "failed",
            vec![invite(), control(&incoming, ANA, 2, "failed")],
            vec![State::IncomingRinging, State::Ended(End::Failed)],
        ),
        (
            "answered_elsewhere",
            vec![invite(), control(&incoming, ANA, 2, "answered_elsewhere")],
            vec![State::IncomingRinging, State::Ended(End::AnsweredElsewhere)],
        ),
    ];
    for (row, updates, want) in rows {
        assert_eq!(states(&mut translator(), &updates), want, "incoming {row}");
    }

    // The caller's view of the same signals.
    let outgoing = spec(ME, ANA);
    let ring = || control(&outgoing, ME, 1, "invite");
    let outgoing_rows: [(&str, Vec<MessageUpdate>, State); 6] = [
        ("outgoing invite", vec![ring()], State::OutgoingRinging),
        ("cancel", vec![ring(), control(&outgoing, ME, 2, "hangup")], State::Ended(End::Cancelled)),
        ("timeout", vec![ring(), control(&outgoing, ME, 2, "timeout")], State::Ended(End::Timeout)),
        (
            "declined",
            vec![ring(), control(&outgoing, ANA, 2, "reject")],
            State::Ended(End::Declined),
        ),
        ("busy", vec![ring(), control(&outgoing, ANA, 2, "busy")], State::Ended(End::Busy)),
        (
            "completed",
            vec![ring(), control(&outgoing, ANA, 2, "answer"), control(&outgoing, ME, 3, "hangup")],
            State::Ended(End::Completed),
        ),
    ];
    for (row, updates, last) in outgoing_rows {
        let got = states(&mut translator(), &updates);
        assert_eq!(got.last(), Some(&last), "outgoing {row}");
    }

    // Rekey proposes a generation; its ack answers exactly that one.
    let mut t = translator();
    let setup = [invite(), answered()];
    let _ = states(&mut t, &setup);
    let id = CallId::new("call-1");
    assert_eq!(generations(&t, &id), Some((0, 0)));
    assert!(states(&mut t, &[control(&incoming, ANA, 3, "rekey")]).is_empty());
    assert_eq!(generations(&t, &id), Some((1, 0)), "rekey proposes");
    assert!(states(&mut t, &[control(&incoming, ME, 3, "rekey_ack")]).is_empty());
    assert_eq!(generations(&t, &id), Some((1, 1)), "its ack accepts");
}

#[test]
fn rekey_generations_only_rise_and_acks_answer_a_proposal() {
    let incoming = spec(ANA, ME);
    let mut t = translator();
    let _ = states(
        &mut t,
        &[control(&incoming, ANA, 1, "invite"), control(&incoming, ME, 2, "answer")],
    );
    let id = CallId::new("call-1");
    let send = |t: &mut Translator, sender: &str, seq: u64, generation: u64, ack: bool| {
        let n = u8::try_from(seq).expect("small");
        let update = incoming.control(
            identity(DM, &[n; 4], "dev-x", seq * 10, seq),
            sender,
            seq,
            1_000,
            rekey(generation, ack),
        );
        let _ = apply(t, &[update]);
    };
    send(&mut t, ME, 3, 1, true);
    assert_eq!(generations(&t, &id), Some((0, 0)), "an ack with no proposal is ignored");
    send(&mut t, ANA, 3, 2, false);
    send(&mut t, ANA, 4, 1, false);
    assert_eq!(generations(&t, &id), Some((2, 0)), "a lower proposal never replaces");
    send(&mut t, ME, 4, 1, true);
    assert_eq!(generations(&t, &id), Some((2, 0)), "an ack of another generation is ignored");
    send(&mut t, ME, 5, 2, true);
    send(&mut t, ME, 6, 2, true);
    assert_eq!(generations(&t, &id), Some((2, 2)));
}

#[test]
fn senders_replays_and_ended_calls_are_checked() {
    let incoming = spec(ANA, ME);
    let invite = || control(&incoming, ANA, 1, "invite");
    // An invite from the callee, a first control that is not an invite, and a
    // sender who is neither endpoint place nothing.
    assert!(states(&mut translator(), &[control(&incoming, ME, 1, "invite")]).is_empty());
    assert!(states(&mut translator(), &[control(&incoming, ANA, 1, "answer")]).is_empty());
    assert!(
        states(&mut translator(), &[control(&incoming, "principal-zed", 1, "invite")]).is_empty()
    );
    // A call between two other principals is not ours.
    let other = spec("principal-bo", "principal-cy");
    assert!(states(&mut translator(), &[control(&other, "principal-bo", 1, "invite")]).is_empty());
    // The envelope and the control must name one conversation.
    let elsewhere = CallSpec::new("c-elsewhere", "call-1", ANA, ME);
    assert!(states(&mut translator(), &[control(&elsewhere, ANA, 1, "invite")]).is_empty());
    // Without a known self principal nothing is placed.
    let mut blank =
        Translator::new(Scope::Account(AccountId::new("acct-main")), "/tmp/s10c-call.sock");
    assert!(states(&mut blank, &[invite()]).is_empty());
    // A replay of a sender's sequence, and a later control after the end.
    let mut t = translator();
    assert_eq!(states(&mut t, &[invite(), invite()]).len(), 1, "replayed invite");
    let ended = control(&incoming, ME, 2, "reject");
    assert_eq!(states(&mut t, &[ended.clone(), ended]), vec![State::Ended(End::Declined)]);
    assert!(states(&mut t, &[control(&incoming, ME, 3, "answer")]).is_empty(), "no revival");
    assert!(states(&mut t, &[control(&incoming, ANA, 4, "ready")]).is_empty(), "no revival");
    // A control that disagrees with the call's endpoints is ignored.
    let mut t = translator();
    let _ = states(&mut t, &[invite()]);
    let swapped = spec(ME, ANA);
    assert!(states(&mut t, &[control(&swapped, ANA, 2, "answer")]).is_empty());
}

#[test]
fn retained_calls_are_bounded_and_cleared_on_lock_and_nothing_secret_is_emitted() {
    let mut t = translator();
    let mut placed = 0;
    for n in 0..(MAX_CALLS + 8) {
        let call = CallSpec::new(DM, &format!("call-{n}"), ANA, ME);
        let id = u32::try_from(n).expect("small").to_be_bytes();
        let update =
            call.control(identity(DM, &id, "dev-x", 1, 1), ANA, 1, 1_000, action("invite"));
        placed += calls(&mut t, &[update]).len();
    }
    assert_eq!(placed, MAX_CALLS, "calls beyond the bound are not placed");
    let _ = t.own_account_locked();
    let incoming = spec(ANA, ME);
    // Everything was dropped: the old call is placed afresh, not a replay.
    assert_eq!(states(&mut t, &[control(&incoming, ANA, 1, "invite")]).len(), 1);

    let mut t = translator();
    let all: Vec<MessageUpdate> = [
        (ANA, 1, "invite"),
        (ME, 2, "ringing"),
        (ME, 3, "answer"),
        (ANA, 4, "ack"),
        (ANA, 5, "ready"),
        (ANA, 6, "rekey"),
        (ME, 7, "rekey_ack"),
        (ANA, 8, "hangup"),
    ]
    .into_iter()
    .map(|(sender, seq, name)| control(&incoming, sender, seq, name))
    .collect();
    let events = apply(&mut t, &all).events;
    assert!(!events.is_empty());
    let text = serde_json::to_string(&events).expect("events serialize");
    for secret in
        [SECRET_KEY, "SECRET-SIGNATURE", "root-principal", "base_key", "allocation", "consent"]
    {
        assert!(!text.contains(secret), "`{secret}` reached a model");
    }
}

const DM_ME: &str = "principal-me";

fn open_dm(
    cx: &mut TestAppContext,
    name: &str,
) -> (
    ramflux_desktop_test::fake_bus::FakeBus,
    std::path::PathBuf,
    gpui_kit::Entity<ramflux_desktop_ui::Shell>,
) {
    let (fake, _state, path) = sdk_fake::start(name);
    let (_handle, shell) = open_with(cx, sdk_fake::options(&path));
    wait(cx, "workspace", |cx| page(cx, &shell) == ramflux_desktop_ui::route::Page::Workspace);
    wait(cx, "the conversation", |cx| {
        with_store(cx, &shell, |s| s.context(&ContextId::new(DM)).is_some())
    });
    wait(cx, "the account principal", |cx| {
        with_store(cx, &shell, |s| s.context(&ContextId::new(DM)).is_some())
    });
    (fake, path, shell)
}

#[gpui_kit::test]
fn a_pushed_v1_call_follows_its_signals_in_the_store(cx: &mut TestAppContext) {
    let _locale = locale_guard();
    let (fake, path, shell) = open_dm(cx, "s10c-call");
    let call = CallSpec::new(DM, "call-ui", ANA, DM_ME);
    let push = |n: u8, sender: &str, name: &str| {
        let update = call.control(
            identity(DM, &[n; 4], "dev-x", u64::from(n) * 10, u64::from(n)),
            sender,
            u64::from(n),
            1_000,
            action(name),
        );
        assert!(sdk_fake::push_generated(&path, update));
    };
    let id = CallId::new("call-ui");
    push(1, ANA, "invite");
    wait(cx, "the call", |cx| with_store(cx, &shell, |s| s.call(&id).is_some()));
    assert_eq!(
        with_store(cx, &shell, |s| s.call(&id).map(|c| c.state)),
        Some(State::IncomingRinging)
    );
    push(2, DM_ME, "answer");
    wait(cx, "connecting", |cx| {
        with_store(cx, &shell, |s| s.call(&id).map(|c| c.state)) == Some(State::Connecting)
    });
    push(3, ANA, "hangup");
    wait(cx, "completed", |cx| {
        with_store(cx, &shell, |s| s.call(&id).map(|c| c.state))
            == Some(State::Ended(End::Completed))
    });
    fake.shutdown();
}
