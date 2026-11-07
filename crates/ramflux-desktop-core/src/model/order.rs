// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The order key of a moment inside one conversation stream.

use serde::{Deserialize, Serialize};

/// Where a moment sits in its conversation, as the daemon orders it: the
/// hybrid logical clock first, then the authoring device, then that device's
/// own sequence. The derived ordering compares the fields in that order.
///
/// This is the only thing that orders a stream. A moment's display time
/// ([`Moment::at`](super::Moment::at)) is shown, never sorted by.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OrderKey {
    /// Hybrid logical clock of the event.
    pub hlc: u64,
    /// Qualified author device, as the daemon names it.
    pub author: String,
    /// The author device's own event sequence.
    pub device_sequence: u64,
}
