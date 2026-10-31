// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! AI participants the owner custodies (DESK-AGENT-01).
//!
//! A headless AI account is created in a ceremony run by its custodian, a
//! person's account on the same daemon, and later started by the agent host
//! from a one-shot credentials bundle the custodian exports. The desktop
//! never unlocks such an account and never sees the data passphrase the
//! daemon made for it: only the typed outcome of the two requests lives
//! here.

use serde::{Deserialize, Serialize};

use crate::ids::AccountId;

/// Why an account cannot be the custodian of an AI participant (or export
/// its credentials): the daemon accepts only an unlocked, connected
/// person's account that exists on that daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentCustodyProblem {
    /// The daemon does not list the account.
    Unknown,
    /// The account is locked: the owner must unlock it first.
    Locked,
    /// The account is not a person's (an AI participant cannot custody
    /// another).
    NotPerson,
    /// The account is not connected to its node yet.
    NotConnected,
}

/// Why the daemon (or the desktop, before asking) refused to export the
/// credentials of an AI participant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentExportRefusal {
    /// No location was chosen, or it is not an absolute path.
    NoLocation,
    /// The daemon does not take the request from this session: the owner's
    /// account is not the unlocked, attended account of this daemon.
    NotOwnerSession,
    /// The owner's account is locked.
    OwnerLocked,
    /// The owner's account is not the custodian of this AI participant, or
    /// the account is not an AI participant.
    NotCustodian,
    /// The owner's or the AI participant's account is not connected.
    NotConnected,
    /// A file is already at the location: the daemon never replaces one.
    FileExists,
    /// The location cannot be written (no permission, a folder, read-only).
    LocationDenied,
    /// The folder of the location does not exist.
    LocationMissing,
    /// The credentials are no longer held: they were exported once already
    /// (the export is one-shot), or the vault does not hold them.
    CredentialsGone,
    /// The daemon did not answer in time.
    Timeout,
    /// The connection to the daemon broke.
    Transport,
    /// A refusal this desktop has no wording for.
    Other,
}

/// Where an export is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentExportPhase {
    /// The daemon wrote the bundle at the location (mode 0600, once).
    Exported,
    /// The export was refused or failed; nothing was written.
    Refused(AgentExportRefusal),
}

/// The last export attempt for an AI participant: the path is the
/// location the owner chose, never any credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentExport {
    /// The AI participant.
    pub account_id: AccountId,
    /// The location the owner chose.
    pub path: String,
    /// What came of it.
    pub phase: AgentExportPhase,
}
