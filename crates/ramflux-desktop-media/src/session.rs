// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! What the engine needs to carry one call: where to send (the relay
//! allocation), how to present itself there (the opaque allocation token),
//! and the SFrame keys of both directions.
//!
//! The daemon hands this out per call (`call.media.session`, CALL-01 §6).
//! Keys never leave the end-to-end encrypted channel the daemon received
//! them on and the engine's memory: they are wiped when dropped and never
//! printed.

use zeroize::Zeroizing;

use crate::sframe::BaseKey;

/// One direction's SFrame key: its KID and base key (RFC 9605 §4.4.1).
#[derive(Clone, PartialEq, Eq)]
pub struct DirectionKey {
    kid: u64,
    base_key: BaseKey,
}

impl DirectionKey {
    /// A key for `kid`.
    pub fn new(kid: u64, base_key: BaseKey) -> Self {
        Self { kid, base_key }
    }

    /// The key id.
    pub fn kid(&self) -> u64 {
        self.kid
    }

    /// The base key.
    pub fn base_key(&self) -> &BaseKey {
        &self.base_key
    }
}

impl std::fmt::Debug for DirectionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectionKey").field("kid", &self.kid).finish_non_exhaustive()
    }
}

/// The media session of one call.
#[derive(Clone, PartialEq, Eq)]
pub struct MediaSession {
    endpoint: String,
    token: Zeroizing<Vec<u8>>,
    suite: u16,
    send: DirectionKey,
    receive: DirectionKey,
}

impl MediaSession {
    /// A session sending to `endpoint` (`host:port` of the relay
    /// allocation), presenting `token` (empty: talk to the endpoint
    /// directly, without relay framing), with SFrame `suite` and the keys
    /// of both directions.
    pub fn new(
        endpoint: impl Into<String>,
        token: Vec<u8>,
        suite: u16,
        send: DirectionKey,
        receive: DirectionKey,
    ) -> Self {
        Self { endpoint: endpoint.into(), token: Zeroizing::new(token), suite, send, receive }
    }

    /// Where media is sent.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The opaque allocation token presented to the relay.
    pub fn token(&self) -> &[u8] {
        &self.token
    }

    /// The SFrame cipher suite id.
    pub fn suite(&self) -> u16 {
        self.suite
    }

    /// This side's sending key.
    pub fn send_key(&self) -> &DirectionKey {
        &self.send
    }

    /// The peer's sending key (this side decrypts with it).
    pub fn receive_key(&self) -> &DirectionKey {
        &self.receive
    }
}

impl std::fmt::Debug for MediaSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaSession")
            .field("endpoint", &self.endpoint)
            .field("suite", &self.suite)
            .field("send_kid", &self.send.kid)
            .field("receive_kid", &self.receive.kid)
            .finish_non_exhaustive()
    }
}
