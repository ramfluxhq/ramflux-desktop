// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Design instruments for the desktop client.
//!
//! - [`manifest`]: the checked-in list of design board references and the
//!   verification of a local reference set against it.
//! - [`matrix`]: the capture matrix (themes, locales, window sizes) and the
//!   capture file names shared by the capture program and the comparison.
//! - [`compare`]: pixel comparison of a capture with its reference, regions,
//!   a perceptual score and the three-up image.
//! - [`deviations`]: the per-board reviewed expected-deviation files.
//! - [`classify`]: attribution of deviation regions to error classes.
//! - [`stroke`]: the stroke-alignment rule (centred design strokes against
//!   the kit's inside borders).
//! - [`kit`]: kit components compared one by one with the component boards.
//! - [`report`]: the machine-readable result and the Markdown report.
//! - [`tokens`]: the design tokens of the code compared by value with the
//!   design variables.
//!
//! The library has no GPUI dependency; the capture program is the
//! `board_capture` example.

#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::disallowed_types, clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

pub mod classify;
pub mod compare;
pub mod deviations;
pub mod kit;
pub mod manifest;
pub mod matrix;
pub mod report;
pub mod stroke;
pub mod tokens;

use std::path::PathBuf;

/// Everything the instruments can fail with.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("{path}: {message}")]
    Toml { path: PathBuf, message: String },
    #[error("{path}: {source}")]
    Image {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
    #[error("{0}")]
    Invalid(String),
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io { path: path.into(), source }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Reads a whole text file.
pub fn read_text(path: &std::path::Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|e| Error::io(path, e))
}

/// Reads and parses a JSON file.
pub fn read_json<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Result<T> {
    let text = read_text(path)?;
    serde_json::from_str(&text).map_err(|source| Error::Json { path: path.to_path_buf(), source })
}

/// Writes `value` as pretty JSON with a trailing newline.
pub fn write_json<T: serde::Serialize>(path: &std::path::Path, value: &T) -> Result<()> {
    let mut text = serde_json::to_string_pretty(value)
        .map_err(|source| Error::Json { path: path.to_path_buf(), source })?;
    text.push('\n');
    write_text(path, &text)
}

/// Writes a text file, creating its directory.
pub fn write_text(path: &std::path::Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    std::fs::write(path, text).map_err(|e| Error::io(path, e))
}

/// The lowercase hex SHA-256 of a file.
pub fn sha256_file(path: &std::path::Path) -> Result<String> {
    use sha2::Digest as _;
    let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
    let digest = sha2::Sha256::digest(&bytes);
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}
