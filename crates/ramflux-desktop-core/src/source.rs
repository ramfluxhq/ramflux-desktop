// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The runtime-agnostic source abstraction.
//!
//! A source executes effects and produces domain events. A scenario-driven
//! mock and an SDK-backed implementation both satisfy this trait; neither the
//! trait nor core depends on an async runtime.

use crate::conformance::RendererProfile;
use crate::event::EventEnvelope;
use crate::ids::{CallId, ContextId, MomentId, Timestamp};
use crate::intent::Effect;
use crate::model::CallMedia;

/// Executes effects and yields sequenced domain events.
pub trait Source {
    /// Accepts one effect for execution. Results arrive later through [`Source::drain`].
    fn submit(&mut self, effect: Effect);

    /// Returns every envelope due at or before `now`, in sequence order.
    fn drain(&mut self, now: Timestamp) -> Vec<EventEnvelope>;

    /// The context the user is looking at changed. A source that pulls
    /// (instead of being pushed) keeps that context fresh; others ignore it.
    fn focus(&mut self, _context: Option<&ContextId>) {}

    /// A view shows these message moments but their bodies are no longer in
    /// the store's bounded body cache. A source that can deliver them again
    /// does so (as `MomentUpdated` carrying the message payload); others
    /// ignore it, and the view keeps its placeholder.
    fn request_bodies(&mut self, _moments: &[MomentId]) {}

    /// What the renderer can show (capability negotiation). Given once when
    /// the source is attached; a source that talks to surface producers
    /// sends it on, others keep or ignore it.
    fn renderer_profile(&mut self, _profile: &RendererProfile) {}

    /// The media engine needs the media session of `call` (relay
    /// endpoint, allocation token, SFrame keys). It is secret, so it never
    /// travels as an event: it arrives through [`Source::take_call_media`].
    /// A source without calls ignores it.
    fn request_call_media(&mut self, _call: &CallId) {}

    /// A media session asked for with [`Source::request_call_media`] that
    /// has arrived, or why it could not be had (a locale key, as effect
    /// failures carry). `None` while nothing arrived.
    fn take_call_media(&mut self) -> Option<(CallId, Result<CallMedia, String>)> {
        None
    }
}
