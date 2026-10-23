// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Transferable objects.

use serde::{Deserialize, Serialize};

use crate::ids::ObjectId;

/// Object transfer direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    /// Received.
    Inbound,
    /// Sent.
    Outbound,
}

/// Object transfer state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferState {
    /// Waiting to start (inbound objects wait for a pull).
    Queued,
    /// Bytes moving.
    Transferring {
        /// Bytes transferred so far.
        transferred: u64,
    },
    /// Finished.
    Complete,
    /// Failed.
    Failed {
        /// Failure reason.
        reason: String,
    },
}

/// A transferable object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Object {
    /// Object id.
    pub id: ObjectId,
    /// File name; `None` when the backend did not report one.
    #[serde(default)]
    pub name: Option<String>,
    /// Size in bytes; `None` when the backend did not report it.
    #[serde(default)]
    pub size: Option<u64>,
    /// Direction.
    pub direction: Direction,
    /// Transfer state.
    pub transfer: TransferState,
    /// Local path the object was saved to.
    #[serde(default)]
    pub saved_to: Option<String>,
    /// Short text preview supplied by the source (never fetched by the UI).
    #[serde(default)]
    pub preview: Option<String>,
}

impl Object {
    /// The name to show: the reported file name, else the object id.
    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.id.to_string())
    }
}

impl Object {
    /// Transfer progress from 0 to 1.
    pub fn progress(&self) -> f32 {
        match &self.transfer {
            TransferState::Queued | TransferState::Failed { .. } => 0.0,
            TransferState::Complete => 1.0,
            TransferState::Transferring { transferred } if self.size.is_some_and(|s| s > 0) => {
                let size = self.size.unwrap_or(1);
                // Lossy by design: progress is only displayed.
                #[allow(clippy::cast_precision_loss)]
                let ratio = *transferred as f64 / size as f64;
                #[allow(clippy::cast_possible_truncation)]
                let ratio = ratio.clamp(0.0, 1.0) as f32;
                ratio
            }
            TransferState::Transferring { .. } => 0.0,
        }
    }
}
