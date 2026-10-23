// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Read-only snapshots of MCP servers and pending A2I control events.
//!
//! The desktop only displays these; registration and tool calls stay in the CLI.

use serde::{Deserialize, Serialize};

/// A registered MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServer {
    /// Server name.
    pub name: String,
    /// Tool names it exposes.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Last refresh succeeded.
    #[serde(default)]
    pub healthy: bool,
}

/// A pending A2I control event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct A2iItem {
    /// Event id.
    pub id: String,
    /// Human summary.
    pub summary: String,
}

/// The integrations snapshot.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Integrations {
    /// MCP servers.
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
    /// Pending A2I events.
    #[serde(default)]
    pub a2i_pending: Vec<A2iItem>,
}
