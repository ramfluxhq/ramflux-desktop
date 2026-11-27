// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The attended account code of an app lock change.
//!
//! Turning the app lock off, or choosing a longer timeout, makes the daemon
//! ask for the account code on the same request connection. The code comes
//! from the window through [`CodeRequest`]: it is never a frame field, and it
//! is zeroized on every path. Only a configure this client is running (an
//! armed [`ArmGuard`]) can be answered; any other prompt is refused.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ramflux_desktop_core::SecretInput;
use ramflux_protocol::custody::pairing::local_dto::CredentialRequest;
use ramflux_sdk::SdkError;
use ramflux_sdk::custody::pairing::{CredentialOutcome, LocalCredentialProvider, SecretUtf8};
use tokio::sync::mpsc;

/// How long the window may take to answer the code sheet. Past it the
/// prompt is refused and the daemon answers the configure with a refusal.
pub(crate) const CODE_WAIT: Duration = Duration::from_secs(60);

/// One code the daemon asked for, to the window. The answer goes to `reply`:
/// an empty secret, or no answer at all, refuses the prompt.
pub(crate) struct CodeRequest {
    pub(crate) reply: std::sync::mpsc::SyncSender<SecretInput>,
}

/// The attended provider of one scope's request connection.
pub(crate) struct CodeGate {
    armed: AtomicUsize,
    requests: mpsc::UnboundedSender<CodeRequest>,
}

/// Keeps the gate armed while one configure runs.
pub(crate) struct ArmGuard(Arc<CodeGate>);

impl CodeGate {
    /// A gate that forwards its prompts to `requests` (the window's inbox).
    pub(crate) fn new(requests: mpsc::UnboundedSender<CodeRequest>) -> Arc<Self> {
        Arc::new(Self { armed: AtomicUsize::new(0), requests })
    }

    /// Arms the gate until the returned guard is dropped.
    pub(crate) fn arm(self: &Arc<Self>) -> ArmGuard {
        self.armed.fetch_add(1, Ordering::SeqCst);
        ArmGuard(Arc::clone(self))
    }
}

impl Drop for ArmGuard {
    fn drop(&mut self) {
        self.0.armed.fetch_sub(1, Ordering::SeqCst);
    }
}

impl LocalCredentialProvider for CodeGate {
    fn provide(&self, request: &CredentialRequest) -> Result<CredentialOutcome, SdkError> {
        // Only the code of a weakening, asked while this client is weakening.
        if request.action != "unlock" || self.armed.load(Ordering::SeqCst) == 0 {
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
