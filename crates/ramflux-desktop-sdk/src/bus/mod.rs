// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Local-bus requests and their failures, independent of any connection.
//!
//! - [`client`]: one multiplexed request connection for one scope.
//! - [`deadline`]: the deadline every request carries.
//! - [`idempotency`]: the retry key every mutating request carries.
//! - [`jobs`]: the daemon-level backup job stream, and the attended
//!   connection the job requests travel on.
//! - [`stream`]: the event stream of an account (`bus.subscribe`).
//! - [`subscription`]: the push subscription of an account on a daemon
//!   without the stream.

pub mod client;
pub mod deadline;
pub mod idempotency;
pub mod jobs;
pub mod stream;
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
    /// The retry key of a mutating request (the frame's `operation_id`);
    /// `None` for reads.
    pub operation_id: Option<String>,
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
            operation_id: None,
        }
    }

    /// Sets the retry key.
    #[must_use]
    pub fn with_operation_id(mut self, key: String) -> Self {
        self.operation_id = Some(key);
        self
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
        /// The typed reason of the refusal (`reason_code` on the error
        /// frame), when the daemon gave one.
        reason_code: Option<String>,
        /// Structured details of the refusal (`details` on the error
        /// frame), e.g. both roots of a root replacement to confirm.
        details: Option<Box<Value>>,
    },
    /// No connection, or it broke.
    Transport(String),
    /// No answer in time.
    Timeout,
}

impl BusError {
    /// A daemon refusal without a typed reason.
    pub fn daemon(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Daemon {
            code: code.into(),
            message: message.into(),
            reason_code: None,
            details: None,
        }
    }

    /// A daemon error frame's body as the refusal it is. A missed deadline
    /// is a timeout, whichever side noticed it.
    pub fn from_error_body(error: Option<ramflux_sdk::LocalBusErrorBody>) -> Self {
        let Some(error) = error else {
            return Self::daemon("ValidationFailed", "unknown local bus error");
        };
        if error.message.contains(DEADLINE_EXCEEDED) {
            return Self::Timeout;
        }
        Self::Daemon {
            code: error.code,
            message: error.message,
            reason_code: error.reason_code,
            details: error.details,
        }
    }

    /// The typed reason of a daemon refusal.
    pub fn reason_code(&self) -> Option<&str> {
        match self {
            Self::Daemon { reason_code, .. } => reason_code.as_deref(),
            _ => None,
        }
    }

    /// Whether the daemon refused the request as a method it does not know
    /// (its API lacks it): `LocalBus("unsupported local bus method: …")`,
    /// which reaches the bus untyped. Only this says a feature is missing;
    /// any other refusal is about this request.
    pub fn is_unknown_method(&self) -> bool {
        matches!(self, Self::Daemon { message, .. } if message.contains(UNKNOWN_METHOD))
    }

    /// Whether the connection to the daemon is gone (not a per-request refusal).
    pub fn is_connection_lost(&self) -> bool {
        matches!(self, Self::Transport(_))
    }

    /// Whether trying the same request again later may work: a transient
    /// outage, a lost link or a missed deadline. A refusal, an
    /// unauthenticated session and a failed trust check are not retried.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Daemon { code, .. } => code == "Unavailable",
            Self::Transport(_) | Self::Timeout => true,
        }
    }

    /// Short text for the user.
    pub fn message(&self) -> String {
        match self {
            Self::Daemon { message, .. } if message.contains(RECONCILE) => RECONCILE_KEY.to_owned(),
            Self::Daemon { code, .. } if code == "Unauthenticated" => {
                UNAUTHENTICATED_KEY.to_owned()
            }
            Self::Daemon { code, .. } if code == "TransportSecurity" => {
                TRANSPORT_SECURITY_KEY.to_owned()
            }
            Self::Daemon { message, .. } => message.clone(),
            Self::Transport(message) => message.clone(),
            Self::Timeout => TIMEOUT_KEY.to_owned(),
        }
    }
}

/// Locale key of the timeout message (translated by the UI).
pub const TIMEOUT_KEY: &str = "backend.failure.timeout";

/// Locale key of the message for a session the node refused as
/// unauthenticated (not retried).
pub const UNAUTHENTICATED_KEY: &str = "backend.failure.unauthenticated";

/// Locale key of the message for a failed TLS or pin check (not retried).
pub const TRANSPORT_SECURITY_KEY: &str = "backend.failure.transport_security";

/// Locale key of the message for an operation the daemon cannot replay.
pub const RECONCILE_KEY: &str = "backend.failure.reconcile";

/// How the daemon refuses a retry of an operation that was interrupted (or
/// failed) after it was reserved: it never guesses whether it applied.
const RECONCILE: &str = "operation requires reconciliation";

/// How the daemon describes a method it does not know (product
/// `bus/daemon.rs`, `SdkError::LocalBus`).
const UNKNOWN_METHOD: &str = "unsupported local bus method";

/// How the client and the daemon describe a missed deadline.
const DEADLINE_EXCEEDED: &str = "request deadline exceeded";

/// Error codes the daemon puts on error frames (besides the typed surface
/// refusal codes, [`daemon_code`]). The transport codes are the product's
/// `docs/local_bus_errors.md`: `Unavailable` is a transient outage (a
/// retry may work), `Unauthenticated` and `TransportSecurity` are not
/// retried (sign in again, or repair the trusted binding, first).
const DAEMON_CODES: [&str; 8] = [
    "ValidationFailed",
    "Unavailable",
    "Unauthenticated",
    "TransportSecurity",
    "CapabilityDenied",
    "GrantInvalidated",
    "SignatureVerificationFailed",
    "RemoteAppApprovalRequired",
];

/// The typed task refusal codes (`docs/conversation_tasks.md` of the
/// product) and the over-cap task delivery code.
const TASK_CODES: [&str; 18] = [
    "invalid_request",
    "invalid_event",
    "not_requester",
    "not_assignee",
    "not_member",
    "not_eligible_acceptor",
    "human_required",
    "stale_revision",
    "terminal",
    "task_stalled",
    "not_found",
    "already_exists",
    "no_local_role",
    "quota_exhausted",
    "rate_limited",
    "decision_consumed",
    "decision_mismatch",
    "TaskDeliveryTooLarge",
];

/// Whether `code` is a code the daemon puts on error frames: one of
/// [`DAEMON_CODES`], a typed task refusal ([`TASK_CODES`]) or a typed
/// surface refusal (`not_out_of_sync`, `too_large`, …, as the SDK defines
/// them).
fn daemon_code(code: &str) -> bool {
    DAEMON_CODES.contains(&code)
        || TASK_CODES.contains(&code)
        || serde_json::from_value::<ramflux_sdk::SurfaceRefusalCode>(serde_json::Value::from(code))
            .is_ok()
}

impl From<SdkError> for BusError {
    fn from(error: SdkError) -> Self {
        match error {
            // Both the client (no answer by the deadline) and the daemon (the
            // deadline passed before the request was applied) say so.
            SdkError::LocalBus(text) if text.contains(DEADLINE_EXCEEDED) => Self::Timeout,
            // An error frame reaches the client as `LocalBus("<code>: <message>")`.
            SdkError::LocalBus(text) => match text.split_once(": ") {
                Some((code, message)) if daemon_code(code) => Self::daemon(code, message),
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
    fn only_an_unknown_method_says_an_api_is_missing() {
        let unknown = BusError::daemon(
            "ValidationFailed",
            "local bus error: unsupported local bus method: contact.requests.list",
        );
        assert!(unknown.is_unknown_method());
        let locked = BusError::daemon("ValidationFailed", "local bus error: account not open: a");
        assert!(!locked.is_unknown_method());
        assert!(!BusError::daemon("Unavailable", "down").is_unknown_method());
        assert!(!BusError::Timeout.is_unknown_method());
    }

    #[test]
    fn error_frames_become_daemon_errors_and_the_rest_transport() {
        let daemon = BusError::from(SdkError::LocalBus(
            "ValidationFailed: local bus error: account not open: a".to_owned(),
        ));
        assert_eq!(
            daemon,
            BusError::daemon("ValidationFailed", "local bus error: account not open: a")
        );
        assert!(!daemon.is_connection_lost());
        let broken = BusError::from(SdkError::Io(std::io::Error::other("broken pipe")));
        assert!(broken.is_connection_lost());
        let odd = BusError::from(SdkError::LocalBus("unexpected local bus frame".to_owned()));
        assert!(odd.is_connection_lost());
        let closed = BusError::from(SdkError::LocalBus("local bus connection closed".to_owned()));
        assert!(closed.is_connection_lost());
        // A typed surface refusal is the daemon's answer, not a lost link.
        let refused = BusError::from(SdkError::LocalBus(
            "not_out_of_sync: surface is not out of sync".to_owned(),
        ));
        assert_eq!(refused, BusError::daemon("not_out_of_sync", "surface is not out of sync"));
        assert!(!refused.is_connection_lost());
    }

    #[test]
    fn error_frames_keep_their_typed_reason_and_details() {
        let body = |message: &str| ramflux_sdk::LocalBusErrorBody {
            details: Some(Box::new(json!({ "principal_id": "ana" }))),
            reason_code: Some("root_replacement_confirmation_required".to_owned()),
            code: "ValidationFailed".to_owned(),
            message: message.to_owned(),
            retry_after_ms: None,
            cause: None,
        };
        let refused = BusError::from_error_body(Some(body("contact root replacement")));
        assert_eq!(refused.reason_code(), Some("root_replacement_confirmation_required"));
        assert!(
            matches!(&refused, BusError::Daemon { details: Some(d), .. } if d["principal_id"] == "ana")
        );
        assert!(!refused.is_connection_lost());
        let late = BusError::from_error_body(Some(body("request deadline exceeded")));
        assert_eq!(late, BusError::Timeout);
        // A retryable lookup failure is the daemon's answer, not a lost link.
        let unavailable =
            BusError::from(SdkError::LocalBus("Unavailable: contact lookup unavailable".into()));
        assert!(matches!(&unavailable, BusError::Daemon { code, .. } if code == "Unavailable"));
        // The transport codes are answers too; only an outage is retried.
        let outage = BusError::from(SdkError::LocalBus("Unavailable: QUIC deadline".into()));
        assert!(!outage.is_connection_lost() && outage.is_retryable());
        for code in ["Unauthenticated", "TransportSecurity"] {
            let refused = BusError::from(SdkError::LocalBus(format!("{code}: tls")));
            assert!(matches!(&refused, BusError::Daemon { code: c, .. } if c == code));
            assert!(!refused.is_connection_lost() && !refused.is_retryable(), "{code}");
            assert!(refused.message().starts_with("backend.failure."), "{code}: a clear text");
        }
        // So is a typed task refusal, and an over-cap task delivery.
        for code in ["stale_revision", "quota_exhausted", "TaskDeliveryTooLarge"] {
            let refused = BusError::from(SdkError::LocalBus(format!("{code}: no")));
            assert_eq!(refused, BusError::daemon(code, "no"));
        }
    }

    #[test]
    fn every_sdk_task_refusal_code_is_a_daemon_code() {
        use ramflux_sdk::TaskRefusalCode as C;
        let all = [
            C::RateLimited,
            C::InvalidRequest,
            C::InvalidEvent,
            C::NotRequester,
            C::NotAssignee,
            C::NotMember,
            C::NotEligibleAcceptor,
            C::HumanRequired,
            C::StaleRevision,
            C::Terminal,
            C::TaskStalled,
            C::NotFound,
            C::AlreadyExists,
            C::NoLocalRole,
            C::SignerQuotaExhausted,
            C::TaskQuotaExhausted,
            C::DecisionConsumed,
            C::DecisionMismatch,
        ];
        for code in all {
            assert!(daemon_code(code.as_str()), "{}", code.as_str());
        }
    }

    #[test]
    fn missed_deadlines_on_either_side_are_timeouts() {
        let client = BusError::from(SdkError::LocalBus("request deadline exceeded".to_owned()));
        assert_eq!(client, BusError::Timeout);
        let daemon = BusError::from(SdkError::LocalBus(
            "ValidationFailed: local bus error: request deadline exceeded or outside supported \
             clock range"
                .to_owned(),
        ));
        assert_eq!(daemon, BusError::Timeout);
        assert!(!daemon.is_connection_lost());
        assert_eq!(daemon.message(), TIMEOUT_KEY);
    }

    #[test]
    fn a_retry_the_daemon_cannot_replay_asks_the_user_to_check() {
        let refused = BusError::from(SdkError::LocalBus(
            "ValidationFailed: storage error: event id conflict: operation requires \
             reconciliation: desk-op-1"
                .to_owned(),
        ));
        assert!(matches!(&refused, BusError::Daemon { code, .. } if code == "ValidationFailed"));
        assert_eq!(refused.message(), RECONCILE_KEY);
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
