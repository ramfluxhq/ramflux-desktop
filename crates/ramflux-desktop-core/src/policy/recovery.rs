// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for recovering an account from its recovery phrase (born-v3).
//!
//! Human only, and only the user starts it: begin names an account that
//! lives on this device, confirm follows the recovery the store reports and
//! carries the fingerprint the daemon named, so a confirmation of another
//! fingerprint never leaves the desktop. The phrase must read as 24 list
//! words before the one attempt is spent. The words and the passphrase are
//! secrets: they travel beside the call, never in its body.

use serde_json::json;

use super::{Decision, IntentError, Policy};
use crate::intent::{IntentKind, methods};
use crate::model::{AccountKind, RecoveryPhase};
use crate::recovery::check_phrase;

const WHAT: &str = "recover an account with a recovery phrase";

/// Decides recovery kinds; `None` for kinds of other areas.
pub(super) fn decide(p: &Policy<'_>, kind: &IntentKind) -> Option<Result<Decision, IntentError>> {
    Some(match kind {
        IntentKind::BeginRecovery { account_id } => p.human_only(WHAT).and_then(|()| {
            if !p.store.offers_born_v3_creation() {
                return Err(IntentError::CeremonyNotOffered);
            }
            let account = p
                .store
                .account(account_id)
                .ok_or_else(|| IntentError::UnknownAccount(account_id.clone()))?;
            // An AI identity has no phrase of its own to recover from.
            if account.kind != AccountKind::HumanInteractive {
                return Err(IntentError::RecoveryNotForKind);
            }
            if p.store.recovery().is_some_and(|recovery| recovery.is_active()) {
                return Err(IntentError::RecoveryPhase);
            }
            Ok(Decision::Call(methods::RECOVER_BEGIN, json!({ "account_id": account_id })))
        }),
        IntentKind::ConfirmRecovery { anchor, phrase, passphrase } => {
            p.human_only(WHAT).and_then(|()| {
                let running = p
                    .store
                    .recovery()
                    .filter(|recovery| matches!(recovery.phase, RecoveryPhase::Prepared))
                    .ok_or(IntentError::RecoveryPhase)?;
                if running.anchor.is_empty() || running.anchor != *anchor {
                    return Err(IntentError::RecoveryAnchor);
                }
                if !check_phrase(phrase.expose()).complete() {
                    return Err(IntentError::RecoveryWords);
                }
                if passphrase.is_empty() {
                    return Err(IntentError::Empty("account passphrase"));
                }
                Ok(Decision::SecretCall(
                    methods::RECOVER_CONFIRM,
                    json!({ "account_id": running.account_id, "confirmed_anchor": anchor }),
                ))
            })
        }
        IntentKind::CancelRecovery => p.human_only(WHAT).and_then(|()| {
            if !p.store.recovery().is_some_and(|recovery| recovery.is_active()) {
                return Err(IntentError::RecoveryPhase);
            }
            Ok(Decision::Call(methods::RECOVER_CANCEL, json!({})))
        }),
        _ => return None,
    })
}
