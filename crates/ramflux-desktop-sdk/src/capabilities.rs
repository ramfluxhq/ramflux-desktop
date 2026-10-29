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
    /// Reporting renderer errors to a surface's author has no daemon
    /// counterpart (K2 carries surfaces, not renderer reports).
    pub const SURFACES: &str = "backend.gap.surfaces";
    /// Handoff (K7).
    pub const HANDOFF: &str = "backend.gap.handoff";
    /// CAS conflict objects (K8).
    pub const CONFLICTS: &str = "backend.gap.conflicts";
    /// Privacy reveal and its audit (K9).
    pub const REVEAL: &str = "backend.gap.reveal";
    /// The daemon has no spaces (long-lived shared contexts).
    pub const SPACES: &str = "backend.gap.spaces";
    /// Group messages carry no attachments on the bus.
    pub const GROUP_ATTACHMENTS: &str = "backend.gap.group_attachments";
    /// A contact request is federated only and needs a federation route to
    /// the contact's node, which no directory lists.
    pub const CONTACT_ROUTE: &str = "backend.gap.contact_route";
    /// Joining by activation code has no daemon counterpart.
    pub const ACTIVATION: &str = "backend.gap.activation";
    /// Device sync needs a relay route and a target device.
    pub const DEVICE_SYNC: &str = "backend.gap.device_sync";
    /// Voice calls need the daemon's call API (CALL-01).
    pub const CALLS: &str = "backend.gap.calls";
    /// Contact requests need the daemon's request API (CONTACT-02).
    pub const CONTACT_REQUESTS: &str = "backend.gap.contact_requests";
    /// Full backup and restore need the daemon's job API (BACKUP-01).
    pub const FULL_BACKUP: &str = "backend.gap.full_backup";
}

/// The contact request methods, which need the daemon's request API
/// (CONTACT-02).
pub const REQUEST_METHODS: [&str; 4] = [
    methods::CONTACT_REQUEST_ACCEPT,
    methods::CONTACT_REQUEST_DECLINE,
    methods::CONTACT_REQUEST_BLOCK,
    methods::CONTACT_REQUESTS_POLICY,
];

/// What the daemon was found to have.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Found {
    /// The call API (it answered `call.list`).
    pub calls: bool,
    /// The daemon has no contact requests: it refused
    /// `contact.requests.list` as a method it does not know. Any other
    /// refusal of the list (a locked account, an outage, a row it could not
    /// read) is not this.
    pub requests_missing: bool,
    /// The full backup job API (BACKUP-01): `account.backup.jobs` answered.
    pub backup: bool,
    /// The daemon reports `born_v3_creation: true` in `daemon.status`: it
    /// creates accounts with a recovery phrase and recovers them from it.
    /// An older daemon has no such field; absent means false.
    pub born_v3: bool,
}

/// The voice call methods, which need the daemon's call API (CALL-01).
pub const CALL_METHODS: [&str; 4] =
    [methods::CALL_INVITE, methods::CALL_ANSWER, methods::CALL_REJECT, methods::CALL_HANGUP];

/// The full backup and restore methods, which need the daemon's job API
/// (BACKUP-01).
pub const BACKUP_METHODS: [&str; 4] = [
    methods::BACKUP_CREATE,
    methods::RESTORE_VERIFY,
    methods::RESTORE_INSTALL,
    methods::BACKUP_CANCEL,
];

/// Every unsupported contract method (or `method@ContextKind`) with its reason.
pub fn unsupported() -> BTreeMap<String, &'static str> {
    let mut table: BTreeMap<String, &'static str> = [
        (methods::RENDERER_ERROR, reason::SURFACES),
        (methods::HANDOFF_ACCEPT, reason::HANDOFF),
        (methods::HANDOFF_DECLINE, reason::HANDOFF),
        (methods::CONFLICT_RESOLVE, reason::CONFLICTS),
        (methods::PRIVACY_REVEAL, reason::REVEAL),
        (methods::CONTACT_REQUEST, reason::CONTACT_ROUTE),
        (methods::DEVICE_ACTIVATE, reason::ACTIVATION),
        (methods::DEVICE_SYNC_EXPORT, reason::DEVICE_SYNC),
        (methods::DEVICE_SYNC_IMPORT, reason::DEVICE_SYNC),
    ]
    .into_iter()
    .map(|(method, why)| (method.to_owned(), why))
    .collect();
    table.insert(methods::in_context(methods::SEND_MESSAGE, ContextKind::Space), reason::SPACES);
    table.insert(methods::in_context(methods::ATTACH_OBJECT, ContextKind::Space), reason::SPACES);
    table.insert(
        methods::in_context(methods::ATTACH_OBJECT, ContextKind::Group),
        reason::GROUP_ATTACHMENTS,
    );
    table
}

/// The capabilities event payload; what the daemon was not `found` to
/// have (the call API, contact requests, the full backup job API) is
/// unsupported too, and creation with a recovery phrase is offered only
/// when the daemon reported it.
pub fn capabilities(found: Found) -> Capabilities {
    let mut unsupported: BTreeMap<String, String> =
        unsupported().into_iter().map(|(m, r)| (m, r.to_owned())).collect();
    if !found.calls {
        for method in CALL_METHODS {
            unsupported.insert(method.to_owned(), reason::CALLS.to_owned());
        }
    }
    if found.requests_missing {
        for method in REQUEST_METHODS {
            unsupported.insert(method.to_owned(), reason::CONTACT_REQUESTS.to_owned());
        }
    }
    if !found.backup {
        for method in BACKUP_METHODS {
            unsupported.insert(method.to_owned(), reason::FULL_BACKUP.to_owned());
        }
    }
    // Account creation with a recovery phrase is offered when the daemon
    // says so in `daemon.status` (`born_v3_creation`; absent means false).
    Capabilities { unsupported, born_v3_creation: found.born_v3 }
}

/// Why `method` is unsupported, if it is.
pub fn reason_for(method: &str) -> Option<&'static str> {
    unsupported().get(method).copied()
}
