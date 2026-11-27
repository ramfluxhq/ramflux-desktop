// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The content hash of a surface version, as its source reports it.
//!
//! The desktop does not hash surfaces itself. The surface protocol defines
//! one snapshot hash (canonical JSON hashed under the protocol's own domain),
//! and the daemon that applies surface events computes it; the SDK source
//! hands it over with every version. Keeping a second encoder here would
//! give two hashes that can disagree, and an action bound to the wrong one
//! would be refused by the surface's author.
//!
//! A source that has no content hash for a surface (a scenario fixture, an
//! unversioned surface payload) leaves it to the store, which labels the
//! version with [`SurfaceHash::of_version`]. That label binds actions to the
//! version number only; it never claims to be a content hash.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The hash (or, without one, the version label) of one surface version.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SurfaceHash(String);

impl SurfaceHash {
    /// Wraps a hash the source reported.
    pub fn new(hash: impl Into<String>) -> Self {
        Self(hash.into())
    }

    /// The label of a surface version whose source reported no content
    /// hash: `v<version>`.
    pub fn of_version(version: u64) -> Self {
        Self(format!("v{version}"))
    }

    /// The hash text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A short form for display (first 12 characters after any
    /// `algorithm:` prefix).
    pub fn short(&self) -> &str {
        let text = self.0.split_once(':').map_or(self.0.as_str(), |(_, rest)| rest);
        text.get(..12).unwrap_or(text)
    }
}

impl fmt::Debug for SurfaceHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SurfaceHash({})", self.short())
    }
}

impl fmt::Display for SurfaceHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_skips_an_algorithm_prefix() {
        assert_eq!(SurfaceHash::new("sha256:0123456789abcdef").short(), "0123456789ab");
        assert_eq!(SurfaceHash::new("Qx3-_abcdefghijk").short(), "Qx3-_abcdefg");
        assert_eq!(SurfaceHash::of_version(3).as_str(), "v3");
    }
}
