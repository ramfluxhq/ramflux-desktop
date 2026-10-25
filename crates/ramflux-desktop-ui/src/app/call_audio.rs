// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The audio side of a call, driven by the store.
//!
//! The store says where a call is; the store handle makes the audio follow
//! (on every clock tick, for the rendered account):
//!
//! - an incoming call rings (and no call is under way): the ring tone plays
//!   until it stops ringing;
//! - a call is answered and the daemon reports it active (signaling done,
//!   relay allocated): the handle asks the source for its media session,
//!   once per call, and starts the media engine with it; the session's
//!   keys go from the source to the engine and nowhere else. The daemon
//!   hands them out once and each key starts once, at counter zero: audio
//!   that fails is not started again on the same call (a fresh call brings
//!   fresh keys), and the audio stays with its call while another account
//!   is shown;
//! - the call ends (or another one takes its place): the engine stops.
//!
//! Audio is a [`CallAudio`] the application installs: the computer's
//! microphone and speaker ([`DeviceAudio`]), or a stand-in in tests. Without
//! one nothing is played or captured, and the in-call bar says so.

use std::rc::Rc;

use ramflux_desktop_core::{AccountId, CallId, CallMedia};
use ramflux_desktop_media::device::{self, DeviceChoice, Ringer, Streams};
use ramflux_desktop_media::{BaseKey, DirectionKey, Engine, MediaSession, Quality, Rebind};

/// How a call sounds, as the in-call bar shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallQuality {
    /// Nothing heard from the peer yet.
    Connecting,
    /// Little loss and jitter.
    Good,
    /// Noticeable loss or jitter.
    Fair,
    /// Heavy loss or jitter.
    Poor,
    /// Nothing heard for a few seconds.
    Lost,
}

impl From<Quality> for CallQuality {
    fn from(quality: Quality) -> Self {
        match quality {
            Quality::Connecting => Self::Connecting,
            Quality::Good => Self::Good,
            Quality::Fair => Self::Fair,
            Quality::Poor => Self::Poor,
            Quality::Lost => Self::Lost,
        }
    }
}

/// What a running call's audio reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveStatus {
    /// How it sounds.
    pub quality: CallQuality,
    /// The microphone delivers only silence (access denied).
    pub microphone_blocked: bool,
}

/// A call's running audio. Dropping it stops it.
pub trait LiveCall {
    /// Mutes or unmutes the microphone.
    fn set_muted(&mut self, muted: bool);
    /// What it reports now.
    fn status(&self) -> LiveStatus;
}

/// The audio an application provides for calls.
pub trait CallAudio {
    /// Starts a call's audio with its media session.
    ///
    /// # Errors
    ///
    /// Why the audio could not start (shown in the in-call bar).
    fn start(&self, media: CallMedia) -> Result<Box<dyn LiveCall>, String>;

    /// Starts the ring tone; dropping the guard stops it. `None` when it
    /// cannot ring (the call still shows).
    fn ring(&self) -> Option<Box<dyn std::any::Any>>;
}

/// What the in-call bar shows about the audio of the current call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallAudioView {
    /// The call.
    pub call_id: CallId,
    /// The microphone is muted.
    pub muted: bool,
    /// The running audio's report; `None` before it runs.
    pub live: Option<LiveStatus>,
    /// Why there is no audio (no audio installed, the media session or the
    /// devices failed).
    pub error: Option<String>,
}

/// The audio link of the current call.
pub(super) struct CallLink {
    pub(super) account: AccountId,
    pub(super) call_id: CallId,
    pub(super) live: Option<Box<dyn LiveCall>>,
    pub(super) muted: bool,
    pub(super) error: Option<String>,
    /// The media session was asked for (once per call, ever).
    pub(super) requested: bool,
}

impl CallLink {
    pub(super) fn new(account: AccountId, call_id: CallId) -> Self {
        Self { account, call_id, live: None, muted: false, error: None, requested: false }
    }

    pub(super) fn view(&self) -> CallAudioView {
        CallAudioView {
            call_id: self.call_id.clone(),
            muted: self.muted,
            live: self.live.as_ref().map(|live| live.status()),
            error: self.error.clone(),
        }
    }

    /// The media session arrived (or could not be had): start the audio.
    pub(super) fn media(
        &mut self,
        answer: Result<CallMedia, String>,
        audio: Option<&Rc<dyn CallAudio>>,
    ) {
        let started = match (answer, audio) {
            (Err(reason), _) => Err(reason),
            (Ok(_), None) => Err(NO_AUDIO.to_owned()),
            (Ok(media), Some(audio)) => audio.start(media),
        };
        match started {
            Ok(mut live) => {
                live.set_muted(self.muted);
                self.live = Some(live);
                self.error = None;
            }
            Err(reason) => self.error = Some(reason),
        }
    }

    pub(super) fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        if let Some(live) = self.live.as_mut() {
            live.set_muted(muted);
        }
    }
}

/// Locale key: no audio is installed (scenarios, tests).
pub const NO_AUDIO: &str = "call.audio.none";

/// The computer's default microphone and speaker.
#[derive(Debug, Default)]
pub struct DeviceAudio {
    choice: DeviceChoice,
}

impl DeviceAudio {
    /// Audio on the chosen devices (`DeviceChoice::default()`: the system's).
    pub fn new(choice: DeviceChoice) -> Self {
        Self { choice }
    }
}

struct DeviceCall {
    engine: Engine,
    _streams: Streams,
}

impl LiveCall for DeviceCall {
    fn set_muted(&mut self, muted: bool) {
        self.engine.set_muted(muted);
    }

    fn status(&self) -> LiveStatus {
        let stats = self.engine.stats();
        LiveStatus {
            quality: ramflux_desktop_media::engine::quality_of(&stats).into(),
            microphone_blocked: stats.microphone_blocked,
        }
    }
}

/// The engine's session for `media` (the keys are copied into wiped
/// buffers; `media` is dropped, and wiped, here).
pub fn media_session(media: CallMedia) -> MediaSession {
    let key = |key: &ramflux_desktop_core::CallKey| {
        DirectionKey::new(key.kid(), BaseKey::new(key.base_key().to_vec()))
    };
    let session = MediaSession::new(
        media.endpoint.clone(),
        media.token.to_vec(),
        media.suite,
        key(&media.send),
        key(&media.receive),
    );
    let rebind = media.rebind.as_ref().and_then(|rebind| {
        let key = <&[u8; 32]>::try_from(rebind.key.as_slice()).ok()?;
        Rebind::new(rebind.lease_id, rebind.side, key)
    });
    match rebind {
        Some(rebind) => session.with_rebind(rebind),
        None => session,
    }
}

impl CallAudio for DeviceAudio {
    fn start(&self, media: CallMedia) -> Result<Box<dyn LiveCall>, String> {
        let session = media_session(media);
        let (capture, playback, streams) =
            device::open(&self.choice).map_err(|error| error.to_string())?;
        let engine = Engine::start(session, Box::new(capture), Box::new(playback))
            .map_err(|error| error.to_string())?;
        Ok(Box::new(DeviceCall { engine, _streams: streams }))
    }

    fn ring(&self) -> Option<Box<dyn std::any::Any>> {
        Ringer::start(self.choice.output.as_deref())
            .ok()
            .map(|ringer| Box::new(ringer) as Box<dyn std::any::Any>)
    }
}
