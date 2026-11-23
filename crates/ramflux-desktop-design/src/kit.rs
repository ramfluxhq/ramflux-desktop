// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Kit components compared one by one with the component boards.
//!
//! `design/kit/components.toml` lists every component of the boards 00B,
//! 00D and B19b with its rectangle. The capture program renders the
//! application's version of each component alone (`kit__<slug>.png`, plus
//! `kit.json` with how it was rendered or why not); this module crops the
//! reference to the rectangle and compares the two with the board rules.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::compare::{self, Options, Rect};
use crate::manifest::Manifest;
use crate::{Error, Result};

pub const COMPONENTS_PATH: &str = "design/kit/components.toml";

/// One component on a component board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KitComponent {
    pub slug: String,
    pub name: String,
    pub board: String,
    pub rect: Rect,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
struct KitFile {
    component: Vec<KitComponent>,
}

/// Loads the component list of the repository at `root`.
pub fn load(root: &Path) -> Result<Vec<KitComponent>> {
    let path = root.join(COMPONENTS_PATH);
    let text = crate::read_text(&path)?;
    let file: KitFile =
        toml::from_str(&text).map_err(|e| Error::Toml { path, message: e.to_string() })?;
    Ok(file.component)
}

/// What the capture program did for one component (`kit.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KitRecord {
    pub slug: String,
    /// `app`, `replica` or `skipped`.
    pub source: String,
    pub note: String,
}

/// The capture file of a component.
pub fn capture_name(slug: &str) -> String {
    format!("kit__{slug}.png")
}

/// The result for one component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KitResult {
    pub slug: String,
    pub name: String,
    pub board: String,
    pub source: String,
    pub note: String,
    pub diff_ratio: f64,
    pub ssim: f64,
    pub regions: usize,
    /// Regions left after the lead's acceptance rules (window chrome, pure
    /// glyph rasterisation); zero means within tolerance.
    pub open_regions: usize,
    /// Class of each open region (`crate::classify`).
    pub open_classes: Vec<String>,
    pub largest: Vec<Rect>,
    pub three_up: Option<String>,
}

/// Compares every captured component and writes `kit.json` and `kit.md`
/// into `out`.
pub fn run(
    root: &Path,
    manifest: &Manifest,
    refs: &Path,
    captures: &Path,
    out: &Path,
) -> Result<Vec<KitResult>> {
    let components = load(root)?;
    let records: Vec<KitRecord> = {
        let path = captures.join("kit.json");
        if path.is_file() { crate::read_json(&path)? } else { Vec::new() }
    };
    let dir = out.join("kit-three-up");
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    let mut results = Vec::new();
    for component in &components {
        let record = records.iter().find(|r| r.slug == component.slug);
        let mut result = KitResult {
            slug: component.slug.clone(),
            name: component.name.clone(),
            board: component.board.clone(),
            source: record.map_or_else(|| "skipped".to_owned(), |r| r.source.clone()),
            note: record.map_or_else(|| "no capture".to_owned(), |r| r.note.clone()),
            diff_ratio: 0.0,
            ssim: 0.0,
            regions: 0,
            open_regions: 0,
            open_classes: Vec::new(),
            largest: Vec::new(),
            three_up: None,
        };
        let capture_path = captures.join(capture_name(&component.slug));
        let board = manifest.board(&component.board, 1).ok_or_else(|| {
            Error::Invalid(format!("kit board {} is not in the manifest", component.board))
        })?;
        let board_path: PathBuf = refs.join(&board.file);
        if capture_path.is_file() && board_path.is_file() {
            let open = |p: &Path| -> Result<image::RgbaImage> {
                image::open(p)
                    .map(|i| compare::flatten(&i.to_rgba8()))
                    .map_err(|source| Error::Image { path: p.to_path_buf(), source })
            };
            let board_image = open(&board_path)?;
            let r = component.rect;
            let reference = image::imageops::crop_imm(&board_image, r.x, r.y, r.w, r.h).to_image();
            let capture = open(&capture_path)?;
            let scale = (capture.width() / r.w.max(1)).max(1);
            let capture = compare::downsample(&capture, scale);
            let options = Options { max_height: None, ..Options::default() };
            let c = compare::compare(&reference, &capture, 1, options);
            result.diff_ratio = c.ratio();
            result.ssim = c.ssim;
            result.regions = c.regions.len();
            // The structure rule (a region over 15 % of a screen) is a
            // screen rule; a component is measured against a board-sized
            // area so its regions get their geometric classes.
            let area = 1440 * 900;
            for g in &c.regions {
                let m = crate::classify::measure(&reference, &capture, g.rect, 1);
                let at = crate::classify::classify(&m, g.rect, area);
                if at.decision != crate::deviations::Decision::Accepted {
                    result.open_regions += 1;
                    result.open_classes.push(at.class.to_owned());
                }
            }
            result.largest = c.regions.iter().take(3).map(|g| g.rect).collect();
            let mut heat = compare::heat_map(&capture, &c);
            for g in &c.regions {
                compare::outline(&mut heat, g.rect, 1, compare::NEW);
            }
            let three = compare::three_up(&reference, &capture, &heat, 1);
            let rel = format!("kit-three-up/{}.png", component.slug);
            let path = out.join(&rel);
            three.save(&path).map_err(|source| Error::Image { path: path.clone(), source })?;
            result.three_up = Some(rel);
        }
        results.push(result);
    }
    crate::write_json(&out.join("kit.json"), &results)?;
    crate::write_text(&out.join("kit.md"), &markdown(&results))?;
    Ok(results)
}

/// The kit report, worst first.
pub fn markdown(results: &[KitResult]) -> String {
    let mut compared: Vec<&KitResult> = results.iter().filter(|r| r.three_up.is_some()).collect();
    compared.sort_by(|a, b| b.diff_ratio.total_cmp(&a.diff_ratio));
    let mut out = String::from("# Kit components\n\n");
    out.push_str(&format!(
        "{} components on the boards, {} compared, {} not capturable.\n\n",
        results.len(),
        compared.len(),
        results.len() - compared.len()
    ));
    let within = compared.iter().filter(|r| r.open_regions == 0).count();
    out.push_str(&format!(
        "Within tolerance (no region left after the chrome and rasterisation rules): {within} of {}.\n\n",
        compared.len()
    ));
    out.push_str("| # | Component | Board | Source | Diff | SSIM | Regions | Open (classes) | Largest | Three-up |\n|---|---|---|---|---|---|---|---|---|---|\n");
    for (i, r) in compared.iter().enumerate() {
        let largest: Vec<String> = r.largest.iter().map(|g| format!("`{g}`")).collect();
        out.push_str(&format!(
            "| {} | {} | {} | {} | {:.1}% | {:.3} | {} | {} {} | {} | [png]({}) |\n",
            i + 1,
            r.name,
            r.board,
            r.source,
            r.diff_ratio * 100.0,
            r.ssim,
            r.regions,
            r.open_regions,
            if r.open_classes.is_empty() {
                String::new()
            } else {
                format!("({})", r.open_classes.join(", "))
            },
            largest.join(" "),
            r.three_up.clone().unwrap_or_default()
        ));
    }
    out.push_str("\n## Not compared\n\n| Component | Reason |\n|---|---|\n");
    for r in results.iter().filter(|r| r.three_up.is_none()) {
        out.push_str(&format!("| {} | {} |\n", r.name, r.note));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_repository_component_list_loads_with_unique_slugs() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let components = load(&root).unwrap();
        assert!(components.len() >= 50);
        let mut slugs: Vec<&str> = components.iter().map(|c| c.slug.as_str()).collect();
        slugs.sort_unstable();
        let before = slugs.len();
        slugs.dedup();
        assert_eq!(before, slugs.len(), "slugs are unique");
        assert!(components.iter().all(|c| ["00B", "00D", "B19b"].contains(&c.board.as_str())));
    }
}
