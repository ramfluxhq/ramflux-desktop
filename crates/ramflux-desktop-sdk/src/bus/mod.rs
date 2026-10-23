// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Local-bus requests and their failures, independent of any connection.
//!
//! - [`lane`]: a small pool of request connections for one scope.
//! - [`subscription`]: the one subscription connection of an account.

pub mod lane;
pub mod subscription;

use ramflux_desktop_core::SecretInput;
use ramflux_sdk::SdkError;
use serde_json::Value;
use zeroize::{Zeroize, Zeroizing};

/// One request to the daemon.
///
/// Secret fields stay out of `body` until the moment the frame is written:
/// the worker splices them in, sends, and wipes the copies it made.
#[derive(Debug)]
pub struct BusRequest {
    /// Account the request runs for; `None` for daemon-level methods.
    pub account: Option<String>,
    /// The `sdk_api` routing label of the frame.
    pub sdk_api: &'static str,
    /// Daemon method.
    pub method: String,
    /// Non-secret body.
    pub body: Value,
    /// Secret fields added to the body only when sending.
    pub secrets: Vec<(&'static str, SecretField)>,
}

/// A secret body field.
pub enum SecretField {
    /// A passphrase or other secret string.
    Text(SecretInput),
    /// A 32-byte seed, sent as an array of numbers.
    Seed(Zeroizing<[u8; 32]>),
}

impl std::fmt::Debug for SecretField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

impl BusRequest {
    /// A request without secrets.
    pub fn new(
        account: Option<&str>,
        sdk_api: &'static str,
        method: impl Into<String>,
        body: Value,
    ) -> Self {
        Self {
            account: account.map(str::to_owned),
            sdk_api,
            method: method.into(),
            body,
            secrets: Vec::new(),
        }
    }

    /// Adds a secret string field.
    #[must_use]
    pub fn with_secret(mut self, field: &'static str, secret: SecretInput) -> Self {
        self.secrets.push((field, SecretField::Text(secret)));
        self
    }

    /// Adds a secret seed field.
    #[must_use]
    pub fn with_seed(mut self, field: &'static str, seed: Zeroizing<[u8; 32]>) -> Self {
        self.secrets.push((field, SecretField::Seed(seed)));
        self
    }

    /// The body with the secrets spliced in. The caller must pass the result
    /// to [`wipe`] after use.
    pub fn wire_body(&self) -> Value {
        let mut body = self.body.clone();
        if let Value::Object(map) = &mut body {
            for (field, secret) in &self.secrets {
                let value = match secret {
                    SecretField::Text(text) => Value::String(text.expose().to_owned()),
                    SecretField::Seed(seed) => {
                        Value::Array(seed.iter().map(|b| Value::from(*b)).collect())
                    }
                };
                map.insert((*field).to_owned(), value);
            }
        }
        body
    }
}

/// Overwrites every string and number in a body that carried secrets, then
/// drops it.
pub fn wipe(mut body: Value) {
    fn walk(value: &mut Value) {
        match value {
            Value::String(text) => text.zeroize(),
            Value::Number(_) => *value = Value::from(0),
            Value::Array(items) => items.iter_mut().for_each(walk),
            Value::Object(map) => map.values_mut().for_each(walk),
            _ => {}
        }
    }
    walk(&mut body);
}

/// Why a request failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusError {
    /// The daemon answered with an error frame.
    Daemon {
        /// Daemon error code.
        code: String,
        /// Daemon message.
        message: String,
    },
    /// No connection, or it broke.
    Transport(String),
    /// No answer in time.
    Timeout,
}

impl BusError {
    /// Whether the connection to the daemon is gone (not a per-request refusal).
    pub fn is_connection_lost(&self) -> bool {
        matches!(self, Self::Transport(_))
    }

    /// Short text for the user.
    pub fn message(&self) -> String {
        match self {
            Self::Daemon { message, .. } => message.clone(),
            Self::Transport(message) => message.clone(),
            Self::Timeout => "the daemon did not answer in time".to_owned(),
        }
    }
}

/// Error codes the daemon puts on error frames.
const DAEMON_CODES: [&str; 5] = [
    "ValidationFailed",
    "CapabilityDenied",
    "GrantInvalidated",
    "SignatureVerificationFailed",
    "RemoteAppApprovalRequired",
];

impl From<SdkError> for BusError {
    fn from(error: SdkError) -> Self {
        match error {
            // An error frame reaches the client as `LocalBus("<code>: <message>")`.
            SdkError::LocalBus(text) => match text.split_once(": ") {
                Some((code, message)) if DAEMON_CODES.contains(&code) => {
                    Self::Daemon { code: code.to_owned(), message: message.to_owned() }
                }
                _ => Self::Transport(text),
            },
            other => Self::Transport(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn error_frames_become_daemon_errors_and_the_rest_transport() {
        let daemon = BusError::from(SdkError::LocalBus(
            "ValidationFailed: local bus error: account not open: a".to_owned(),
        ));
        assert_eq!(
            daemon,
            BusError::Daemon {
                code: "ValidationFailed".to_owned(),
                message: "local bus error: account not open: a".to_owned(),
            }
        );
        assert!(!daemon.is_connection_lost());
        let broken = BusError::from(SdkError::Io(std::io::Error::other("broken pipe")));
        assert!(broken.is_connection_lost());
        let odd = BusError::from(SdkError::LocalBus("unexpected local bus frame".to_owned()));
        assert!(odd.is_connection_lost());
    }

    #[test]
    fn secrets_are_spliced_only_into_the_wire_body() {
        let request = BusRequest::new(Some("a"), "account", "account.unlock", json!({}))
            .with_secret("passphrase", SecretInput::new("river"));
        assert_eq!(request.body, json!({}));
        assert!(!format!("{request:?}").contains("river"));
        let wire = request.wire_body();
        assert_eq!(wire["passphrase"], "river");
        wipe(wire);
    }
}
