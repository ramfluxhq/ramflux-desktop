// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! One frame's way out and one packet's way in, without I/O.
//!
//! Out: PCM frame → Opus → RTP header → SFrame (the RTP header is the
//! authenticated metadata) → relay framing.
//! In: datagram → RTP header → SFrame decryption (with that header as
//! metadata) → the Opus packet, for the jitter buffer.
//!
//! The packet on the wire is `rtp_header || sframe_header || ciphertext ||
//! tag`: the relay sees sequence numbers and timestamps, never audio.

use crate::codec::{CodecError, Encoder, FRAME_SAMPLES, Frame};
use crate::relay;
use crate::rtp::{self, OPUS_PAYLOAD_TYPE, RtpHeader};
use crate::session::MediaSession;
use crate::sframe::{self, SframeError};

/// Why a frame or packet could not pass.
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    /// The codec failed.
    #[error(transparent)]
    Codec(#[from] CodecError),
    /// SFrame refused (unknown suite or key, failed authentication, replay).
    #[error(transparent)]
    Sframe(#[from] SframeError),
    /// Not a minimal RTP packet with the Opus payload type.
    #[error("not an RTP packet of this call")]
    NotRtp,
}

/// The sending half of a call.
#[derive(Debug)]
pub struct Outbound {
    encoder: Encoder,
    sframe: sframe::Sender,
    token: Vec<u8>,
    ssrc: u32,
    sequence: u16,
    timestamp: u32,
    marker: bool,
}

impl Outbound {
    /// The sending half for `session`, with this stream's `ssrc` and
    /// initial sequence number and timestamp (random in a real call, RFC
    /// 3550 §5.1).
    ///
    /// # Errors
    ///
    /// An unknown suite or empty key, or libopus refused to start.
    pub fn new(
        session: &MediaSession,
        ssrc: u32,
        sequence: u16,
        timestamp: u32,
    ) -> Result<Self, PipelineError> {
        let suite = sframe::suite(session.suite())?;
        let key = session.send_key();
        Ok(Self {
            encoder: Encoder::new()?,
            sframe: sframe::Sender::new(suite, key.kid(), key.base_key().bytes())?,
            token: session.token().to_vec(),
            ssrc,
            sequence,
            timestamp,
            marker: true,
        })
    }

    /// The datagram carrying one frame.
    ///
    /// # Errors
    ///
    /// The codec or SFrame failed.
    pub fn packet(&mut self, frame: &Frame) -> Result<Vec<u8>, PipelineError> {
        let opus = self.encoder.encode(frame)?;
        let header = RtpHeader {
            marker: std::mem::take(&mut self.marker),
            payload_type: OPUS_PAYLOAD_TYPE,
            sequence: self.sequence,
            timestamp: self.timestamp,
            ssrc: self.ssrc,
        }
        .encode();
        self.sequence = self.sequence.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(FRAME_SAMPLES as u32);
        let protected = self.sframe.encrypt(&header, &opus)?;
        let mut packet = Vec::with_capacity(header.len() + protected.len());
        packet.extend_from_slice(&header);
        packet.extend_from_slice(&protected);
        Ok(relay::frame(&self.token, &packet))
    }

    /// A datagram that only keeps the relay binding alive (an empty
    /// packet: the peer drops it). `None` without a relay.
    pub fn keepalive(&self) -> Option<Vec<u8>> {
        (!self.token.is_empty()).then(|| relay::frame(&self.token, &[]))
    }
}

impl Drop for Outbound {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.token);
    }
}

/// One accepted packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received {
    /// Its RTP header.
    pub header: RtpHeader,
    /// The decrypted Opus packet.
    pub opus: Vec<u8>,
}

/// The receiving half of a call.
#[derive(Debug)]
pub struct Inbound {
    sframe: sframe::Receiver,
}

impl Inbound {
    /// The receiving half for `session`.
    ///
    /// # Errors
    ///
    /// An unknown suite or empty key.
    pub fn new(session: &MediaSession) -> Result<Self, PipelineError> {
        let suite = sframe::suite(session.suite())?;
        let key = session.receive_key();
        let mut receiver = sframe::Receiver::new();
        receiver.add_key(suite, key.kid(), key.base_key().bytes())?;
        Ok(Self { sframe: receiver })
    }

    /// Opens one packet (as the relay delivers it: no framing).
    ///
    /// # Errors
    ///
    /// Not RTP of this call, or SFrame refused it (forged, altered,
    /// replayed or under an unknown key).
    pub fn open(&mut self, packet: &[u8]) -> Result<Received, PipelineError> {
        let header = RtpHeader::decode(packet).ok_or(PipelineError::NotRtp)?;
        if header.payload_type != OPUS_PAYLOAD_TYPE {
            return Err(PipelineError::NotRtp);
        }
        let opus = self.sframe.decrypt(&packet[..rtp::HEADER_LEN], &packet[rtp::HEADER_LEN..])?;
        Ok(Received { header, opus })
    }
}
