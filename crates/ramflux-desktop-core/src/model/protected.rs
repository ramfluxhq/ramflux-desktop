// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Protected text: content the human revealed with a hold.
//!
//! It is shown in its moment and nowhere else. `Debug` prints a marker,
//! so it never reaches logs, the event inspector or a diagnostics bundle;
//! the only way to read it is [`Protected::expose`], called by the view
//! that displays it.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Revealed protected text.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Protected(String);

impl Protected {
    /// Wraps revealed text.
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// The text, for the view that displays it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Protected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Protected([redacted])")
    }
}

impl From<&str> for Protected {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}
