// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Agent surface blocks and actions (declarative catalog, never code).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::grant::Risk;
use crate::ids::BlockId;

/// Catalog of block kinds a surface may contain.
///
/// Unknown kinds deserialize to [`BlockKind::Fallback`] and never fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// Fallback for unsupported or unknown blocks.
    #[serde(other)]
    Fallback,
}

/// One block of an agent surface. `data` is kind-specific and untrusted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    /// Block id, unique within its surface.
    pub id: BlockId,
    /// Kind.
    pub kind: BlockKind,
    /// Kind-specific data.
    #[serde(default)]
    pub data: Value,
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
