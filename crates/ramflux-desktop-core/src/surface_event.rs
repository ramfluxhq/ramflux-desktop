// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Agent surface facts, as a conversation carries them.
//!
//! An agent surface is a conversation message presenting a card. The card
//! itself is drawn by the card runtime (`crate::card_event`); the surface
//! carries what the desktop shows without it: the fallback text, the
//! actions a task step's decision is answered with, and the version and
//! hash those actions are bound to.
//!
//! A source that reads the surface's current state from an authority (the
//! daemon) sends a [`SurfaceSnapshot`]: the whole state at some version,
//! which may skip versions the source did not see one by one, and says
//! whether the authority knows it is out of sync with the author.
//!
//! The store applies these events with [`crate::Store::apply`].

use serde::{Deserialize, Serialize};

use crate::ids::{ContextId, MomentId, ParticipantId, SurfaceId, Timestamp};
use crate::model::{OrderKey, SurfaceAction};
use crate::surface_hash::SurfaceHash;

/// The whole current state of a surface, as an authority holds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceSnapshot {
    /// The conversation message presenting the surface.
    pub moment_id: MomentId,
    /// Conversation.
    pub context_id: ContextId,
    /// Author.
    pub origin: ParticipantId,
    /// When the surface was posted, if the source knows (else the time the
    /// source saw it).
    #[serde(default)]
    pub at: Option<Timestamp>,
    /// Surface id.
    pub surface_id: SurfaceId,
    /// The version this state is (at least 1).
    pub version: u64,
    /// Its content hash.
    pub hash: SurfaceHash,
    /// Offered actions.
    #[serde(default)]
    pub actions: Vec<SurfaceAction>,
    /// Text shown where the card cannot be rendered. Required.
    pub fallback_text: String,
    /// The author closed it.
    #[serde(default)]
    pub closed: bool,
    /// The authority knows it missed a change of the author's: this state
    /// is not the author's current one, and actions on it are refused until
    /// it catches up.
    #[serde(default)]
    pub out_of_sync: bool,
    /// The creating event's place in the conversation stream, when the
    /// source knows it; a surface without one sorts before keyed moments.
    /// It only matters when the snapshot creates the moment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<OrderKey>,
}

/// A fact about one agent surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SurfaceEvent {
    /// The authority's current state of a surface (created when new).
    Snapshot(Box<SurfaceSnapshot>),
    /// The authority refused an action the user took on `version`: the
    /// surface had changed. Nothing was done; the user reviews it again.
    ActionRefused {
        /// Surface.
        surface_id: SurfaceId,
        /// The version the action was shown on.
        version: u64,
    },
    /// The answer to the user's request for the author's current state of
    /// an out-of-sync surface: sent, or refused with the reason.
    UpdateRequested {
        /// Surface.
        surface_id: SurfaceId,
        /// Why the request could not be sent, when it could not.
        #[serde(default)]
        refused: Option<String>,
    },
    /// The authority's last-updated time of a surface (the author's clock,
    /// display only); set after the snapshot that brought it.
    UpdatedAt {
        /// Surface.
        surface_id: SurfaceId,
        /// When the author last changed it.
        at: Timestamp,
    },
    /// The authority no longer holds the surface: its moment is removed.
    Removed {
        /// Surface.
        surface_id: SurfaceId,
    },
}

impl SurfaceEvent {
    /// The surface the event is about.
    pub fn surface_id(&self) -> &SurfaceId {
        match self {
            Self::ActionRefused { surface_id, .. }
            | Self::UpdateRequested { surface_id, .. }
            | Self::UpdatedAt { surface_id, .. }
            | Self::Removed { surface_id } => surface_id,
            Self::Snapshot(snapshot) => &snapshot.surface_id,
        }
    }
}
