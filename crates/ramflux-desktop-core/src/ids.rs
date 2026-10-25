// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Identifier newtypes and the contract timestamp.
//!
//! Every id is an opaque string serialized transparently. Distinct types keep
//! a `MomentId` from being passed where a `ContextId` is expected.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! string_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Wraps a raw identifier string.
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            /// Returns the raw identifier string.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(id: &str) -> Self {
                Self(id.to_owned())
            }
        }

        impl From<String> for $name {
            fn from(id: String) -> Self {
                Self(id)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_id!(
    /// A local account (identity) known to the desktop client.
    AccountId
);
string_id!(
    /// A Ramflux node.
    NodeId
);
string_id!(
    /// A participant: human, agent, device, or node.
    ParticipantId
);
string_id!(
    /// A conversation context (direct, group, or space).
    ContextId
);
string_id!(
    /// A moment inside a context timeline.
    MomentId
);
string_id!(
    /// A transferable object (file, blob).
    ObjectId
);
string_id!(
    /// An authorization grant awaiting or holding a decision.
    GrantId
);
string_id!(
    /// A multi-step task started from an intent.
    TaskId
);
string_id!(
    /// An agent surface (declarative UI payload).
    SurfaceId
);
string_id!(
    /// One of the account's devices.
    DeviceId
);
string_id!(
    /// A standing approval (a pre-authorized, bounded grant policy).
    StandingApprovalId
);
string_id!(
    /// A block inside an agent surface.
    BlockId
);
string_id!(
    /// A step of a task, unique within its task.
    StepId
);

impl ParticipantId {
    /// A short form of the id for display when no name is known: a
    /// `kind-` prefix is kept with the first eight characters after it
    /// (`principal-c786a534…`); any other long id keeps its first twelve.
    /// Short ids are returned unchanged.
    pub fn short(&self) -> String {
        const KEEP: usize = 8;
        let id = self.as_str();
        let (prefix, rest) = match id.split_once('-') {
            Some((prefix, rest))
                if !prefix.is_empty()
                    && prefix.len() <= 12
                    && prefix.chars().all(|c| c.is_ascii_alphabetic()) =>
            {
                (&id[..=prefix.len()], rest)
            }
            _ => ("", id),
        };
        let keep = if prefix.is_empty() { 12 } else { KEEP };
        if rest.chars().count() <= keep + 2 {
            return id.to_owned();
        }
        let head: String = rest.chars().take(keep).collect();
        format!("{prefix}{head}\u{2026}")
    }
}

/// A point in time as unix milliseconds.
///
/// Core never reads the wall clock; every timestamp is passed in by a source.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct Timestamp(pub i64);

impl Timestamp {
    /// Returns the raw unix milliseconds.
    pub const fn millis(self) -> i64 {
        self.0
    }

    /// Returns this timestamp moved forward by `ms`, saturating at `i64::MAX`.
    pub fn saturating_add_ms(self, ms: u64) -> Self {
        let delta = i64::try_from(ms).unwrap_or(i64::MAX);
        Self(self.0.saturating_add(delta))
    }

    /// Returns the non-negative distance from `earlier` to `self` in ms.
    pub fn saturating_since(self, earlier: Timestamp) -> u64 {
        u64::try_from(self.0.saturating_sub(earlier.0)).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_participant_id_is_shortened_for_display() {
        let id = ParticipantId::new("principal-c786a534839ad12fef6dd00033a374ef");
        assert_eq!(id.short(), "principal-c786a534\u{2026}");
        assert_eq!(ParticipantId::new("p-orbit").short(), "p-orbit");
        assert_eq!(
            ParticipantId::new("device-5e9182ab04cc1e9505bea48d137ab46c").short(),
            "device-5e9182ab\u{2026}"
        );
        assert_eq!(ParticipantId::new("5e9182ab04cc1e9505bea48d").short(), "5e9182ab04cc\u{2026}");
    }

    #[test]
    fn ids_serialize_as_plain_strings() {
        let id = MomentId::new("m-1");
        assert_eq!(serde_json::to_string(&id).ok().as_deref(), Some("\"m-1\""));
        assert_eq!(serde_json::to_string(&Timestamp(42)).ok().as_deref(), Some("42"));
    }

    #[test]
    fn timestamp_arithmetic_saturates() {
        assert_eq!(Timestamp(i64::MAX - 1).saturating_add_ms(10), Timestamp(i64::MAX));
        assert_eq!(Timestamp(5).saturating_since(Timestamp(10)), 0);
        assert_eq!(Timestamp(10).saturating_since(Timestamp(4)), 6);
    }
}
