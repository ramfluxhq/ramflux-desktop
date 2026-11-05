// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Networking meaning belongs to the workspace title, above transient facts.
use crate::i18n::{tr, tr_args};
use gpui_kit::SharedString;
use ramflux_desktop_core::{
    Account, FailureCode, Networking, NetworkingReason, NetworkingReasonKind as Kind,
    RegistrationRefusal as Code,
};

pub(super) fn header(
    account: Option<&Account>,
    failure: Option<&FailureCode>,
    retry_remaining: Option<u64>,
) -> (SharedString, Option<SharedString>) {
    let Some(account) = account else { return (tr("app.name"), None) };
    if account.authority_blocked {
        return (tr("network_header.authority"), None);
    }
    if let Some(failure) = failure {
        let key = match failure {
            FailureCode::Daemon(code) if code == "CustodyForkFenced" => Some("network_header.fork"),
            FailureCode::Daemon(code) if code == "OperationReconciliationRequired" => {
                Some("backend.failure.reconcile")
            }
            FailureCode::Daemon(code) if code == "AuthorityUnavailable" => {
                Some("network_header.authority")
            }
            _ => None,
        };
        if let Some(key) = key {
            return (tr(key), None);
        }
    }
    if let Some(reason) = &account.networking_reason {
        let detail = reason_text(reason, retry_remaining);
        if reason.requires_action() {
            return (detail, None);
        }
        if account.networking == Networking::Pending {
            return (tr("network_header.connecting"), Some(detail));
        }
    }
    if account.networking == Networking::Pending {
        (tr("network_header.connecting"), None)
    } else {
        (tr("app.name"), None)
    }
}

fn reason_text(reason: &NetworkingReason, retry_remaining: Option<u64>) -> SharedString {
    let key = match reason.kind {
        Kind::NoNodeSelected => "network_header.choose_node",
        Kind::NodeCustodyUnavailable => "network_header.choose_custody",
        Kind::CustodyExpired => "network_header.renew",
        Kind::NodeBusy => "network_header.busy",
        Kind::NodeUnreachable => "network_header.unreachable",
        Kind::RegistrationRefused => match reason.code.as_ref() {
            Some(Code::Busy) => "network_header.busy",
            Some(Code::Capacity) => "network_header.capacity",
            Some(Code::PersistenceUnavailable) => "network_header.persistence",
            Some(Code::SessionRequired) => "network_header.session_required",
            Some(Code::SessionExpired) => "network_header.session_expired",
            Some(Code::Unsupported | Code::UpdateRequired) => "network_header.update",
            Some(Code::AuthorityUnavailable) => "network_header.authority",
            Some(Code::PrincipalIdTaken) => "network_header.identity",
            Some(Code::ChallengeUnknown | Code::ChallengeExpired | Code::Replay) => {
                "network_header.restart"
            }
            Some(Code::InvalidInput | Code::BindingRefused) | None => "network_header.refused",
            Some(Code::HomeMigration { reason, .. }) => migration_key(reason),
        },
    };
    let mut text = tr(key).to_string();
    if let Some(ms) =
        retry_remaining.or(reason.retry_after_ms).filter(|_| !reason.requires_action())
    {
        text = tr_args(
            "network_header.retry",
            &[("reason", &text), ("seconds", &ms.div_ceil(1000).to_string())],
        )
        .to_string();
    }
    if let Some(capacity) = &reason.capacity {
        text = tr_args(
            "network_header.capacity_detail",
            &[
                ("reason", &text),
                (
                    "scope",
                    tr(match capacity.scope.as_str() {
                        "prefix_daily" => "network_header.prefix_daily",
                        "global_daily" => "network_header.global_daily",
                        _ => "network_header.capacity",
                    })
                    .as_ref(),
                ),
                (
                    "seconds",
                    &capacity
                        .resets_at_unix
                        .saturating_sub(
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs(),
                        )
                        .to_string(),
                ),
            ],
        )
        .to_string();
    }
    text.into()
}

fn migration_key(reason: &serde_json::Value) -> &'static str {
    match reason.as_str() {
        Some("v3_required" | "authority_update_required") => "network_header.update",
        Some("home_fork") => "network_header.fork",
        Some("capacity_refused" | "rate_limited") => "network_header.capacity",
        Some("storage_unavailable") => "network_header.persistence",
        Some("peering_unavailable") => "network_header.unreachable",
        Some("admission_expired" | "commit_expired") => "network_header.restart",
        Some("commit_outcome_unknown" | "idempotency_conflict") => "backend.failure.reconcile",
        Some("actor_revoked" | "pin_mismatch" | "base_mismatch") => "network_header.identity",
        Some("custody_approval_required" | "custody_user_presence_unavailable") => {
            "network_header.migration_approval"
        }
        Some("migration_in_progress" | "previous_window_active") => {
            "network_header.migration_active"
        }
        Some("channel_binding_required") => "network_header.session_required",
        Some(
            "home_genesis_required"
            | "home_checkpoint_required"
            | "backlog_incomplete"
            | "backlog_evidence_required"
            | "evidence_required",
        ) => "network_header.migration_evidence",
        _ => "network_header.migration",
    }
}
