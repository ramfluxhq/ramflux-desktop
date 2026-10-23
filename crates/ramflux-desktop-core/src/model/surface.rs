// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Agent surface blocks and actions (declarative catalog, never code).

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::grant::Risk;
use crate::ids::BlockId;

/// Catalog of block kinds a surface may contain.
///
/// Unknown kinds deserialize to [`BlockKind::Unsupported`] and never fail;
/// the renderer shows them as a fallback and records a renderer error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BlockKind {
    /// Plain text.
    Text,
    /// Markdown document (rendered without scripts).
    Markdown,
    /// Heading.
    Heading,
    /// Key/value pairs.
    KeyValue,
    /// Single metric.
    Metric,
    /// List of items.
    List,
    /// Table rendered in fallback form.
    TableFallback,
    /// Form container.
    Form,
    /// Text input.
    Input,
    /// Selection input.
    Select,
    /// Progress indicator.
    Progress,
    /// Streaming log lines.
    LogStream,
    /// Warning callout.
    Warning,
    /// Error callout.
    Error,
    /// Success callout.
    Success,
    /// Approval request bound to a grant.
    Approval,
    /// Group of actions.
    ActionGroup,
    /// File preview.
    FilePreview,
    /// An explicit fallback block (the surface's lower-capability text).
    Fallback,
    /// A kind outside this catalog. Never sent; produced by deserialization.
    #[serde(other)]
    Unsupported,
}

/// The block version a sender means when it names none.
pub const DEFAULT_BLOCK_VERSION: u32 = 1;

const fn default_block_version() -> u32 {
    DEFAULT_BLOCK_VERSION
}

/// One block of an agent surface. `data` is kind-specific and untrusted.
///
/// A redacted block is display-blocked: its data is never rendered, never
/// copied or exported, and never printed by `Debug`.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    /// Block id, unique within its surface.
    pub id: BlockId,
    /// Kind.
    pub kind: BlockKind,
    /// Version of the kind's data shape.
    #[serde(default = "default_block_version")]
    pub version: u32,
    /// The block's privacy forbids displaying it here.
    #[serde(default)]
    pub redacted: bool,
    /// Kind-specific data.
    #[serde(default)]
    pub data: Value,
}

impl Block {
    /// A version-1, unredacted block.
    pub fn new(id: impl Into<BlockId>, kind: BlockKind, data: Value) -> Self {
        Self { id: id.into(), kind, version: DEFAULT_BLOCK_VERSION, redacted: false, data }
    }
}

impl fmt::Debug for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = f.debug_struct("Block");
        out.field("id", &self.id)
            .field("kind", &self.kind)
            .field("version", &self.version)
            .field("redacted", &self.redacted);
        if self.redacted {
            out.field("data", &"[redacted]");
        } else {
            out.field("data", &self.data);
        }
        out.finish()
    }
}

/// An action offered by a surface. Invoking it becomes an intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceAction {
    /// Action id, unique within its surface.
    pub id: String,
    /// Button label.
    pub label: String,
    /// Declared risk.
    #[serde(default)]
    pub risk: Risk,
    /// When set, the action is shown disabled with this reason.
    #[serde(default)]
    pub disabled_reason: Option<String>,
}
