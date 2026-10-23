// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Contract methods the daemon cannot perform yet, and why.
//!
//! The source declares this table once on attach
//! (`DomainEvent::CapabilitiesUpdated`), so policy refuses these intents and
//! the UI shows their controls disabled with the reason. Each reason is a
//! locale key (`backend.gap.*`) the UI translates. An effect that reaches
//! the source anyway fails with `FailureCode::UnsupportedByBackend`; nothing
//! is faked.

use std::collections::BTreeMap;

use ramflux_desktop_core::{Capabilities, ContextKind, methods};

/// Reason keys, one per gap.
pub mod reason {
    /// Tasks need the intent and task API (K3).
    pub const TASKS: &str = "backend.gap.tasks";
    /// Agent surfaces need A2UI bound to conversations and streaming (K2).
    pub const SURFACES: &str = "backend.gap.surfaces";
    /// Handoff (K7).
    pub const HANDOFF: &str = "backend.gap.handoff";
    /// CAS conflict objects (K8).
    pub const CONFLICTS: &str = "backend.gap.conflicts";
    /// Privacy reveal and its audit (K9).
    pub const REVEAL: &str = "backend.gap.reveal";
    /// Direct sends need the conversation route, not listed by the daemon.
    pub const DIRECT_ROUTE: &str = "backend.gap.direct_route";
    /// Objects carry no listable metadata (name, size, direction).
    pub const OBJECTS: &str = "backend.gap.objects";
    /// Contact requests are not delivered to clients as events.
    pub const CONTACT_REQUESTS: &str = "backend.gap.contact_requests";
    /// Verification needs the contact's identity commitment.
    pub const SAFETY: &str = "backend.gap.safety";
    /// Adding or inviting members needs their keys, which cannot be looked up.
    pub const MEMBER_KEYS: &str = "backend.gap.member_keys";
    /// Joining by activation code has no daemon counterpart.
    pub const ACTIVATION: &str = "backend.gap.activation";
    /// Device sync needs a relay route and a target device.
    pub const DEVICE_SYNC: &str = "backend.gap.device_sync";
    /// Backups need a passphrase the desktop does not collect yet.
    pub const BACKUP_PASSPHRASE: &str = "backend.gap.backup_passphrase";
    /// The guest role has no daemon counterpart.
    pub const GUEST_ROLE: &str = "backend.gap.guest_role";
}

/// Every unsupported contract method (or `method@ContextKind`) with its reason.
pub fn unsupported() -> BTreeMap<String, &'static str> {
    let mut table: BTreeMap<String, &'static str> = [
        (methods::TASK_START, reason::TASKS),
        (methods::TASK_CANCEL, reason::TASKS),
        (methods::SURFACE_ACTION, reason::SURFACES),
        (methods::HANDOFF_ACCEPT, reason::HANDOFF),
        (methods::HANDOFF_DECLINE, reason::HANDOFF),
        (methods::CONFLICT_RESOLVE, reason::CONFLICTS),
        (methods::PRIVACY_REVEAL, reason::REVEAL),
        (methods::ATTACH_OBJECT, reason::OBJECTS),
        (methods::OBJECT_PULL, reason::OBJECTS),
        (methods::OBJECT_SAVE, reason::OBJECTS),
        (methods::OBJECT_RETRY, reason::OBJECTS),
        (methods::CONTACT_ACCEPT, reason::CONTACT_REQUESTS),
        (methods::CONTACT_REJECT, reason::CONTACT_REQUESTS),
        (methods::SAFETY_VERIFY, reason::SAFETY),
        (methods::GROUP_ADD_MEMBER, reason::MEMBER_KEYS),
        (methods::GROUP_CREATE_INVITE, reason::MEMBER_KEYS),
        (methods::DEVICE_ACTIVATE, reason::ACTIVATION),
        (methods::DEVICE_SYNC_EXPORT, reason::DEVICE_SYNC),
        (methods::DEVICE_SYNC_IMPORT, reason::DEVICE_SYNC),
        (methods::ACCOUNT_BACKUP_EXPORT, reason::BACKUP_PASSPHRASE),
    ]
    .into_iter()
    .map(|(method, why)| (method.to_owned(), why))
    .collect();
    table.insert(
        methods::in_context(methods::SEND_MESSAGE, ContextKind::Direct),
        reason::DIRECT_ROUTE,
    );
    table.insert(
        methods::in_context(methods::SEND_MESSAGE, ContextKind::Space),
        reason::DIRECT_ROUTE,
    );
    table
}

/// The capabilities event payload.
pub fn capabilities() -> Capabilities {
    Capabilities {
        unsupported: unsupported().into_iter().map(|(m, r)| (m, r.to_owned())).collect(),
    }
}

/// Why `method` is unsupported, if it is.
pub fn reason_for(method: &str) -> Option<&'static str> {
    unsupported().get(method).copied()
}
