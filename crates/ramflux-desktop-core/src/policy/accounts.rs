// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for the entry page and settings: accounts, devices, nodes and
//! diagnostics. Identity-level decisions are human only.

use serde_json::json;

use super::{Decision, IntentError, Policy};
use crate::ids::{AccountId, NodeId};
use crate::intent::{AppLockPhase, IntentKind, SecretInput, SyncDirection, methods};
use crate::model::{Node, PinSource, Risk};

/// The longest app lock timeout the daemon accepts: one day, in milliseconds.
const APP_LOCK_MAX_TIMEOUT_MS: u64 = 86_400_000;

/// Decides account-area kinds; `None` for kinds of other areas.
pub(super) fn decide(p: &Policy<'_>, kind: &IntentKind) -> Option<Result<Decision, IntentError>> {
    Some(match kind {
        IntentKind::Unlock { account_id, secret } => {
            p.human_only("unlock an account").and_then(|()| {
                account(p, account_id)?;
                secret_present(secret, "passphrase")?;
                Ok(Decision::SecretCall(
                    methods::ACCOUNT_UNLOCK,
                    json!({ "account_id": account_id }),
                ))
            })
        }
        IntentKind::AddNode { url } => non_empty(url, "node address")
            .map(|()| Decision::Call(methods::NODE_ADD, json!({ "url": url.trim() }))),
        IntentKind::ConfirmNode { node_id, pin_source, operator_key } => {
            p.human_only("confirm a node").and_then(|()| {
                let node = node(p, node_id)?;
                if node.confirmed {
                    return Err(IntentError::NodeAlreadyConfirmed(node_id.clone()));
                }
                // The operator's key must be the discovered one, character
                // for character: that is what makes the pin an operator pin.
                let operator_key = operator_key.as_deref().map(str::trim);
                if *pin_source == PinSource::Operator {
                    let matches = operator_key
                        .is_some_and(|key| !key.is_empty() && node.public_key.as_deref() == Some(key));
                    if !matches {
                        return Err(IntentError::OperatorKeyMismatch(node_id.clone()));
                    }
                } else if node.pin_source != *pin_source || node.key_changed {
                    // A key that changed has no first-use way on.
                    return Err(IntentError::PinMismatch(node_id.clone()));
                }
                if node.is_plain_http() && *pin_source != PinSource::Operator {
                    return Err(IntentError::HttpNeedsOperatorKey(node_id.clone()));
                }
                let risk = node.pin_risk(*pin_source);
                Ok(p.gated(
                    risk,
                    methods::NODE_CONFIRM,
                    json!({
                        "node_id": node_id,
                        "pin_source": pin_source,
                        "fingerprint": node.fingerprint,
                        "public_key": node.public_key,
                        "operator_key": operator_key,
                    }),
                ))
            })
        }
        IntentKind::LockAccount { account_id } => account(p, account_id)
            .map(|()| Decision::Call(methods::ACCOUNT_LOCK, json!({ "account_id": account_id }))),
        IntentKind::RotatePassphrase { account_id, current, next } => {
            p.human_only("rotate a passphrase").and_then(|()| {
                account(p, account_id)?;
                secret_present(current, "current passphrase")?;
                secret_present(next, "new passphrase")?;
                Ok(Decision::SecretCall(
                    methods::ACCOUNT_ROTATE_PASSPHRASE,
                    json!({ "account_id": account_id }),
                ))
            })
        }
        // The directory answers for the operation asked about (`null`: the
        // current state), echoing its id so the answer settles that claim.
        IntentKind::UsernameStatusRefresh { operation_id } => {
            p.human_only("read the username").map(|()| {
                Decision::Call(methods::USERNAME_STATUS, json!({ "operation_id": operation_id }))
            })
        }
        IntentKind::ClaimUsername { account_id, name, secret } => {
            p.human_only("claim a username").and_then(|()| {
                account(p, account_id)?;
                let name = name.trim().trim_start_matches('@').trim();
                non_empty(name, "username")?;
                secret_present(secret, "recovery phrase")?;
                Ok(Decision::SecretCall(
                    methods::USERNAME_CLAIM,
                    json!({ "account_id": account_id, "name": name }),
                ))
            })
        }
        IntentKind::RevokeDevice { device_id } => p.human_only("revoke a device").and_then(|()| {
            let device = p
                .store
                .device(device_id)
                .ok_or_else(|| IntentError::UnknownDevice(device_id.clone()))?;
            if device.revoked {
                return Err(IntentError::DeviceRevoked(device_id.clone()));
            }
            if device.this_device {
                return Err(IntentError::ThisDevice(device_id.clone()));
            }
            Ok(p.gated(Risk::High, methods::DEVICE_REVOKE, json!({ "device_id": device_id })))
        }),
        IntentKind::SyncDevices { direction, path } => {
            p.human_only("sync devices").and_then(|()| {
                non_empty(path, "sync path")?;
                let method = match direction {
                    SyncDirection::Export => methods::DEVICE_SYNC_EXPORT,
                };
                Ok(Decision::Call(method, json!({ "path": path })))
            })
        }
        IntentKind::ProbeNode { node_id } => node(p, node_id)
            .map(|_| Decision::Call(methods::NODE_PROBE, json!({ "node_id": node_id }))),
        IntentKind::RemoveNode { node_id } => node(p, node_id).and_then(|_| {
            if bound(p, node_id) {
                return Err(IntentError::NodeBound(node_id.clone()));
            }
            Ok(Decision::Call(methods::NODE_REMOVE, json!({ "node_id": node_id })))
        }),
        IntentKind::SetDefaultNode { node_id } => node(p, node_id).and_then(|node| {
            if !node.enabled {
                return Err(IntentError::DefaultDisabled(node_id.clone()));
            }
            if !node.confirmed {
                return Err(IntentError::NodeNotConfirmed(node_id.clone()));
            }
            Ok(Decision::Call(methods::NODE_SET_DEFAULT, json!({ "node_id": node_id })))
        }),
        IntentKind::SetNodeEnabled { node_id, enabled } => node(p, node_id).and_then(|node| {
            if !enabled && node.is_default {
                return Err(IntentError::DefaultDisabled(node_id.clone()));
            }
            Ok(Decision::Call(
                methods::NODE_SET_ENABLED,
                json!({ "node_id": node_id, "enabled": enabled }),
            ))
        }),
        IntentKind::ExportDiagnostics { path } => non_empty(path, "diagnostics path")
            .map(|()| Decision::Call(methods::DIAGNOSTICS_EXPORT, json!({ "path": path }))),
        IntentKind::AnswerAppLockCode { account_id, .. } => p.human_only("answer the account code").and_then(|()| {
            account(p, account_id)?;
            Ok(Decision::SecretCall(methods::APP_LOCK_CODE_ANSWER, json!({ "account_id": account_id })))
        }),
        IntentKind::ConfigureAppLock { account_id, enabled, timeout_ms } => {
            p.human_only("change the app lock").and_then(|()| {
                account(p, account_id)?;
                if *timeout_ms > APP_LOCK_MAX_TIMEOUT_MS {
                    return Err(IntentError::AppLockTimeout(*timeout_ms));
                }
                Ok(Decision::Call(
                    methods::APP_LOCK_CONFIGURE,
                    json!({ "account_id": account_id, "enabled": enabled, "timeout_ms": timeout_ms }),
                ))
            })
        }
        IntentKind::AppLockLifecycle { account_id, phase } => {
            account(p, account_id).map(|()| {
                let state = match phase {
                    AppLockPhase::Background => "background",
                    AppLockPhase::Foreground => "foreground",
                };
                Decision::Call(
                    methods::APP_LOCK_LIFECYCLE,
                    json!({ "account_id": account_id, "state": state }),
                )
            })
        }
        _ => return None,
    })
}

fn non_empty(value: &str, what: &'static str) -> Result<(), IntentError> {
    if value.trim().is_empty() { Err(IntentError::Empty(what)) } else { Ok(()) }
}

fn secret_present(secret: &SecretInput, what: &'static str) -> Result<(), IntentError> {
    if secret.is_empty() { Err(IntentError::Empty(what)) } else { Ok(()) }
}

fn account(p: &Policy<'_>, id: &AccountId) -> Result<(), IntentError> {
    p.store.account(id).map(|_| ()).ok_or_else(|| IntentError::UnknownAccount(id.clone()))
}

fn node<'a>(p: &Policy<'a>, id: &NodeId) -> Result<&'a Node, IntentError> {
    p.store.node(id).ok_or_else(|| IntentError::UnknownNode(id.clone()))
}

fn bound(p: &Policy<'_>, id: &NodeId) -> bool {
    p.store.accounts().any(|account| account.node_id.as_ref() == Some(id))
}
