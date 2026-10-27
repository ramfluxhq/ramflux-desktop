// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Policy for creating an account with a recovery phrase (born-v3).
//!
//! Every step is human only, and the steps follow the ceremony the store
//! reports: begin only when the daemon offers it and no ceremony runs, show
//! only after begin, hide (ask for the words) only while the phrase is out,
//! confirm only when the words were asked, cancel while it runs. The
//! daemon enforces the same order and the five minutes; the policy keeps
//! the desktop from sending a step it knows is refused, and from spending
//! the single confirmation attempt on three empty words.

use serde_json::json;

use super::{Decision, IntentError, Policy};
use crate::intent::{IntentKind, methods};
use crate::model::{CHALLENGE_WORDS, CeremonyPhase};

/// Decides ceremony kinds; `None` for kinds of other areas.
pub(super) fn decide(p: &Policy<'_>, kind: &IntentKind) -> Option<Result<Decision, IntentError>> {
    Some(match kind {
        IntentKind::BeginCeremony { label } => {
            p.human_only("create an account with a recovery phrase").and_then(|()| {
                if !p.store.offers_born_v3_creation() {
                    return Err(IntentError::CeremonyNotOffered);
                }
                if label.trim().is_empty() {
                    return Err(IntentError::Empty("account name"));
                }
                if p.store.ceremony().is_some_and(|ceremony| ceremony.is_active()) {
                    return Err(IntentError::CeremonyPhase);
                }
                Ok(Decision::Call(
                    methods::CUSTODY_BEGIN,
                    json!({ "display_name": label.trim(), "principal_kind": "human" }),
                ))
            })
        }
        IntentKind::ShowRecoveryPhrase => {
            at(p, &CeremonyPhase::Prepared, Decision::Call(methods::CUSTODY_SHOW_ONCE, json!({})))
        }
        IntentKind::HideRecoveryPhrase => at(
            p,
            &CeremonyPhase::PhraseReady,
            Decision::Call(methods::CUSTODY_HIDDEN_CHALLENGE, json!({})),
        ),
        IntentKind::ConfirmCeremony { words, passphrase } => {
            p.human_only("create an account with a recovery phrase").and_then(|()| {
                let asked = matches!(
                    p.store.ceremony().map(|c| &c.phase),
                    Some(CeremonyPhase::Challenged { .. })
                );
                if !asked {
                    return Err(IntentError::CeremonyPhase);
                }
                if words.len() != CHALLENGE_WORDS || words.iter().any(|word| word.is_empty()) {
                    return Err(IntentError::CeremonyWords);
                }
                if passphrase.is_empty() {
                    return Err(IntentError::Empty("account passphrase"));
                }
                Ok(Decision::SecretCall(methods::CUSTODY_CONFIRM, json!({})))
            })
        }
        IntentKind::CancelCeremony => {
            p.human_only("create an account with a recovery phrase").and_then(|()| {
                if !p.store.ceremony().is_some_and(|ceremony| ceremony.is_active()) {
                    return Err(IntentError::CeremonyPhase);
                }
                Ok(Decision::Call(methods::CUSTODY_CANCEL, json!({})))
            })
        }
        _ => return None,
    })
}

/// A step valid only while the ceremony is in `phase`.
fn at(p: &Policy<'_>, phase: &CeremonyPhase, decision: Decision) -> Result<Decision, IntentError> {
    p.human_only("create an account with a recovery phrase")?;
    match p.store.ceremony() {
        Some(ceremony) if &ceremony.phase == phase => Ok(decision),
        _ => Err(IntentError::CeremonyPhase),
    }
}
