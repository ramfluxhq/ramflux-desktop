// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The PRX runtime on this computer, managed in process (as `rf prx` does).
//!
//! The provider key of an install is handed to the SDK host controller only
//! through its key callback, which answers for the provider the owner chose
//! and for nothing else. It is never logged, never stored, never put in a
//! body or an event, and its copy is wiped when the call ends. Refusals of
//! the host come back as the host's typed codes; nothing else from the host
//! (paths, command output) is kept.

use std::path::{Path, PathBuf};

use ramflux_desktop_core::{AgentRuntimeAction, SecretInput};
use ramflux_sdk::SdkError;
use ramflux_sdk::prx_host::{self, Action, Request};
use serde_json::Value;
use zeroize::Zeroizing;

use crate::bus::BusError;

/// Runs one runtime action and answers the host status as JSON (no secret).
///
/// # Errors
///
/// [`BusError::Daemon`] carrying the host's typed refusal code.
pub async fn run(
    action: AgentRuntimeAction,
    provider: &str,
    model: &str,
    key: Option<SecretInput>,
) -> Result<Value, BusError> {
    let secret = key.map(SecretInput::into_zeroizing);
    let request = Request {
        action: host_action(action),
        provider: (!provider.is_empty()).then(|| provider.to_owned()),
        model: (!model.is_empty()).then(|| model.to_owned()),
        native_tools: false,
        no_native_tools: false,
        target_version: None,
        binary: PathBuf::from("prx"),
        state_dir: None,
        // The owner's confirmation was taken by the core for install, stop
        // and uninstall; the desktop's own Restart button is the confirmation
        // for a restart. Status needs none.
        confirmed: action != AgentRuntimeAction::Status,
    };
    let status = prx_host::manage(request, key_callback(secret, provider.to_owned()))
        .await
        .map_err(bus_error)?;
    serde_json::to_value(&status).map_err(|_| BusError::Daemon {
        retry_after_ms: None,
        code: "prx_status".to_owned(),
        message: "the PRX status could not be read".to_owned(),
        reason_code: None,
        details: None,
        cause: None,
    })
}

/// The state directory of the runtime a status names (its `runtime.json`
/// lives there). The owned daemon restarts with it as `--prx-state-dir`
/// after an install or a restart, so it adopts the runtime.
pub fn state_dir_of(status: &Value) -> Option<PathBuf> {
    let runtime = status["runtime_path"].as_str().filter(|path| !path.is_empty())?;
    Path::new(runtime).parent().map(Path::to_path_buf)
}

fn host_action(action: AgentRuntimeAction) -> Action {
    match action {
        AgentRuntimeAction::Install => Action::Install,
        AgentRuntimeAction::Restart => Action::Restart,
        AgentRuntimeAction::Stop => Action::Stop,
        AgentRuntimeAction::Uninstall => Action::Uninstall,
        AgentRuntimeAction::Status => Action::Status,
    }
}

/// The key callback: the key of `provider`, and nothing for any other name.
fn key_callback(
    secret: Option<Zeroizing<String>>,
    provider: String,
) -> impl FnMut(&str) -> Result<Zeroizing<String>, SdkError> {
    move |name: &str| match secret.as_ref() {
        Some(key) if name == provider => Ok(Zeroizing::new(key.as_str().to_owned())),
        _ => Err(prx_host::refused("credential_unavailable", "the provider key is not held")),
    }
}

/// Maps the host's refusal to the bus error shape the translator reads. The
/// message is the host's own short text; the code is the typed token.
fn bus_error(error: SdkError) -> BusError {
    let (code, message) = match error {
        SdkError::LocalBusRefused { code, message, .. } => (code, message),
        _ => ("prx_failed".to_owned(), "the PRX host failed".to_owned()),
    };
    BusError::Daemon {
        retry_after_ms: None,
        code: code.clone(),
        message,
        reason_code: Some(code),
        details: None,
        cause: None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_key_is_answered_for_its_provider_only() {
        let mut callback = key_callback(Some(Zeroizing::new("k-test".to_owned())), "p".into());
        assert_eq!(callback("p").expect("the provider's key").as_str(), "k-test");
        let refused = callback("other").expect_err("another provider gets nothing");
        assert!(matches!(
            refused,
            SdkError::LocalBusRefused { ref code, .. } if code == "credential_unavailable"
        ));
        let mut none = key_callback(None, "p".into());
        assert!(none("p").is_err(), "no key held means no key");
    }

    #[test]
    fn the_state_dir_is_the_folder_of_the_runtime_file() {
        let status = json!({"runtime_path": "/state/prx/runtime.json"});
        assert_eq!(state_dir_of(&status), Some(PathBuf::from("/state/prx")));
        assert_eq!(state_dir_of(&json!({"runtime_path": ""})), None);
        assert_eq!(state_dir_of(&json!({})), None);
    }

    #[test]
    fn a_host_refusal_keeps_its_typed_code() {
        let error = bus_error(prx_host::refused("prx_missing", "runtime.json"));
        assert!(matches!(
            error,
            BusError::Daemon { ref code, reason_code: Some(ref reason), .. }
                if code == "prx_missing" && reason == "prx_missing"
        ));
    }
}
