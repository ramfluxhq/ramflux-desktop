// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The data source of a window: command-line flags, then the environment
//! (`RAMFLUX_DESKTOP_SOURCE`, `RAMFLUX_DESKTOP_SOCKET`), then the saved
//! launch settings, then the build's default (scenarios in debug builds,
//! the daemon at the reference CLI's socket in release builds). Whether the
//! source polls when the daemon offers no event stream comes from the saved
//! launch settings.
//!
//! A socket given on the command line or in the environment without a
//! source there means the daemon.

use ramflux_desktop_sdk::SdkConfig;
use ramflux_desktop_sdk::local::{LaunchSettings, SourceKind};

use crate::shell::SourceChoice;

/// Names the source in the environment.
pub const SOURCE_ENV: &str = "RAMFLUX_DESKTOP_SOURCE";

/// Names the daemon socket in the environment.
pub const SOCKET_ENV: &str = "RAMFLUX_DESKTOP_SOCKET";

/// Source and socket given on the command line or in the environment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Overrides {
    /// `mock` or `rfd`.
    pub source: Option<String>,
    /// A socket path.
    pub socket: Option<String>,
}

impl Overrides {
    /// Parses `[--source mock|rfd] [--socket PATH]`.
    ///
    /// # Errors
    ///
    /// An unknown argument.
    pub fn from_args(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut overrides = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--source" => overrides.source = args.next(),
                "--socket" => overrides.socket = args.next(),
                other => return Err(format!("unknown argument: {other}")),
            }
        }
        Ok(overrides)
    }

    /// Fills what `self` leaves open from `fallback`.
    #[must_use]
    pub fn or(self, fallback: Self) -> Self {
        Self { source: self.source.or(fallback.source), socket: self.socket.or(fallback.socket) }
    }
}

/// The source `overrides` and `settings` choose.
///
/// # Errors
///
/// An unknown source name.
pub fn resolve(overrides: &Overrides, settings: &LaunchSettings) -> Result<SourceChoice, String> {
    let kind = match overrides.source.as_deref() {
        Some(name) => SourceKind::parse(name)
            .ok_or_else(|| format!("unknown source: {name} (use mock or rfd)"))?,
        None if overrides.socket.is_some() => SourceKind::Rfd,
        None => settings.source_or_default(),
    };
    Ok(match kind {
        SourceKind::Mock => SourceChoice::Mock,
        SourceKind::Rfd => {
            let socket = overrides
                .socket
                .as_ref()
                .map_or_else(|| settings.socket_or_default(), std::path::PathBuf::from);
            SourceChoice::Rfd(SdkConfig {
                poll_fallback: settings.poll_fallback_or_default(),
                ..SdkConfig::new(socket)
            })
        }
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use ramflux_desktop_sdk::DEFAULT_SOCKET;

    use super::*;

    fn socket(choice: &SourceChoice) -> Option<&Path> {
        match choice {
            SourceChoice::Rfd(config) => Some(&config.socket_path),
            SourceChoice::Mock => None,
        }
    }

    fn args(list: &[&str]) -> Overrides {
        Overrides::from_args(list.iter().map(|s| (*s).to_owned())).expect("args")
    }

    #[test]
    fn flags_win_over_environment_and_settings() {
        let settings = LaunchSettings {
            source: Some(SourceKind::Mock),
            socket_path: Some(PathBuf::from("/saved.sock")),
            ..LaunchSettings::default()
        };
        let env = Overrides { source: Some("mock".into()), socket: Some("/env.sock".into()) };
        let flags = args(&["--source", "rfd", "--socket", "/flag.sock"]);
        let chosen = resolve(&flags.or(env.clone()), &settings).expect("choice");
        assert_eq!(socket(&chosen), Some(Path::new("/flag.sock")));
        let chosen = resolve(&args(&[]).or(env), &settings).expect("choice");
        assert!(matches!(chosen, SourceChoice::Mock), "the environment names mock");
    }

    #[test]
    fn saved_settings_apply_when_nothing_overrides_them() {
        let saved = LaunchSettings {
            source: Some(SourceKind::Rfd),
            socket_path: Some(PathBuf::from("/saved.sock")),
            ..LaunchSettings::default()
        };
        let chosen = resolve(&Overrides::default(), &saved).expect("choice");
        assert_eq!(socket(&chosen), Some(Path::new("/saved.sock")));
        let socket_only = Overrides { socket: Some("/env.sock".into()), source: None };
        let mock_saved = LaunchSettings { source: Some(SourceKind::Mock), ..saved };
        let chosen = resolve(&socket_only, &mock_saved).expect("choice");
        assert_eq!(socket(&chosen), Some(Path::new("/env.sock")), "a socket means the daemon");
    }

    #[test]
    fn the_build_default_uses_the_reference_socket() {
        let chosen = resolve(&Overrides::default(), &LaunchSettings::default()).expect("choice");
        if cfg!(debug_assertions) {
            assert!(matches!(chosen, SourceChoice::Mock));
        } else {
            assert_eq!(socket(&chosen), Some(Path::new(DEFAULT_SOCKET)));
        }
        let rfd = resolve(&args(&["--source", "rfd"]), &LaunchSettings::default()).expect("rfd");
        assert_eq!(socket(&rfd), Some(Path::new(DEFAULT_SOCKET)));
    }

    #[test]
    fn the_polling_fallback_comes_from_the_saved_settings() {
        let fallback = |saved: Option<bool>| {
            let settings = LaunchSettings { poll_fallback: saved, ..LaunchSettings::default() };
            match resolve(&args(&["--source", "rfd"]), &settings).expect("rfd") {
                SourceChoice::Rfd(config) => config.poll_fallback,
                SourceChoice::Mock => panic!("the daemon was asked for"),
            }
        };
        assert!(fallback(None), "on unless turned off");
        assert!(!fallback(Some(false)));
    }

    #[test]
    fn unknown_arguments_and_sources_are_errors() {
        assert!(Overrides::from_args(["--verbose".to_owned()]).is_err());
        let bad = Overrides { source: Some("daemon".into()), socket: None };
        assert!(resolve(&bad, &LaunchSettings::default()).is_err());
    }
}
