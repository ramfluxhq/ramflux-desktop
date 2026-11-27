// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The attended account code of an app lock change.
//!
//! Turning the app lock off, or choosing a longer timeout, makes the daemon
//! ask for the account code on the same request connection. The code comes
//! from the window through [`CodeRequest`]: it is never a frame field, and it
//! is zeroized on every path. Only a weakening configure this client is
//! running (an [`ArmGuard`] armed with `prompt` for its account) can be
//! answered; any other prompt is refused.

use std::sync::Arc;

use parking_lot::Mutex;
use ramflux_desktop_core::{AccountId, SecretInput};
use ramflux_protocol::custody::pairing::local_dto::CredentialRequest;
use ramflux_sdk::SdkError;
use ramflux_sdk::custody::pairing::{CredentialOutcome, LocalCredentialProvider, SecretUtf8};
use tokio::sync::mpsc;

use std::time::Duration;

/// How long the window may take to answer the code sheet. Past it the
/// prompt is refused and the daemon answers the configure with a refusal.
pub(crate) const CODE_WAIT: Duration = Duration::from_secs(60);

/// One code the daemon asked for, to the window. The answer goes to `reply`:
/// an empty secret, or no answer at all, refuses the prompt.
pub(crate) struct CodeRequest {
    pub(crate) reply: std::sync::mpsc::SyncSender<SecretInput>,
}

/// A configure is already running on this client: the second one is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CodeBusy;

/// The configure that holds the gate.
struct Armed {
    /// The account the configure changes.
    account: AccountId,
    /// Whether the configure weakens the stored lock, so the daemon may ask
    /// for the account code. A strengthening one holds the gate (one
    /// configure at a time) but has every prompt refused.
    prompt: bool,
}

/// The attended provider of one scope's request connection.
pub(crate) struct CodeGate {
    /// The configure that is running (one at a time).
    armed: Mutex<Option<Armed>>,
    requests: mpsc::UnboundedSender<CodeRequest>,
}

/// Keeps the gate armed for one account while one configure runs.
pub(crate) struct ArmGuard(Arc<CodeGate>);

impl CodeGate {
    /// A gate that forwards its prompts to `requests` (the window's inbox).
    pub(crate) fn new(requests: mpsc::UnboundedSender<CodeRequest>) -> Arc<Self> {
        Arc::new(Self { armed: Mutex::new(None), requests })
    }

    /// Arms the gate for one configure of `account` until the returned guard
    /// is dropped; its code prompt is answered only when `prompt` (the
    /// configure weakens the lock). Refused with [`CodeBusy`] while another
    /// configure holds the gate.
    pub(crate) fn arm(
        self: &Arc<Self>,
        account: AccountId,
        prompt: bool,
    ) -> Result<ArmGuard, CodeBusy> {
        let mut slot = self.armed.lock();
        if slot.is_some() {
            return Err(CodeBusy);
        }
        *slot = Some(Armed { account, prompt });
        Ok(ArmGuard(Arc::clone(self)))
    }

    /// Whether a configure holds the gate. The slot is the only record of
    /// it: the guard clears it however the configure task ends.
    pub(crate) fn is_armed(&self) -> bool {
        self.armed.lock().is_some()
    }
}

impl Drop for ArmGuard {
    fn drop(&mut self) {
        *self.0.armed.lock() = None;
    }
}

impl LocalCredentialProvider for CodeGate {
    fn provide(&self, request: &CredentialRequest) -> Result<CredentialOutcome, SdkError> {
        // Only the code of a weakening, for the account this client is weakening.
        let for_armed_account = self.armed.lock().as_ref().is_some_and(|armed| {
            armed.prompt && armed.account.as_str() == request.local_account_id
        });
        if request.action != "unlock" || !for_armed_account {
            return Ok(CredentialOutcome::Refused);
        }
        let (reply, answer) = std::sync::mpsc::sync_channel(1);
        if self.requests.send(CodeRequest { reply }).is_err() {
            return Ok(CredentialOutcome::Refused);
        }
        match answer.recv_timeout(CODE_WAIT) {
            Ok(secret) if !secret.is_empty() => Ok(CredentialOutcome::Provided {
                passphrase: SecretUtf8::from_bytes(secret.expose().as_bytes())?,
                confirmation: None,
            }),
            _ => Ok(CredentialOutcome::Refused),
        }
    }
}
