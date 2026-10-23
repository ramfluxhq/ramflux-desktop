// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for the entry page and settings: accounts, devices, nodes and
//! diagnostics. Identity-level decisions are human only.

use serde_json::json;

use super::{Decision, IntentError, Policy};
use crate::ids::{AccountId, NodeId};
use crate::intent::{IntentKind, SecretInput, SyncDirection, methods};
use crate::model::{Node, Risk};

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
        IntentKind::CreateAccount { kind, label, node_id, secret } => {
            p.human_only("create an account").and_then(|()| {
                usable_node(p, node_id)?;
                non_empty(label, "account label")?;
                secret_present(secret, "passphrase")?;
                Ok(Decision::SecretCall(
                    methods::ACCOUNT_CREATE,
                    json!({ "kind": kind, "label": label, "node_id": node_id }),
                ))
            })
        }
        IntentKind::ActivateDevice { node_id, device_label, code } => {
            p.human_only("activate a device").and_then(|()| {
                usable_node(p, node_id)?;
                non_empty(device_label, "device label")?;
                secret_present(code, "activation code")?;
                Ok(Decision::SecretCall(
                    methods::DEVICE_ACTIVATE,
                    json!({ "node_id": node_id, "device_label": device_label }),
                ))
            })
        }
        IntentKind::RestoreBackup { path, secret } => {
            p.human_only("restore a backup").and_then(|()| {
                non_empty(path, "backup path")?;
                secret_present(secret, "passphrase")?;
                Ok(Decision::SecretCall(methods::ACCOUNT_BACKUP_IMPORT, json!({ "path": path })))
            })
        }
        IntentKind::AddNode { url } => non_empty(url, "node address")
            .map(|()| Decision::Call(methods::NODE_ADD, json!({ "url": url.trim() }))),
        IntentKind::ConfirmNode { node_id, pin_source } => {
            p.human_only("confirm a node").and_then(|()| {
                let node = node(p, node_id)?;
                if node.confirmed {
                    return Err(IntentError::NodeAlreadyConfirmed(node_id.clone()));
                }
                if node.pin_source != *pin_source {
                    return Err(IntentError::PinMismatch(node_id.clone()));
                }
                // Trusting a key on first use is a security decision: hold it.
                let risk =
                    if pin_source.is_trust_on_first_use() { Risk::High } else { Risk::Medium };
                Ok(p.gated(
                    risk,
                    methods::NODE_CONFIRM,
                    json!({
                        "node_id": node_id,
                        "pin_source": pin_source,
                        "fingerprint": node.fingerprint,
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
        IntentKind::ExportBackup { account_id, path } => {
            p.human_only("export a backup").and_then(|()| {
                account(p, account_id)?;
                non_empty(path, "backup path")?;
                Ok(Decision::Call(
                    methods::ACCOUNT_BACKUP_EXPORT,
                    json!({ "account_id": account_id, "path": path }),
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
                    SyncDirection::Import => methods::DEVICE_SYNC_IMPORT,
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

/// A node new accounts may bind to: confirmed (pinned).
fn usable_node(p: &Policy<'_>, id: &NodeId) -> Result<(), IntentError> {
    if node(p, id)?.confirmed { Ok(()) } else { Err(IntentError::NodeNotConfirmed(id.clone())) }
}

fn bound(p: &Policy<'_>, id: &NodeId) -> bool {
    p.store.accounts().any(|account| account.node_id.as_ref() == Some(id))
}
