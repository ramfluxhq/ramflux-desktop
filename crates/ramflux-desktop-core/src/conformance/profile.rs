// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The renderer profile: what one renderer can show.
//!
//! A renderer builds its profile from its own block registry, so the
//! profile never claims a kind the renderer cannot draw. Sources may send it
//! to the peer that produces surfaces (capability negotiation).

use serde::{Deserialize, Serialize};

use crate::model::BlockKind;

/// The kind of device a renderer runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceClass {
    /// A desktop computer with a pointer and keyboard.
    Desktop,
}

/// How the user gives input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum InputMode {
    /// Mouse or trackpad.
    Pointer,
    /// Keyboard.
    Keyboard,
}

/// How the renderer presents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum OutputMode {
    /// On screen.
    Visual,
    /// As a system notification.
    Notification,
}

/// Where a surface may appear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum SurfaceMode {
    /// Inline in the conversation.
    Inline,
    /// In a separate, resizable window.
    ResizableWindow,
    /// As a notification.
    Notification,
    /// Beside the conversation.
    Split,
}

/// Which limit a surface exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LimitKind {
    /// Too many blocks.
    Blocks,
    /// Too many actions.
    Actions,
    /// A text field is too long.
    TextLength,
    /// The block data is too large.
    DataBytes,
}

/// Bounds on one surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    /// Blocks per surface.
    pub max_blocks: usize,
    /// Actions per surface.
    pub max_actions: usize,
    /// Characters in any one string (block data, labels, fallback text).
    pub max_text_chars: usize,
    /// Bytes of all block data, canonically encoded.
    pub max_data_bytes: usize,
}

impl Limits {
    /// The desktop renderer's limits.
    pub const DESKTOP: Self =
        Self { max_blocks: 64, max_actions: 16, max_text_chars: 16_384, max_data_bytes: 65_536 };
}

impl Default for Limits {
    fn default() -> Self {
        Self::DESKTOP
    }
}

/// One block kind a renderer draws, with the data versions it reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockSupport {
    /// Kind.
    pub kind: BlockKind,
    /// Supported data versions.
    pub versions: Vec<u32>,
}

/// What one renderer can show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RendererProfile {
    /// Device class.
    pub device_class: DeviceClass,
    /// Renderer runtime name.
    pub runtime: String,
    /// Block kinds and versions, in registry order.
    pub blocks: Vec<BlockSupport>,
    /// Input modes.
    pub input: Vec<InputMode>,
    /// Output modes.
    pub output: Vec<OutputMode>,
    /// Surface modes.
    pub surface_modes: Vec<SurfaceMode>,
    /// Limits.
    pub limits: Limits,
}

impl RendererProfile {
    /// Whether the profile draws `kind` at data `version`.
    pub fn supports(&self, kind: BlockKind, version: u32) -> bool {
        self.blocks.iter().any(|b| b.kind == kind && b.versions.contains(&version))
    }
}
