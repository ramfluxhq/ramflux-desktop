// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Voice calls on a real node (DESK-F16 on CALL-01), after the demo's
//! accounts, contacts and direct messages:
//!
//! 1. Alice calls Bob from the conversation header; Bob's desktop rings and
//!    he answers from the incoming-call card; both daemons report the call
//!    active; each desktop's media engine (the production engine, with a
//!    test tone for a microphone: Alice 440 Hz, Bob 660 Hz) carries audio
//!    through the node's relay, and each side decodes the other's tone;
//!    Alice hangs up (completed on both sides);
//! 2. Alice calls again and Bob declines (declined on both sides);
//! 3. Alice calls and nobody answers for the 45-second ring timeout
//!    (timeout for Alice, missed for Bob).
//!
//! No audio devices: the audio each desktop installs is [`ToneAudio`].

use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ramflux_desktop_core::{CallEndReason, CallId, CallMedia, CallState, selectors};
use ramflux_desktop_media::codec::{FRAME_SAMPLES, Frame};
use ramflux_desktop_media::{Capture, Engine, EngineStats, Playback};
use ramflux_desktop_ui::app::{CallAudio, LiveCall, LiveStatus, media_session};

use super::super::daemon::DemoDaemon;
use super::super::host::Host;
use super::super::steps::Notes;
use super::Demo;

/// What one desktop's call audio did.
#[derive(Debug, Default, Clone)]
pub struct Report {
    /// Media sessions the engine started with: (call, send KID, rebind side).
    pub sessions: Vec<(String, u64, Option<u8>)>,
    /// Frames played that carry a tone, and of those, the ones at the
    /// peer's frequency.
    pub loud: u64,
    pub peer_tone: u64,
    /// The engine's counters when last read.
    pub stats: EngineStats,
    /// Times the ring tone was asked for.
    pub rings: u64,
    /// Why the engine did not start, if it did not.
    pub error: Option<String>,
}

/// Call audio of a desktop without devices: a test tone for a microphone,
/// and a player that measures what it decodes.
pub struct ToneAudio {
    hz: f32,
    peer_hz: f32,
    pub report: Arc<Mutex<Report>>,
}

impl ToneAudio {
    pub fn new(hz: f32, peer_hz: f32) -> Rc<Self> {
        Rc::new(Self { hz, peer_hz, report: Arc::default() })
    }

    pub fn report(&self) -> Report {
        self.report.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
    }
}

struct Tone {
    hz: f32,
    index: usize,
    next: Instant,
}

impl Capture for Tone {
    fn next_frame(&mut self, _timeout: Duration) -> Option<Frame> {
        let now = Instant::now();
        if self.next > now {
            std::thread::sleep(self.next - now);
        } else if now - self.next > Duration::from_millis(200) {
            self.next = now;
        }
        self.next += Duration::from_millis(20);
        let mut frame = [0_i16; FRAME_SAMPLES];
        for (i, sample) in frame.iter_mut().enumerate() {
            let t = (self.index * FRAME_SAMPLES + i) as f32 / 48_000.0;
            *sample = ((2.0 * std::f32::consts::PI * self.hz * t).sin() * 12_000.0) as i16;
        }
        self.index += 1;
        Some(frame)
    }
}

/// Measures the decoded audio: loud frames, and their frequency from the
/// zero crossings (a 20 ms frame of `hz` crosses zero `hz * 0.04` times).
struct Meter {
    peer_hz: f32,
    report: Arc<Mutex<Report>>,
}

impl Playback for Meter {
    fn play(&mut self, frame: &Frame) {
        let rms = (frame.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / FRAME_SAMPLES as f64)
            .sqrt();
        if rms < 2_000.0 {
            return;
        }
        let crossings = frame.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count() as f32;
        let hz = crossings / 0.04;
        let mut report = self.report.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        report.loud += 1;
        if (hz - self.peer_hz).abs() < self.peer_hz * 0.1 {
            report.peer_tone += 1;
        }
    }
}

struct ToneCall {
    engine: Engine,
    report: Arc<Mutex<Report>>,
}

impl LiveCall for ToneCall {
    fn set_muted(&mut self, muted: bool) {
        self.engine.set_muted(muted);
    }

    fn status(&self) -> LiveStatus {
        let stats = self.engine.stats();
        self.report.lock().unwrap_or_else(std::sync::PoisonError::into_inner).stats = stats;
        LiveStatus {
            quality: ramflux_desktop_media::engine::quality_of(&stats).into(),
            microphone_blocked: stats.microphone_blocked,
        }
    }
}

impl Drop for ToneCall {
    fn drop(&mut self) {
        let stats = self.engine.stats();
        self.report.lock().unwrap_or_else(std::sync::PoisonError::into_inner).stats = stats;
    }
}

impl CallAudio for ToneAudio {
    fn start(&self, media: CallMedia) -> Result<Box<dyn LiveCall>, String> {
        let call = media.call_id.to_string();
        let side = media.rebind.as_ref().map(|r| r.side);
        let kid = media.send.kid();
        let session = media_session(media);
        let started = Engine::start(
            session,
            Box::new(Tone { hz: self.hz, index: 0, next: Instant::now() }),
            Box::new(Meter { peer_hz: self.peer_hz, report: self.report.clone() }),
        );
        let mut report = self.report.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        report.sessions.push((call, kid, side));
        match started {
            Ok(engine) => {
                drop(report);
                Ok(Box::new(ToneCall { engine, report: self.report.clone() }))
            }
            Err(error) => {
                report.error = Some(error.to_string());
                Err(error.to_string())
            }
        }
    }

    fn ring(&self) -> Option<Box<dyn std::any::Any>> {
        self.report.lock().unwrap_or_else(std::sync::PoisonError::into_inner).rings += 1;
        Some(Box::new(()))
    }
}

fn current<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    ix: usize,
) -> Option<(CallId, CallState)> {
    demo.read(ix, |s| selectors::current_call(s).map(|c| (c.id.clone(), c.state))).ok().flatten()
}

fn state<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    ix: usize,
    id: &CallId,
) -> Option<CallState> {
    demo.read(ix, |s| s.call(id).map(|c| c.state)).ok().flatten()
}

fn wait_state<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    ix: usize,
    id: &CallId,
    want: CallState,
    limit: Duration,
) -> Result<Duration, String> {
    let id = id.clone();
    demo.wait(ix, limit, &format!("call {want:?}"), move |desk, host| {
        desk.store(host, |s| s.call(&id).map(|c| c.state)) == Some(want)
    })
}

/// Alice places a call from the conversation header; both desktops see it
/// (ringing out, ringing in). Returns the call.
fn place<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    notes: &mut Notes,
) -> Result<CallId, String> {
    let limit = demo.config.limits.answer;
    let context = demo.conversation.clone();
    for ix in [0, 1] {
        demo.sides[ix].desk()?.show_context(demo.host, &context)?;
    }
    let before = current(demo, 0).map(|(id, _)| id);
    demo.wait(0, limit, "the call button to be enabled", |desk, host| {
        desk.exists(host, "conversation-call")
            && desk.store(host, |s| {
                s.unsupported_reason(ramflux_desktop_core::methods::CALL_INVITE).is_none()
                    && !selectors::call_under_way(s)
            })
    })?;
    demo.sides[0].desk()?.click(demo.host, None, "conversation-call")?;
    let started = Instant::now();
    demo.wait(0, limit, "the call to ring out", |desk, host| {
        desk.store(host, |s| {
            selectors::current_call(s).is_some_and(|c| c.state == CallState::OutgoingRinging)
        })
    })?;
    let (id, _) = current(demo, 0).ok_or("no call on Alice's desktop")?;
    if Some(&id) == before.as_ref() {
        return Err("the call button placed no new call".into());
    }
    demo.wait(1, limit, "the call to ring in", |desk, host| {
        desk.store(host, |s| {
            selectors::ringing_calls(s).iter().any(|c| c.state == CallState::IncomingRinging)
        })
    })?;
    let ring_id = demo.read(1, |s| selectors::ringing_calls(s).first().map(|c| c.id.clone()))?;
    if ring_id.as_ref() != Some(&id) {
        return Err(format!("Bob rings for {ring_id:?}, Alice placed {id}"));
    }
    if id.as_str().starts_with('-') {
        return Err(format!("call id {id} starts with '-'"));
    }
    notes.add(format!("call {id} rings on Bob's desktop {:?} after the click", started.elapsed()));
    Ok(id)
}

/// Step: a call answered, audio both ways through the relay, hung up.
pub fn answered<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    audio: &[Rc<ToneAudio>; 2],
    notes: &mut Notes,
) -> Result<(), String> {
    let limit = demo.config.limits.answer;
    let id = place(demo, notes)?;
    demo.host.advance(200);
    demo.shots(0, "11_calling", notes)?;
    demo.shots(1, "11_incoming", notes)?;
    if audio[1].report().rings == 0 {
        return Err("Bob's desktop did not ring".into());
    }
    demo.sides[1].desk()?.click(demo.host, None, "incoming-answer")?;
    let answered = Instant::now();
    for ix in [1, 0] {
        wait_state(demo, ix, &id, CallState::Active, limit)?;
    }
    notes.add(format!("both daemons report the call active {:?} after Answer", answered.elapsed()));
    let names = ["Alice", "Bob"];
    demo.wait(0, Duration::from_secs(30), "decoded tones both ways", |_, _| {
        audio.iter().all(|a| a.report().peer_tone >= 100)
    })
    .map_err(|e| format!("{e}: Alice {:?} / Bob {:?}", audio[0].report(), audio[1].report()))?;
    notes.add(format!("first tones decoded both ways {:?} after Answer", answered.elapsed()));
    // Let audio run, then read the bars (quality) and the engines.
    let run = Instant::now();
    while run.elapsed() < Duration::from_secs(8) {
        demo.host.tick();
    }
    demo.shots(0, "12_in_call", notes)?;
    demo.shots(1, "12_in_call", notes)?;
    for ix in [0, 1] {
        let report = audio[ix].report();
        let quality = demo.sides[ix].desk()?.label(demo.host, "call-bar-quality");
        let sessions: Vec<_> = report.sessions.iter().filter(|(c, ..)| c == id.as_str()).collect();
        if sessions.len() != 1 {
            return Err(format!("{}: {} media sessions for one call", names[ix], sessions.len()));
        }
        let (_, kid, side) = sessions[0];
        if (*kid, *side) != (ix as u64, Some(ix as u8)) {
            return Err(format!("{}: send KID {kid}, rebind side {side:?}", names[ix]));
        }
        if report.stats.rejected != 0 || report.peer_tone * 10 < report.loud * 9 {
            return Err(format!("{}: {report:?}", names[ix]));
        }
        notes.add(format!(
            "{}: KID {kid}, side {side:?}; sent {}, played {} (recovered {}, concealed {}), \
             {} loud frames of which {} at the peer's tone; rejected {}, foreign {}, jitter \
             {:.1} ms; bar quality {quality:?}",
            names[ix],
            report.stats.sent,
            report.stats.played,
            report.stats.recovered,
            report.stats.concealed,
            report.loud,
            report.peer_tone,
            report.stats.rejected,
            report.stats.foreign,
            report.stats.jitter.jitter_ms,
        ));
    }
    demo.sides[0].desk()?.click(demo.host, None, "call-hang-up")?;
    for ix in [0, 1] {
        wait_state(demo, ix, &id, CallState::Ended(CallEndReason::Completed), limit)?;
    }
    let duration = demo.read(0, |s| s.call(&id).and_then(|c| c.outcome()))?;
    notes.add(format!("hung up by Alice: completed on both sides, Alice's record {duration:?}"));
    demo.host.advance(200);
    demo.shots(0, "13_ended", notes)?;
    demo.shots(1, "13_ended", notes)?;
    Ok(())
}

/// Step: a call declined by Bob.
pub fn declined<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    audio: &[Rc<ToneAudio>; 2],
    notes: &mut Notes,
) -> Result<(), String> {
    let limit = demo.config.limits.answer;
    let id = place(demo, notes)?;
    demo.sides[1].desk()?.click(demo.host, None, "incoming-decline")?;
    for ix in [1, 0] {
        wait_state(demo, ix, &id, CallState::Ended(CallEndReason::Declined), limit)?;
    }
    if audio.iter().any(|a| a.report().sessions.iter().any(|(c, ..)| c == id.as_str())) {
        return Err("a declined call started audio".into());
    }
    notes.add("declined on both sides; no media session asked for");
    demo.host.advance(200);
    demo.shots(0, "14_declined", notes)?;
    demo.shots(1, "14_declined", notes)?;
    Ok(())
}

/// Step: a call nobody answers within the 45-second ring timeout.
pub fn unanswered<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    notes: &mut Notes,
) -> Result<(), String> {
    let id = place(demo, notes)?;
    let rang = Instant::now();
    let limit = Duration::from_secs(90);
    wait_state(demo, 0, &id, CallState::Ended(CallEndReason::Timeout), limit)?;
    let caller = rang.elapsed();
    wait_state(demo, 1, &id, CallState::Ended(CallEndReason::Missed), limit)?;
    let outcome = demo.read(1, |s| s.call(&id).and_then(|c| c.outcome()))?;
    notes.add(format!(
        "Alice: timeout after {caller:?} of ringing; Bob: missed after {:?}, record {outcome:?}",
        rang.elapsed()
    ));
    if state(demo, 1, &id) != Some(CallState::Ended(CallEndReason::Missed)) {
        return Err("Bob's call is not missed".into());
    }
    demo.host.advance(200);
    demo.shots(0, "15_no_answer", notes)?;
    demo.shots(1, "15_missed", notes)?;
    Ok(())
}
