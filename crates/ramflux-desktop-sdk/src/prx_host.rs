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
    // Resolved once: the host runs this exact file, not whatever `prx` the
    // search path names by the time it spawns.
    let binary = resolve_prx(std::env::var_os("PATH").as_deref()).map_err(resolve_error)?;
    let request = Request {
        action: host_action(action),
        provider: (!provider.is_empty()).then(|| provider.to_owned()),
        model: (!model.is_empty()).then(|| model.to_owned()),
        native_tools: false,
        no_native_tools: false,
        target_version: None,
        binary,
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

/// Why `prx` could not be resolved to a file this computer trusts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveError {
    /// No executable `prx` on the search path.
    Missing,
    /// A `prx` exists only where it is not trusted (see [`resolve_prx`]).
    Unsafe,
}

/// Resolves `prx` on `search_path` to an absolute, canonical file path.
///
/// Relative and empty search path entries are refused (they name the
/// current folder). A candidate is accepted only when its folder and the
/// file itself are owned by root or by the current user and are not
/// writable by group or others; an unsafe candidate is skipped, and the
/// search goes on. Without a match the answer is [`ResolveError::Unsafe`]
/// when something was skipped, else [`ResolveError::Missing`].
///
/// # Errors
///
/// [`ResolveError`] as above.
pub fn resolve_prx(search_path: Option<&std::ffi::OsStr>) -> Result<PathBuf, ResolveError> {
    let Some(search_path) = search_path.filter(|path| !path.is_empty()) else {
        return Err(ResolveError::Missing);
    };
    let mut skipped = false;
    let trusted = trusted_owners();
    for dir in std::env::split_paths(search_path) {
        if !dir.is_absolute() {
            skipped = true;
            continue;
        }
        let candidate = dir.join("prx");
        let Ok(real) = std::fs::canonicalize(&candidate) else { continue };
        if !real.is_file() {
            continue;
        }
        if trusted_file(&real, &trusted) {
            return Ok(real);
        }
        skipped = true;
    }
    Err(if skipped { ResolveError::Unsafe } else { ResolveError::Missing })
}

/// The `prx` the runtime panel would run, for the confirmation to show.
pub fn resolved_prx() -> Result<PathBuf, ResolveError> {
    resolve_prx(std::env::var_os("PATH").as_deref())
}

/// The owners trusted for the file and its folder: root and the user (the
/// owner of the home folder).
#[cfg(unix)]
fn trusted_owners() -> Vec<u32> {
    use std::os::unix::fs::MetadataExt as _;
    let mut owners = vec![0];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && home.is_absolute()
        && let Ok(meta) = std::fs::metadata(&home)
    {
        owners.push(meta.uid());
    }
    owners
}

#[cfg(not(unix))]
fn trusted_owners() -> Vec<u32> {
    Vec::new()
}

/// Whether the file and the folder it lives in are trusted.
#[cfg(unix)]
fn trusted_file(file: &Path, owners: &[u32]) -> bool {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let ok = |path: &Path, executable: bool| {
        std::fs::metadata(path).is_ok_and(|meta| {
            let mode = meta.permissions().mode();
            owners.contains(&meta.uid()) && mode & 0o022 == 0 && (!executable || mode & 0o111 != 0)
        })
    };
    file.parent().is_some_and(|dir| ok(dir, false)) && ok(file, true)
}

#[cfg(not(unix))]
fn trusted_file(_file: &Path, _owners: &[u32]) -> bool {
    true
}

fn resolve_error(error: ResolveError) -> BusError {
    let (code, message) = match error {
        ResolveError::Missing => ("prx_missing", "PRX is not installed on this computer"),
        ResolveError::Unsafe => ("prx_path_unsafe", "PRX was found only in an untrusted place"),
    };
    BusError::Daemon {
        retry_after_ms: None,
        code: code.to_owned(),
        message: message.to_owned(),
        reason_code: Some(code.to_owned()),
        details: None,
        cause: None,
    }
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

    #[cfg(unix)]
    mod resolve {
        use std::os::unix::fs::PermissionsExt as _;

        use super::*;

        fn prx_in(dir: &Path, mode: u32) {
            std::fs::create_dir_all(dir).expect("dir");
            let file = dir.join("prx");
            std::fs::write(&file, "#!/bin/sh\n").expect("file");
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).expect("mode");
        }

        fn dir_mode(dir: &Path, mode: u32) {
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode)).expect("mode");
        }

        fn scratch(tag: &str) -> PathBuf {
            let dir = std::env::temp_dir().join(format!("rfd-prx-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch");
            dir
        }

        #[test]
        fn a_prx_in_a_private_absolute_folder_resolves_to_its_real_path() {
            let root = scratch("ok");
            let bin = root.join("bin");
            prx_in(&bin, 0o755);
            dir_mode(&bin, 0o755);
            let path = std::env::join_paths([&bin]).expect("path");
            let found = resolve_prx(Some(&path)).expect("resolves");
            assert!(found.is_absolute());
            assert_eq!(found, std::fs::canonicalize(bin.join("prx")).expect("real"));
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn relative_empty_and_missing_entries_never_resolve() {
            let root = scratch("rel");
            let bin = root.join("bin");
            prx_in(&bin, 0o755);
            let relative = std::env::join_paths(["", ".", "bin"]).expect("path");
            // Relative entries are refused whatever the current folder holds.
            assert_eq!(resolve_prx(Some(&relative)), Err(ResolveError::Unsafe));
            assert_eq!(resolve_prx(None), Err(ResolveError::Missing));
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn a_folder_or_file_others_can_write_is_skipped() {
            let root = scratch("open");
            let loose = root.join("loose");
            prx_in(&loose, 0o755);
            dir_mode(&loose, 0o777);
            let path = std::env::join_paths([&loose]).expect("path");
            assert_eq!(resolve_prx(Some(&path)), Err(ResolveError::Unsafe));
            let tight = root.join("tight");
            prx_in(&tight, 0o777);
            dir_mode(&tight, 0o755);
            let path = std::env::join_paths([&tight]).expect("path");
            assert_eq!(resolve_prx(Some(&path)), Err(ResolveError::Unsafe));
            // The search goes on past an unsafe candidate to a safe one.
            let good = root.join("good");
            prx_in(&good, 0o755);
            dir_mode(&good, 0o755);
            let path = std::env::join_paths([&loose, &good]).expect("path");
            assert_eq!(
                resolve_prx(Some(&path)),
                Ok(std::fs::canonicalize(good.join("prx")).expect("real"))
            );
            dir_mode(&loose, 0o755);
            let _ = std::fs::remove_dir_all(root);
        }

        #[test]
        fn a_file_that_is_not_executable_is_not_a_prx() {
            let root = scratch("noexec");
            let bin = root.join("bin");
            prx_in(&bin, 0o644);
            dir_mode(&bin, 0o755);
            let path = std::env::join_paths([&bin]).expect("path");
            assert_eq!(resolve_prx(Some(&path)), Err(ResolveError::Unsafe));
            let _ = std::fs::remove_dir_all(root);
        }
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
