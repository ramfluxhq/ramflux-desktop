// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The runtime-agnostic source abstraction.
//!
//! A source executes effects and produces domain events. A scenario-driven
//! mock and an SDK-backed implementation both satisfy this trait; neither the
//! trait nor core depends on an async runtime.

use crate::event::EventEnvelope;
use crate::ids::Timestamp;
use crate::intent::Effect;

/// Executes effects and yields sequenced domain events.
pub trait Source {
    /// Accepts one effect for execution. Results arrive later through [`Source::drain`].
    fn submit(&mut self, effect: Effect);

    /// Returns every envelope due at or before `now`, in sequence order.
    fn drain(&mut self, now: Timestamp) -> Vec<EventEnvelope>;
}
