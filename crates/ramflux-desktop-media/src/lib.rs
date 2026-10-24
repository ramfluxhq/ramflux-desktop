// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The desktop's voice call media engine (DESK-F16, on the CALL-01
//! decisions).
//!
//! One 1:1 call: the microphone is captured, encoded with Opus (48 kHz
//! mono, 20 ms), put in a minimal RTP packet whose payload is end-to-end
//! encrypted with SFrame, and sent over UDP to the relay allocation the
//! daemon hands out; the peer's packets come back the same way into an
//! adaptive jitter buffer and the speaker. The relay sees sequence numbers,
//! timestamps and ciphertext, never audio or keys.
//!
//! - [`sframe`]: RFC 9605 (header, cipher suites, sender and receiver with
//!   replay protection).
//! - [`rtp`]: the minimal RTP header (RFC 3550).
//! - [`codec`]: Opus through the reference libopus (RFC 6716, RFC 7587).
//! - [`jitter`]: the adaptive jitter buffer.
//! - [`relay`]: datagram framing towards the relay allocation.
//! - [`session`]: what one call needs (endpoint, token, keys).
//! - [`pipeline`]: one frame out, one packet in, without I/O.
//! - [`engine`]: the send, receive and playout threads.
//! - [`device`]: the microphone and speaker (cpal).
//!
//! No GPUI, no async runtime, nothing from the daemon: the desktop hands a
//! [`session::MediaSession`] in and reads [`engine::Engine::stats`] back.

#![forbid(unsafe_code)]

pub mod codec;
pub mod device;
pub mod engine;
pub mod jitter;
pub mod pipeline;
pub mod relay;
pub mod rtp;
pub mod session;
pub mod sframe;

pub use engine::{Capture, Engine, EngineError, EngineStats, Playback, Quality};
pub use session::{DirectionKey, MediaSession};
pub use sframe::BaseKey;
