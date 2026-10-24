// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Voice calls in the production window, on a fake daemon with the CALL-01
//! call API and stand-in audio (no devices): the call button of a direct
//! conversation places a call; the in-call bar shows calling, connecting
//! and the call's quality once the media engine runs with the daemon's
//! media session; Mute reaches the audio; a blocked microphone is
//! explained; Hang up ends the call and stops the audio; an incoming call
//! rings (the ring tone plays), shows its card, and is answered or
//! declined there; a call nobody answers becomes a missed call with Call
//! back; and without the call API the call button is disabled.

mod common;
mod sdk_fake;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use ramflux_desktop_core::{
    CallEndReason, CallId, CallMedia, CallOutcome, CallState, ContextId, Store, SyncMode, selectors,
};
use ramflux_desktop_ui::app::{CallAudio, CallQuality, LiveCall, LiveStatus};
use ramflux_desktop_ui::i18n::tr;
use ramflux_desktop_ui::route::Page;
use ramflux_desktop_ui::shell::SourceChoice;
use sdk_fake::wait;

const ACCOUNT: &str = "acct-main";
const DM: &str = "dm-ana";

/// What the stand-in audio saw and reports.
#[derive(Default)]
struct Audio {
    started: Vec<(String, u16, u64, u64)>,
    running: bool,
    muted: bool,
    status: Option<LiveStatus>,
    rings: usize,
    ringing: bool,
}

#[derive(Clone, Default)]
struct FakeAudio(Rc<RefCell<Audio>>);

struct FakeLive(Rc<RefCell<Audio>>);

impl LiveCall for FakeLive {
    fn set_muted(&mut self, muted: bool) {
        self.0.borrow_mut().muted = muted;
    }

    fn status(&self) -> LiveStatus {
        self.0
            .borrow()
            .status
            .unwrap_or(LiveStatus { quality: CallQuality::Good, microphone_blocked: false })
    }
}

impl Drop for FakeLive {
    fn drop(&mut self) {
        self.0.borrow_mut().running = false;
    }
}

struct Ring(Rc<RefCell<Audio>>);

impl Drop for Ring {
    fn drop(&mut self) {
        self.0.borrow_mut().ringing = false;
    }
}

impl CallAudio for FakeAudio {
    fn start(&self, media: CallMedia) -> Result<Box<dyn LiveCall>, String> {
        let mut audio = self.0.borrow_mut();
        audio.started.push((
            media.endpoint.clone(),
            media.suite,
            media.send.kid(),
            media.receive.kid(),
        ));
        audio.running = true;
        Ok(Box::new(FakeLive(self.0.clone())))
    }

    fn ring(&self) -> Option<Box<dyn std::any::Any>> {
        let mut audio = self.0.borrow_mut();
        audio.rings += 1;
        audio.ringing = true;
        Some(Box::new(Ring(self.0.clone())))
    }
}

struct World {
    fake: std::sync::Arc<ramflux_desktop_test::fake_bus::FakeBus>,
    calls: std::sync::Arc<ramflux_desktop_test::fake_calls::FakeCalls>,
    handle: gpui_kit::WindowHandle<gpui_kit::base::Root>,
    shell: gpui_kit::Entity<ramflux_desktop_ui::Shell>,
    audio: FakeAudio,
}

fn open_calls(cx: &mut TestAppContext, name: &str, call_api: bool) -> World {
    let (fake, state, path) = sdk_fake::start(name);
    fake.enable_stream();
    let fake = std::sync::Arc::new(fake);
    let calls = state.lock().expect("state").calls.clone();
    let bus = std::sync::Arc::downgrade(&fake);
    calls.set_publisher(Box::new(move |account, topic, body| {
        if let Some(bus) = bus.upgrade() {
            bus.publish(account, topic, body);
        }
    }));
    let mut options = sdk_fake::options(&path);
    if let SourceChoice::Rfd(config) = &mut options.source {
        config.call_api = call_api;
    }
    let (handle, shell) = open_sized(cx, options, 900.);
    let audio = FakeAudio::default();
    let installed = audio.clone();
    store_handle(cx, &shell).update(cx, |store, _| store.set_call_audio(Rc::new(installed)));
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the stream", |cx| {
        with_store(cx, &shell, |s| s.sync_status().is_some_and(|x| x.mode == SyncMode::Stream))
    });
    show_context(cx, handle, &shell, DM);
    World { fake, calls, handle, shell, audio }
}

fn call_state(cx: &mut TestAppContext, world: &World, id: &CallId) -> Option<CallState> {
    with_store(cx, &world.shell, |s| s.call(id).map(|c| c.state))
}

fn label(cx: &mut TestAppContext, world: &World, id: &'static str) -> Option<String> {
    in_window(cx, world.handle, move |window, _| {
        window.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
}

fn current(store: &Store) -> Option<CallId> {
    selectors::current_call(store).map(|c| c.id.clone())
}

#[gpui_kit::test]
fn an_outgoing_call_from_the_header_runs_and_hangs_up(cx: &mut TestAppContext) {
    let world = open_calls(cx, "ui-call-out", true);
    assert!(exists(cx, world.handle, "conversation-call"));
    assert!(!exists(cx, world.handle, "call-bar-status"));

    click(cx, world.handle, "conversation-call");
    wait(cx, "calling", |cx| with_store(cx, &world.shell, current).is_some());
    let id = with_store(cx, &world.shell, current).unwrap();
    assert_eq!(call_state(cx, &world, &id), Some(CallState::OutgoingRinging));
    assert_eq!(label(cx, &world, "call-bar-status").as_deref(), Some(&*tr("call.calling")));
    assert_eq!(label(cx, &world, "call-hang-up").as_deref(), Some(&*tr("call.cancel")));
    let invites = world.fake.seen().into_iter().filter(|s| s.method == "call.invite").count();
    assert_eq!(invites, 1);
    // The header's call button waits while this call is under way.
    let disabled =
        in_window(cx, world.handle, |window, _| window.find("conversation-call").disabled());
    assert_eq!(disabled, Some(true));
    assert!(world.audio.0.borrow().started.is_empty(), "no audio before an answer");

    world.calls.peer_answers(id.as_str());
    wait(cx, "audio", |_| world.audio.0.borrow().running);
    let (endpoint, suite, send, receive) = world.audio.0.borrow().started[0].clone();
    assert_eq!((endpoint.as_str(), suite, send, receive), ("203.0.113.7:3478", 5, 1, 2));
    wait(cx, "active", |cx| call_state(cx, &world, &id) == Some(CallState::Active));
    wait(cx, "quality shown", |cx| {
        label(cx, &world, "call-bar-quality").as_deref() == Some(&*tr("call.quality.good"))
    });
    assert_eq!(label(cx, &world, "call-hang-up").as_deref(), Some(&*tr("call.hang_up")));

    click(cx, world.handle, "call-mute");
    assert!(world.audio.0.borrow().muted);
    assert_eq!(label(cx, &world, "call-mute").as_deref(), Some(&*tr("call.unmute")));
    click(cx, world.handle, "call-mute");
    assert!(!world.audio.0.borrow().muted);

    // macOS hands a denied microphone silence: the bar explains it.
    world.audio.0.borrow_mut().status =
        Some(LiveStatus { quality: CallQuality::Poor, microphone_blocked: true });
    advance(cx, 1_100);
    assert!(exists(cx, world.handle, "call-bar-warning"));
    assert_eq!(
        exists(cx, world.handle, "call-open-settings"),
        cfg!(target_os = "macos"),
        "the settings shortcut is a macOS one"
    );

    click(cx, world.handle, "call-hang-up");
    wait(cx, "ended", |cx| {
        call_state(cx, &world, &id) == Some(CallState::Ended(CallEndReason::HungUp))
    });
    wait(cx, "audio stopped", |_| !world.audio.0.borrow().running);
    advance(cx, 100);
    assert!(!exists(cx, world.handle, "call-bar-status"));
    // The call stays as one record, with Call back, which calls again.
    assert!(rows(cx, &world.shell).iter().any(|row| row == &format!("call:{id}")));
    click(cx, world.handle, "call-back");
    wait(cx, "calling back", |cx| {
        with_store(cx, &world.shell, current).is_some_and(|again| again != id)
    });
}

#[gpui_kit::test]
fn an_incoming_call_rings_and_is_answered_from_its_card(cx: &mut TestAppContext) {
    let world = open_calls(cx, "ui-call-in", true);
    let id = CallId::new(world.calls.incoming(ACCOUNT, DM));
    wait(cx, "ringing", |cx| call_state(cx, &world, &id) == Some(CallState::IncomingRinging));
    advance(cx, 100);
    assert!(exists(cx, world.handle, "incoming-call"));
    assert!(exists(cx, world.handle, "incoming-answer"));
    assert!(world.audio.0.borrow().ringing, "the ring tone plays");

    click(cx, world.handle, "incoming-answer");
    wait(cx, "answered", |cx| call_state(cx, &world, &id) == Some(CallState::Active));
    advance(cx, 100);
    assert!(!world.audio.0.borrow().ringing, "the ring tone stopped");
    assert!(!exists(cx, world.handle, "incoming-answer"));
    assert!(exists(cx, world.handle, "call-bar-status"));
    assert_eq!(world.audio.0.borrow().started[0].2, 2, "the callee sends under its own key");

    world.calls.peer_hangs_up(id.as_str());
    wait(cx, "ended", |cx| {
        call_state(cx, &world, &id) == Some(CallState::Ended(CallEndReason::HungUp))
    });
    wait(cx, "audio stopped", |_| !world.audio.0.borrow().running);
}

#[gpui_kit::test]
fn a_declined_or_unanswered_call_leaves_its_record(cx: &mut TestAppContext) {
    let world = open_calls(cx, "ui-call-decline", true);
    let declined = CallId::new(world.calls.incoming(ACCOUNT, DM));
    wait(cx, "ringing", |cx| call_state(cx, &world, &declined) == Some(CallState::IncomingRinging));
    advance(cx, 100);
    click(cx, world.handle, "incoming-decline");
    wait(cx, "declined", |cx| {
        call_state(cx, &world, &declined) == Some(CallState::Ended(CallEndReason::Rejected))
    });
    advance(cx, 100);
    assert!(!exists(cx, world.handle, "incoming-call-name"));
    assert!(!world.audio.0.borrow().ringing);
    assert!(world.audio.0.borrow().started.is_empty(), "no audio for a declined call");

    world.calls.advance(Duration::from_secs(60));
    let missed = CallId::new(world.calls.incoming(ACCOUNT, DM));
    wait(cx, "ringing", |cx| call_state(cx, &world, &missed) == Some(CallState::IncomingRinging));
    world.calls.advance(Duration::from_secs(45));
    wait(cx, "missed", |cx| {
        call_state(cx, &world, &missed) == Some(CallState::Ended(CallEndReason::Timeout))
    });
    let outcome = with_store(cx, &world.shell, |s| s.call(&missed).and_then(|c| c.outcome()));
    assert_eq!(outcome, Some(CallOutcome::Missed));
    advance(cx, 100);
    let records: Vec<String> =
        rows(cx, &world.shell).into_iter().filter(|r| r.starts_with("call:")).collect();
    assert_eq!(records.len(), 2);
    assert_eq!(world.audio.0.borrow().rings, 2);
    assert!(exists_in(cx, world.handle, &format!("moment-call:{missed}"), "call-back"));
    assert!(with_store(cx, &world.shell, |s| !selectors::call_under_way(s)));
    let context = with_store(cx, &world.shell, |s| s.call(&missed).map(|c| c.context_id.clone()));
    assert_eq!(context, Some(ContextId::new(DM)));
}

#[gpui_kit::test]
fn without_the_call_api_the_call_button_is_disabled(cx: &mut TestAppContext) {
    let world = open_calls(cx, "ui-call-off", false);
    let disabled =
        in_window(cx, world.handle, |window, _| window.find("conversation-call").disabled());
    assert_eq!(disabled, Some(true));
    click(cx, world.handle, "conversation-call");
    advance(cx, 300);
    assert!(world.fake.seen().iter().all(|s| s.method != "call.invite"));
    assert!(with_store(cx, &world.shell, |s| s.calls().next().is_none()));
}
