// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The username centre this computer uses: the host-provisioned directory
//! authority that signs username answers.
//!
//! The pin ships with the app as a bundled configuration file beside the
//! desktop executable (`username-center.json`, the same four fields the
//! `RAMFLUX_USERNAME_CENTER_*` variables carry). Those variables override
//! it when all of them are set, which is how a development build points at
//! another centre. Without either, usernames are not set up here, and the
//! app says so; messaging is unaffected.
//!
//! The pin is only ever read from these two sources. Nothing a directory
//! answers can establish or replace it.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use ramflux_protocol::username_directory::NativeDirectoryPin;
use ramflux_sdk::username_directory::{configured_center, validate_center};

/// The bundled configuration file's name, beside the desktop executable.
pub const BUNDLED_FILE: &str = "username-center.json";

/// The largest bundled file read; a pin is a few hundred bytes.
const MAX_FILE_LEN: u64 = 4096;

/// The centre this computer uses now: the environment override when it is
/// complete and valid, else the bundled file when it is present and valid.
pub fn current() -> Option<NativeDirectoryPin> {
    if let Ok(pin) = configured_center() {
        return Some(pin);
    }
    bundled().and_then(|path| load(&path))
}

/// The bundled configuration file, if the app ships one.
pub fn bundled() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let candidate = exe.parent()?.join(BUNDLED_FILE);
    candidate.is_file().then_some(candidate)
}

/// Reads and validates a pin file (a small JSON object with `url`, `id`,
/// `public_key` and `generation`). A file that is missing, too large,
/// malformed or refused by the SDK's own pin checks yields `None`.
pub fn load(path: &Path) -> Option<NativeDirectoryPin> {
    let file = std::fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(MAX_FILE_LEN + 1).read_to_string(&mut text).ok()?;
    if text.len() as u64 > MAX_FILE_LEN {
        return None;
    }
    let pin: NativeDirectoryPin = serde_json::from_str(&text).ok()?;
    validate_center(&pin).ok()?;
    Some(pin)
}
