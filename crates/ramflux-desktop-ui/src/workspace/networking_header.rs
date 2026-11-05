// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Networking meaning belongs to the workspace title, above transient facts.
use crate::i18n::{tr, tr_args};
use gpui_kit::SharedString;
use ramflux_desktop_core::{
    Account, EffectFailure, FailureCode, Networking, NetworkingReason,
    NetworkingReasonKind as Kind, RegistrationRefusal as Code,
};

pub(super) fn header(
    account: Option<&Account>,
    failure: Option<&EffectFailure>,
) -> (SharedString, Option<SharedString>) {
    let Some(account) = account else { return (tr("app.name"), None) };
    if let Some(failure) = failure {
        let key = match &failure.code {
            FailureCode::Daemon(code) if code == "CustodyForkFenced" => Some("network_header.fork"),
            FailureCode::Daemon(code) if code == "OperationReconciliationRequired" => {
                Some("backend.failure.reconcile")
            }
            FailureCode::Daemon(code)
                if code == "AuthorityUnavailable" || code == "CustodyAuthorityBlocked" =>
            {
                Some("network_header.authority")
            }
            _ => None,
        };
        if let Some(key) = key {
            return (tr(key), None);
        }
    }
    if let Some(reason) = &account.networking_reason {
        let detail = reason_text(reason);
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

fn reason_text(reason: &NetworkingReason) -> SharedString {
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
            Some(Code::HomeMigration { reason, nack }) => {
                return tr_args(
                    "network_header.migration",
                    &[
                        ("reason", &reason.to_string()),
                        (
                            "nack",
                            &nack.as_ref().map_or_else(|| "—".to_owned(), ToString::to_string),
                        ),
                    ],
                );
            }
        },
    };
    let mut text = tr(key).to_string();
    if let Some(ms) = reason.retry_after_ms.filter(|_| !reason.requires_action()) {
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
                ("scope", &capacity.scope),
                ("deadline", &capacity.resets_at_unix.to_string()),
            ],
        )
        .to_string();
    }
    text.into()
}
