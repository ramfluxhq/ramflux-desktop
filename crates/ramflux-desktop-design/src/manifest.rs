// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The checked-in board reference manifest (`design/boards/manifest.json`).
//!
//! The reference PNGs are not vendored: they live in the design reference set
//! exported from the design file (`boards-desktop/` with its own
//! `manifest.json`), found through `RAMFLUX_DESIGN_REFS`. The checked-in
//! manifest pins every reference by key, node id, size, scale and SHA-256,
//! and pins the design file by its SHA-256, so a local set that drifted from
//! the reviewed export is detected before anything is compared.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// The environment variable naming the local reference directory.
pub const REFS_ENV: &str = "RAMFLUX_DESIGN_REFS";
/// Where the manifest lives, relative to the repository root.
pub const MANIFEST_PATH: &str = "design/boards/manifest.json";
/// The manifest format this code reads and writes.
pub const FORMAT_VERSION: u32 = 1;

/// One exported reference image of a design board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardRef {
    /// Board key as written in the design file (`02`, `A01`, `I24`).
    pub key: String,
    /// Pen node id of the top-level frame.
    pub node_id: String,
    pub module: String,
    pub title: String,
    /// Path inside the reference directory.
    pub file: String,
    /// Image size in pixels.
    pub width: u32,
    pub height: u32,
    /// Export scale (1 or 2).
    pub scale: u32,
    /// Frame size in design pixels.
    pub frame_width: u32,
    pub frame_height: u32,
    /// `light`, `dark` or `light+dark`.
    pub theme: String,
    /// Language of the board copy (`zh-CN`, `en`).
    pub language: String,
    pub state: String,
    #[serde(default)]
    pub flags: Vec<String>,
    #[serde(default)]
    pub owner_decision: Option<String>,
    /// Member of the priority baseline set.
    pub priority: bool,
    pub sha256: String,
}

/// The checked-in manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format_version: u32,
    /// The design file the references were exported from.
    pub design_file: String,
    pub design_file_sha256: String,
    pub export_date: String,
    /// SHA-256 of the reference set's own `manifest.json` at import time.
    pub source_manifest_sha256: String,
    pub boards: Vec<BoardRef>,
}

/// The reference set's own manifest (as exported).
#[derive(Debug, Deserialize)]
struct SourceManifest {
    design_file: String,
    design_file_sha256: String,
    export_date: String,
    boards: Vec<BoardRef>,
}

impl Manifest {
    /// Loads the manifest of the repository at `root`.
    pub fn load(root: &Path) -> Result<Self> {
        let manifest: Self = crate::read_json(&root.join(MANIFEST_PATH))?;
        if manifest.format_version != FORMAT_VERSION {
            return Err(Error::Invalid(format!(
                "{MANIFEST_PATH}: format_version {} is not {FORMAT_VERSION}",
                manifest.format_version
            )));
        }
        Ok(manifest)
    }

    /// Builds the manifest from a reference directory's own `manifest.json`.
    pub fn import(refs: &Path) -> Result<Self> {
        let path = refs.join("manifest.json");
        let source: SourceManifest = crate::read_json(&path)?;
        let mut boards = source.boards;
        boards.sort_by(|a, b| (&a.key, a.scale).cmp(&(&b.key, b.scale)));
        let manifest = Self {
            format_version: FORMAT_VERSION,
            design_file: source.design_file,
            design_file_sha256: source.design_file_sha256,
            export_date: source.export_date,
            source_manifest_sha256: crate::sha256_file(&path)?,
            boards,
        };
        manifest.validate()?;
        Ok(manifest)
    }

    /// Structural checks: unique (key, scale), sizes agree with the scale.
    pub fn validate(&self) -> Result<()> {
        let mut seen = BTreeSet::new();
        for board in &self.boards {
            if !seen.insert((board.key.as_str(), board.scale)) {
                return Err(Error::Invalid(format!(
                    "board {} at {}x is listed twice",
                    board.key, board.scale
                )));
            }
            if board.width != board.frame_width * board.scale
                || board.height != board.frame_height * board.scale
            {
                return Err(Error::Invalid(format!(
                    "board {} at {}x: image {}x{} does not match frame {}x{}",
                    board.key,
                    board.scale,
                    board.width,
                    board.height,
                    board.frame_width,
                    board.frame_height
                )));
            }
            if board.sha256.len() != 64 {
                return Err(Error::Invalid(format!("board {}: bad sha256", board.key)));
            }
        }
        Ok(())
    }

    /// The reference of `key` at `scale`, if exported.
    pub fn board(&self, key: &str, scale: u32) -> Option<&BoardRef> {
        self.boards.iter().find(|b| b.key == key && b.scale == scale)
    }

    /// Keys of the priority baseline set, in manifest order.
    pub fn priority_keys(&self) -> Vec<&str> {
        let mut keys: Vec<&str> = self
            .boards
            .iter()
            .filter(|b| b.priority && b.scale == 1)
            .map(|b| b.key.as_str())
            .collect();
        keys.dedup();
        keys
    }
}

/// The reference directory: an explicit path or `RAMFLUX_DESIGN_REFS`.
pub fn refs_dir(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    match std::env::var_os(REFS_ENV) {
        Some(path) if !path.is_empty() => Ok(PathBuf::from(path)),
        _ => Err(Error::Invalid(format!(
            "no reference directory: set {REFS_ENV} to the exported boards-desktop directory \
             or pass --refs"
        ))),
    }
}

/// One problem found while verifying a reference set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finding {
    Missing { file: String },
    Changed { file: String, expected: String, actual: String },
    WrongSize { file: String, expected: (u32, u32), actual: (u32, u32) },
    DesignFileChanged { expected: String, actual: String },
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing { file } => write!(f, "MISSING {file}"),
            Self::Changed { file, expected, actual } => {
                write!(f, "CHANGED {file}: sha256 {actual}, manifest {expected}")
            }
            Self::WrongSize { file, expected, actual } => write!(
                f,
                "SIZE {file}: {}x{}, manifest {}x{}",
                actual.0, actual.1, expected.0, expected.1
            ),
            Self::DesignFileChanged { expected, actual } => write!(
                f,
                "DESIGN FILE the reference set was exported from {actual}, manifest {expected}"
            ),
        }
    }
}

/// Which references a verification covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    All,
    Priority,
}

/// Verifies the reference directory against the manifest. Returns the
/// number of references checked and every finding.
pub fn verify(manifest: &Manifest, refs: &Path, scope: Scope) -> Result<(usize, Vec<Finding>)> {
    let mut findings = Vec::new();
    let source_path = refs.join("manifest.json");
    if source_path.is_file() {
        let source: SourceManifest = crate::read_json(&source_path)?;
        if source.design_file_sha256 != manifest.design_file_sha256 {
            findings.push(Finding::DesignFileChanged {
                expected: manifest.design_file_sha256.clone(),
                actual: source.design_file_sha256,
            });
        }
    }
    let mut checked = 0;
    for board in &manifest.boards {
        if scope == Scope::Priority && !board.priority {
            continue;
        }
        checked += 1;
        let path = refs.join(&board.file);
        if !path.is_file() {
            findings.push(Finding::Missing { file: board.file.clone() });
            continue;
        }
        let actual = crate::sha256_file(&path)?;
        if actual != board.sha256 {
            findings.push(Finding::Changed {
                file: board.file.clone(),
                expected: board.sha256.clone(),
                actual,
            });
            continue;
        }
        let (w, h) = image::image_dimensions(&path)
            .map_err(|source| Error::Image { path: path.clone(), source })?;
        if (w, h) != (board.width, board.height) {
            findings.push(Finding::WrongSize {
                file: board.file.clone(),
                expected: (board.width, board.height),
                actual: (w, h),
            });
        }
    }
    Ok((checked, findings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board(key: &str, scale: u32) -> BoardRef {
        BoardRef {
            key: key.into(),
            node_id: "n".into(),
            module: "0".into(),
            title: "t".into(),
            file: format!("{key}@{scale}.png"),
            width: 4 * scale,
            height: 3 * scale,
            scale,
            frame_width: 4,
            frame_height: 3,
            theme: "light".into(),
            language: "zh-CN".into(),
            state: "default".into(),
            flags: vec![],
            owner_decision: None,
            priority: true,
            sha256: "0".repeat(64),
        }
    }

    fn manifest(boards: Vec<BoardRef>) -> Manifest {
        Manifest {
            format_version: FORMAT_VERSION,
            design_file: "d.pen".into(),
            design_file_sha256: "a".repeat(64),
            export_date: "2026-11-23".into(),
            source_manifest_sha256: "b".repeat(64),
            boards,
        }
    }

    #[test]
    fn duplicate_and_size_errors_are_rejected() {
        assert!(manifest(vec![board("02", 1), board("02", 2)]).validate().is_ok());
        assert!(manifest(vec![board("02", 1), board("02", 1)]).validate().is_err());
        let mut wrong = board("02", 2);
        wrong.width = 4;
        assert!(manifest(vec![wrong]).validate().is_err());
    }

    #[test]
    fn verify_reports_missing_changed_and_matching_files() {
        let dir = std::env::temp_dir().join(format!("rf-design-verify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let image = image::RgbaImage::from_pixel(4, 3, image::Rgba([1, 2, 3, 255]));
        let good = dir.join("02@1.png");
        image.save(&good).unwrap();
        let mut ok = board("02", 1);
        ok.sha256 = crate::sha256_file(&good).unwrap();
        let mut changed = board("03", 1);
        changed.file = "02@1.png".into();
        let missing = board("04", 1);
        let (checked, findings) =
            verify(&manifest(vec![ok, changed, missing]), &dir, Scope::All).unwrap();
        assert_eq!(checked, 3);
        assert_eq!(findings.len(), 2);
        assert!(matches!(findings[0], Finding::Changed { .. }));
        assert!(matches!(findings[1], Finding::Missing { .. }));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
