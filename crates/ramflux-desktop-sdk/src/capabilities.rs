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
}

/// The voice call methods, which need the daemon's call API (CALL-01).
pub const CALL_METHODS: [&str; 4] =
    [methods::CALL_INVITE, methods::CALL_ANSWER, methods::CALL_REJECT, methods::CALL_HANGUP];

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

/// The capabilities event payload; without the call API (`call_api`
/// false) the call methods are unsupported too.
pub fn capabilities(call_api: bool) -> Capabilities {
    let mut unsupported: BTreeMap<String, String> =
        unsupported().into_iter().map(|(m, r)| (m, r.to_owned())).collect();
    if !call_api {
        for method in CALL_METHODS {
            unsupported.insert(method.to_owned(), reason::CALLS.to_owned());
        }
    }
    Capabilities { unsupported }
}

/// Why `method` is unsupported, if it is.
pub fn reason_for(method: &str) -> Option<&'static str> {
    unsupported().get(method).copied()
}
