// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Reviewed expected deviations, one TOML file per board:
//! `design/deviations/<board-key>.toml`.
//!
//! ```toml
//! board = "02"
//! capture = "02__zh-CN__light__1440x900"
//! reference_sha256 = "41db…"
//!
//! [[deviation]]
//! id = "r01"
//! region = { x = 68, y = 0, w = 264, h = 900 }
//! class = "layout"
//! reason = "List column is 264 px wide; the board draws 332 px."
//! decision = "fix"            # unreviewed | accepted | fix
//! reviewer = "lead"
//!
//! [no_capture]                 # a board that cannot be captured yet
//! reason = "No search states in code (B39)."
//! decision = "unreviewed"
//! ```
//!
//! Rules:
//! - A differing region is known when at least 95 % of its differing pixels
//!   lie inside the entries' regions grown by 4 design pixels. Anything else
//!   is NEW and fails `--check`.
//! - Every recorded entry is known whatever its decision; `--review` lists
//!   the `unreviewed` ones. `accepted` means the difference is intended (the
//!   board is out of date or the owner decided otherwise); `fix` means it is
//!   a confirmed defect scheduled for an alignment wave.
//! - An entry with no differing pixel left is reported as resolved: remove it
//!   in the change that aligned the screen. Entries are never widened to hide
//!   a new difference.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::compare::{Comparison, Rect, Region};
use crate::{Error, Result};

/// Directory of the per-board files, relative to the repository root.
pub const DEVIATIONS_DIR: &str = "design/deviations";
/// Growth of recorded regions when matching, in design pixels.
pub const MATCH_GROWTH: u32 = 4;
/// Share of a region's differing pixels that must be covered.
pub const MATCH_SHARE: f64 = 0.95;
/// Reason written for entries recorded without a review.
pub const UNREVIEWED: &str = "UNREVIEWED";

/// A reviewer decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    #[default]
    Unreviewed,
    Accepted,
    Fix,
}

impl Decision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unreviewed => "unreviewed",
            Self::Accepted => "accepted",
            Self::Fix => "fix",
        }
    }
}

/// One expected deviation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub region: Rect,
    /// Error class: token, spacing, type, missing, layout, copy, generated,
    /// fixture, rasterisation (free text, empty until reviewed).
    #[serde(default)]
    pub class: String,
    pub reason: String,
    #[serde(default)]
    pub decision: Decision,
    #[serde(default)]
    pub reviewer: String,
}

/// A recorded reason why a board has no capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoCapture {
    pub reason: String,
    #[serde(default)]
    pub decision: Decision,
    #[serde(default)]
    pub reviewer: String,
}

/// A recorded capture size that differs from the board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizeDeviation {
    /// The capture size the entry was recorded for, `<w>x<h>` (design px).
    pub capture: String,
    pub reason: String,
    #[serde(default)]
    pub decision: Decision,
    #[serde(default)]
    pub reviewer: String,
}

/// The expected deviations of one board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardDeviations {
    pub board: String,
    #[serde(default)]
    pub capture: String,
    #[serde(default)]
    pub reference_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_capture: Option<NoCapture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<SizeDeviation>,
    #[serde(default, rename = "deviation")]
    pub deviations: Vec<Entry>,
}

impl BoardDeviations {
    pub fn empty(board: &str) -> Self {
        Self {
            board: board.to_owned(),
            capture: String::new(),
            reference_sha256: String::new(),
            no_capture: None,
            size: None,
            deviations: Vec::new(),
        }
    }

    pub fn path(dir: &Path, board: &str) -> PathBuf {
        dir.join(format!("{board}.toml"))
    }

    /// Loads the file of `board`, or an empty set when there is none.
    pub fn load(dir: &Path, board: &str) -> Result<Self> {
        let path = Self::path(dir, board);
        if !path.is_file() {
            return Ok(Self::empty(board));
        }
        let text = crate::read_text(&path)?;
        let parsed: Self = toml::from_str(&text)
            .map_err(|e| Error::Toml { path: path.clone(), message: e.to_string() })?;
        if parsed.board != board {
            return Err(Error::Invalid(format!(
                "{}: board is {:?}, expected {board:?}",
                path.display(),
                parsed.board
            )));
        }
        Ok(parsed)
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        let path = Self::path(dir, &self.board);
        let body = toml::to_string_pretty(self)
            .map_err(|e| Error::Toml { path: path.clone(), message: e.to_string() })?;
        let header = format!(
            "# Expected deviations of design board {}. Format and rules:\n\
             # crates/ramflux-desktop-design/src/deviations.rs and docs/design-instrument.md.\n\
             # decision: unreviewed | accepted | fix. Never widen a region to hide a new difference.\n\n",
            self.board
        );
        crate::write_text(&path, &(header + &body))
    }

    /// Entries still waiting for a reviewer.
    pub fn unreviewed(&self) -> impl Iterator<Item = &Entry> {
        self.deviations.iter().filter(|e| e.decision == Decision::Unreviewed)
    }

    fn rects(&self) -> Vec<Rect> {
        self.deviations.iter().map(|e| e.region.grow(MATCH_GROWTH)).collect()
    }
}

/// A region with its match against the recorded entries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Matched {
    #[serde(flatten)]
    pub region: Region,
    pub known: bool,
}

/// Matches the deviation regions of `comparison` against `expected`.
/// Returns the matched regions and the ids of resolved entries.
pub fn match_regions(
    comparison: &Comparison,
    expected: &BoardDeviations,
) -> (Vec<Matched>, Vec<String>) {
    let rects = expected.rects();
    let matched = comparison
        .regions
        .iter()
        .map(|region| {
            let known = !rects.is_empty() && {
                let covered = comparison.differing_covered(region.rect, &rects);
                covered as f64 >= MATCH_SHARE * region.pixels as f64
            };
            Matched { region: region.clone(), known }
        })
        .collect();
    let resolved = expected
        .deviations
        .iter()
        .filter(|e| comparison.differing_in(e.region) == 0)
        .map(|e| e.id.clone())
        .collect();
    (matched, resolved)
}

/// Records every NEW region of a comparison as an unreviewed entry, keeping
/// existing entries (and their reasons) that still have differences.
pub fn record(
    mut expected: BoardDeviations,
    capture: &str,
    reference_sha256: &str,
    matched: &[Matched],
    resolved: &[String],
) -> BoardDeviations {
    expected.capture = capture.to_owned();
    expected.reference_sha256 = reference_sha256.to_owned();
    expected.no_capture = None;
    // Ids are never reused, also not those of resolved entries.
    let mut next = expected
        .deviations
        .iter()
        .filter_map(|e| e.id.strip_prefix('r').and_then(|n| n.parse::<u32>().ok()))
        .max()
        .unwrap_or(0);
    expected.deviations.retain(|e| !resolved.contains(&e.id));
    let mut fresh: Vec<&Matched> = matched.iter().filter(|m| !m.known).collect();
    fresh.sort_by_key(|m| (m.region.rect.y, m.region.rect.x));
    for m in fresh {
        next += 1;
        expected.deviations.push(Entry {
            id: format!("r{next:02}"),
            region: m.region.rect,
            class: String::new(),
            reason: UNREVIEWED.to_owned(),
            decision: Decision::Unreviewed,
            reviewer: String::new(),
        });
    }
    expected
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::{Options, compare};
    use image::{Rgba, RgbaImage};

    fn pair(block: (u32, u32, u32, u32)) -> Comparison {
        let a = RgbaImage::from_pixel(100, 80, Rgba([250, 250, 250, 255]));
        let mut b = a.clone();
        for y in block.1..block.1 + block.3 {
            for x in block.0..block.0 + block.2 {
                b.put_pixel(x, y, Rgba([20, 20, 20, 255]));
            }
        }
        compare(&a, &b, 1, Options::default())
    }

    fn entry(id: &str, region: Rect, decision: Decision) -> Entry {
        Entry {
            id: id.into(),
            region,
            class: String::new(),
            reason: "reviewed".into(),
            decision,
            reviewer: "lead".into(),
        }
    }

    #[test]
    fn recorded_regions_are_known_and_new_ones_are_not() {
        let comparison = pair((10, 10, 20, 10));
        let mut expected = BoardDeviations::empty("02");
        let (matched, _) = match_regions(&comparison, &expected);
        assert_eq!(matched.len(), 1);
        assert!(!matched[0].known, "no entry: the region is new");
        // A slightly smaller recorded region still covers it (grown by 4).
        expected.deviations.push(entry("r01", Rect { x: 12, y: 11, w: 17, h: 8 }, Decision::Fix));
        let (matched, resolved) = match_regions(&comparison, &expected);
        assert!(matched[0].known);
        assert!(resolved.is_empty());
        // A region elsewhere does not cover it.
        expected.deviations[0].region = Rect { x: 60, y: 50, w: 10, h: 10 };
        let (matched, resolved) = match_regions(&comparison, &expected);
        assert!(!matched[0].known);
        assert_eq!(resolved, vec!["r01".to_owned()]);
    }

    #[test]
    fn record_adds_unreviewed_entries_and_drops_resolved_ones() {
        let comparison = pair((10, 10, 20, 10));
        let mut expected = BoardDeviations::empty("02");
        expected.deviations.push(entry("r07", Rect { x: 60, y: 50, w: 10, h: 10 }, Decision::Fix));
        let (matched, resolved) = match_regions(&comparison, &expected);
        let recorded = record(expected, "cap", "sha", &matched, &resolved);
        assert_eq!(recorded.deviations.len(), 1);
        let e = &recorded.deviations[0];
        assert_eq!(e.id, "r08");
        assert_eq!(e.decision, Decision::Unreviewed);
        assert_eq!(e.reason, UNREVIEWED);
        assert_eq!(e.region, Rect { x: 10, y: 10, w: 20, h: 10 });
        let (again, _) = match_regions(&comparison, &recorded);
        assert!(again.iter().all(|m| m.known));
    }

    #[test]
    fn files_round_trip() {
        let dir = std::env::temp_dir().join(format!("rf-design-dev-{}", std::process::id()));
        let mut set = BoardDeviations::empty("A01");
        set.capture = "A01__zh-CN__light__1440x900".into();
        set.deviations.push(entry("r01", Rect { x: 1, y: 2, w: 3, h: 4 }, Decision::Accepted));
        set.no_capture = Some(NoCapture {
            reason: "why".into(),
            decision: Decision::Unreviewed,
            reviewer: String::new(),
        });
        set.save(&dir).unwrap();
        assert_eq!(BoardDeviations::load(&dir, "A01").unwrap(), set);
        assert_eq!(BoardDeviations::load(&dir, "A02").unwrap(), BoardDeviations::empty("A02"));
        assert!(BoardDeviations::load(&dir, "A01").unwrap().unreviewed().next().is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
