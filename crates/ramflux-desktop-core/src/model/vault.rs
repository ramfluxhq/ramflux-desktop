// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Why the daemon's key vault refused an operation (S10c D6).
//!
//! The nine reasons the daemon names (`reason_code` on its refusals). The
//! desktop keeps its own copy of the set so that core stays free of the
//! product crates; the SDK part maps the product's reasons onto it with an
//! exhaustive match, so a tenth reason cannot compile unnoticed.

use serde::{Deserialize, Serialize};

/// A refusal reason of the key vault. No secret is ever part of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VaultReason {
    /// The installation or the requested entry does not exist.
    Missing,
    /// Initialization is incomplete; it must be completed on this device.
    InitPending,
    /// A provisioning import is incomplete; it must be retried.
    ImportPending,
    /// Stored vault state is invalid or fails its checks.
    Corrupt,
    /// Ownership or permissions are unsafe, or access was denied.
    Permission,
    /// The key provider is locked, was cancelled or timed out.
    Locked,
    /// The vault is in use; trying again shortly may work.
    Busy,
    /// This handle or role holds no primary authority.
    PrimaryUnavailable,
    /// The requested mode or role is not supported here.
    ModeMismatch,
}

impl VaultReason {
    /// Every reason, in the daemon's order.
    pub const ALL: [Self; 9] = [
        Self::Missing,
        Self::InitPending,
        Self::ImportPending,
        Self::Corrupt,
        Self::Permission,
        Self::Locked,
        Self::Busy,
        Self::PrimaryUnavailable,
        Self::ModeMismatch,
    ];

    /// The daemon's `reason_code` for it.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Missing => "vault_missing",
            Self::InitPending => "vault_init_pending",
            Self::ImportPending => "vault_import_pending",
            Self::Corrupt => "vault_corrupt",
            Self::Permission => "vault_permission",
            Self::Locked => "vault_locked",
            Self::Busy => "vault_busy",
            Self::PrimaryUnavailable => "vault_primary_unavailable",
            Self::ModeMismatch => "vault_mode_mismatch",
        }
    }

    /// The reason the daemon named, when it belongs to the error `code` that
    /// carried it (`ModeMismatch` travels as `CustodySetupInvalid`, the rest
    /// as `CustodyVaultUnavailable`). An unknown reason, or one on another
    /// code, is not a vault reason.
    #[must_use]
    pub fn from_wire(code: &str, reason_code: Option<&str>) -> Option<Self> {
        let reason_code = reason_code?;
        Self::ALL
            .into_iter()
            .find(|reason| reason.code() == reason_code && reason.wire_code() == code)
    }

    /// The error code the daemon carries the reason on.
    #[must_use]
    pub const fn wire_code(self) -> &'static str {
        match self {
            Self::ModeMismatch => "CustodySetupInvalid",
            Self::Missing
            | Self::InitPending
            | Self::ImportPending
            | Self::Corrupt
            | Self::Permission
            | Self::Locked
            | Self::Busy
            | Self::PrimaryUnavailable => "CustodyVaultUnavailable",
        }
    }

    /// Only a busy vault is worth trying again.
    #[must_use]
    pub const fn retryable(self) -> bool {
        matches!(self, Self::Busy)
    }
}
