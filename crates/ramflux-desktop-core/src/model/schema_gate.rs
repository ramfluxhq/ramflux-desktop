// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Whether the desktop may open an account's messages (S10c D1/D7).
//!
//! The source decides from the local daemon's reported schema generation
//! and this build's support; the store only keeps the verdict per account
//! for the screens. Nothing here opens, converts or deletes an account.

use serde::{Deserialize, Serialize};

/// The verdict for one account's message traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SchemaGate {
    /// Messages are open.
    #[default]
    Open,
    /// The daemon's answer is awaited (a new connection): the last view is
    /// kept, read-only, and nothing message-related is sent.
    Unresolved,
    /// The daemon's status could not be read in this connection: retrying.
    Failed,
    /// The local daemon predates what this app needs.
    OlderDaemon,
    /// The account is on a newer schema than this app opens.
    UpdateApp,
    /// The daemon reports a schema this app does not know.
    Unsupported,
    /// The account was made by an older development build and cannot be
    /// opened: only a new account or another account can be used.
    ResetRequired,
}

impl SchemaGate {
    /// Whether message traffic flows.
    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(self, Self::Open)
    }

    /// Whether the messages area is replaced by a blocking screen (the
    /// waiting states keep the last view).
    #[must_use]
    pub const fn blocks_view(self) -> bool {
        matches!(
            self,
            Self::OlderDaemon | Self::UpdateApp | Self::Unsupported | Self::ResetRequired
        )
    }
}
