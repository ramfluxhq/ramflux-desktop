// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI-side view types for message schema v1 payload fields.
//!
//! These are plain display models. They carry no key material, no raw
//! envelope bytes and no store handles.

use serde::{Deserialize, Serialize};

use crate::ids::{ContextId, MomentId, ObjectId, ParticipantId};

/// Reference to the message a message replies to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyRef {
    /// Replied-to message.
    pub message_id: MomentId,
}

/// Origin of a forwarded message.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardRef {
    /// Conversation the message was forwarded from, when disclosed.
    #[serde(default)]
    pub from_context: Option<ContextId>,
    /// Original message, when disclosed.
    #[serde(default)]
    pub from_message: Option<MomentId>,
    /// Display name of the original author, when disclosed.
    #[serde(default)]
    pub from_name: Option<String>,
}

/// One reaction chip: a single grapheme token and how many reacted with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionCount {
    /// The reaction token (one grapheme).
    pub token: String,
    /// Number of distinct reactors.
    pub count: u32,
    /// The local user is among the reactors.
    #[serde(default)]
    pub mine: bool,
}

/// Formatting kind of a [`TextSpan`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpanKind {
    /// Bold text.
    Bold,
    /// Italic text.
    Italic,
    /// Underlined text.
    Underline,
    /// Struck-through text.
    Strikethrough,
    /// Inline code.
    Code,
    /// Hidden until revealed.
    Spoiler,
    /// Mention of a participant.
    Mention {
        /// Mentioned participant.
        participant: ParticipantId,
    },
    /// Link with an explicit target.
    Link {
        /// Link target as sent.
        url: String,
    },
}

/// A formatted range of the message text. Offsets count Unicode scalar
/// values, never UTF-8 bytes or UTF-16 units.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextSpan {
    /// First scalar of the range.
    pub offset: u32,
    /// Number of scalars in the range.
    pub length: u32,
    /// How the range is formatted.
    pub kind: SpanKind,
}

/// Availability of a media attachment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaState {
    /// Can be opened or downloaded.
    #[default]
    Available,
    /// The sender revoked access.
    Revoked,
    /// Not retrievable right now.
    Unavailable,
}

/// A decoded inline thumbnail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thumbnail {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Encoded image bytes.
    pub bytes: Vec<u8>,
}

/// A media row of a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaView {
    /// Backing object.
    pub object_id: ObjectId,
    /// File name shown on the row.
    #[serde(default)]
    pub name: String,
    /// Media type.
    #[serde(default)]
    pub mime: String,
    /// Size in bytes.
    #[serde(default)]
    pub size: u64,
    /// Availability.
    #[serde(default)]
    pub state: MediaState,
    /// Inline thumbnail, only when the local opt-in allowed it.
    #[serde(default)]
    pub thumbnail: Option<Thumbnail>,
}

/// Who a delete applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeleteScope {
    /// Removed for every participant.
    Everyone,
    /// Removed on the local account only.
    Local,
}

/// Why a message is shown as a placeholder instead of its content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsupportedInfo {
    /// Content kind the local build does not understand.
    pub kind: String,
    /// Layer that rejected it.
    pub layer: String,
    /// Machine-readable reason.
    pub reason: String,
    /// Length of the stored raw content in bytes.
    pub raw_length: u64,
}
