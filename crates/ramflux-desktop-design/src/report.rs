// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The comparison run over a set of boards: per-board results, the
//! machine-readable `result.json`, the Markdown `report.md` sorted worst
//! first and one three-up image per compared board.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::compare::{self, Crop, Options};
use crate::deviations::{self, BoardDeviations, Matched};
use crate::manifest::Manifest;
use crate::matrix::{self, CaptureLog, CaptureName};
use crate::{Error, Result};

/// The outcome of one board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// A deviation or a size mismatch nobody recorded.
    New,
    /// No capture and no recorded reason.
    NoCaptureNew,
    /// No reference image in the local set.
    NoReference,
    /// Every deviation is recorded in the board's file.
    Known,
    /// No capture, with a recorded reason.
    NoCaptureKnown,
    /// Within tolerance: no deviation region and the size matches.
    Pass,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::New => "NEW",
            Self::NoCaptureNew => "no capture (NEW)",
            Self::NoReference => "no reference",
            Self::Known => "known",
            Self::NoCaptureKnown => "no capture (known)",
            Self::Pass => "pass",
        }
    }

    /// Fails `--check`.
    pub fn fails_check(self) -> bool {
        matches!(self, Self::New | Self::NoCaptureNew | Self::NoReference)
    }
}

/// The result of one board.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardResult {
    pub key: String,
    pub title: String,
    pub module: String,
    pub state: String,
    pub flags: Vec<String>,
    pub theme: String,
    pub locale: String,
    pub capture: String,
    pub reference: String,
    pub status: Status,
    /// Capture note from the capture program, or why there is none.
    pub note: String,
    pub missing_landmarks: Vec<String>,
    /// Device pixels per design pixel of the comparison.
    pub scale: u32,
    /// Sizes in design pixels.
    pub reference_size: (u32, u32),
    pub capture_size: (u32, u32),
    pub compared_size: (u32, u32),
    pub crop: Option<Crop>,
    pub size_match: bool,
    /// Share of differing pixels beyond the anti-aliasing tolerance.
    pub diff_ratio: f64,
    /// Share of pixels beyond the channel threshold, without tolerance.
    pub strict_ratio: f64,
    /// Share of pixels beyond the tint threshold (close surface colours).
    pub tint_ratio: f64,
    /// Mean SSIM of the luminance (1.0 identical).
    pub ssim: f64,
    pub regions: Vec<Matched>,
    pub noise_regions: usize,
    pub new_regions: usize,
    pub known_regions: usize,
    /// Recorded entries with no difference left.
    pub resolved: Vec<String>,
    /// Recorded entries still unreviewed.
    pub unreviewed: usize,
    /// Three-up image, relative to the output directory.
    pub three_up: Option<String>,
}

/// The whole run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub design_file_sha256: String,
    pub threshold: u8,
    pub neighbourhood: u32,
    pub min_extent: u32,
    pub capture_environment: Vec<String>,
    pub boards: Vec<BoardResult>,
}

impl RunResult {
    pub fn count(&self, status: Status) -> usize {
        self.boards.iter().filter(|b| b.status == status).count()
    }

    pub fn fails_check(&self) -> bool {
        self.boards.iter().any(|b| b.status.fails_check())
    }
}

/// What a run compares and where it writes.
#[derive(Debug, Clone)]
pub struct RunOptions {
    pub refs: PathBuf,
    pub captures: PathBuf,
    pub out: PathBuf,
    pub deviations: PathBuf,
    /// Board keys; empty means the priority set.
    pub boards: Vec<String>,
    /// Compare 2x captures with 2x references when both exist.
    pub prefer_2x: bool,
    /// Write NEW regions into the per-board files as unreviewed entries.
    pub record: bool,
    pub compare: Options,
}

fn load_rgba(path: &Path) -> Result<image::RgbaImage> {
    let image =
        image::open(path).map_err(|source| Error::Image { path: path.to_path_buf(), source })?;
    Ok(compare::flatten(&image.to_rgba8()))
}

/// Runs the comparison over the boards and writes the results.
pub fn run(manifest: &Manifest, options: &RunOptions) -> Result<RunResult> {
    let log_path = options.captures.join("captures.json");
    let log: CaptureLog =
        if log_path.is_file() { crate::read_json(&log_path)? } else { CaptureLog::default() };
    let keys: Vec<String> = if options.boards.is_empty() {
        manifest.priority_keys().into_iter().map(str::to_owned).collect()
    } else {
        options.boards.clone()
    };
    std::fs::create_dir_all(options.out.join("three-up"))
        .map_err(|e| Error::io(options.out.join("three-up"), e))?;
    let mut boards = Vec::new();
    for key in &keys {
        let reference = manifest
            .board(key, 1)
            .ok_or_else(|| Error::Invalid(format!("board {key} is not in the manifest")))?;
        let name = matrix::board_capture(
            key,
            &reference.language,
            &reference.theme,
            (reference.frame_width, reference.frame_height),
        );
        let expected = BoardDeviations::load(&options.deviations, key)?;
        let result = compare_board(manifest, options, &log, &name, expected)?;
        println!(
            "{key:6} {:20} diff {:6.2}%  ssim {:.3}  regions {} new / {} known",
            result.status.as_str(),
            result.diff_ratio * 100.0,
            result.ssim,
            result.new_regions,
            result.known_regions
        );
        boards.push(result);
    }
    let run = RunResult {
        design_file_sha256: manifest.design_file_sha256.clone(),
        threshold: options.compare.threshold,
        neighbourhood: options.compare.neighbourhood,
        min_extent: options.compare.min_extent,
        capture_environment: log.environment.clone(),
        boards,
    };
    crate::write_json(&options.out.join("result.json"), &run)?;
    crate::write_text(&options.out.join("report.md"), &markdown(&run))?;
    Ok(run)
}

fn compare_board(
    manifest: &Manifest,
    options: &RunOptions,
    log: &CaptureLog,
    name: &CaptureName,
    expected: BoardDeviations,
) -> Result<BoardResult> {
    let key = name.key.as_str();
    let mut reference = manifest
        .board(key, 1)
        .ok_or_else(|| Error::Invalid(format!("board {key} is not in the manifest")))?;
    let record = log.find(name);
    let mut result = BoardResult {
        key: key.to_owned(),
        title: reference.title.clone(),
        module: reference.module.clone(),
        state: reference.state.clone(),
        flags: reference.flags.clone(),
        theme: name.theme.clone(),
        locale: name.locale.clone(),
        capture: name.file_name(),
        reference: reference.file.clone(),
        status: Status::NoCaptureNew,
        note: record.map(|r| r.note.clone()).unwrap_or_default(),
        missing_landmarks: record.map(|r| r.missing_landmarks.clone()).unwrap_or_default(),
        scale: 1,
        reference_size: (reference.frame_width, reference.frame_height),
        capture_size: (0, 0),
        compared_size: (0, 0),
        crop: None,
        size_match: false,
        diff_ratio: 0.0,
        strict_ratio: 0.0,
        tint_ratio: 0.0,
        ssim: 0.0,
        regions: Vec::new(),
        noise_regions: 0,
        new_regions: 0,
        known_regions: 0,
        resolved: Vec::new(),
        unreviewed: expected.unreviewed().count(),
        three_up: None,
    };
    let capture_path = options.captures.join(name.file_name());
    if !capture_path.is_file() {
        if result.note.is_empty() {
            result.note = "no capture file".to_owned();
        }
        result.status = if expected.no_capture.is_some() {
            Status::NoCaptureKnown
        } else {
            Status::NoCaptureNew
        };
        if options.record && expected.no_capture.is_none() {
            let mut updated = expected;
            updated.capture = name.stem();
            updated.reference_sha256 = reference.sha256.clone();
            updated.no_capture = Some(deviations::NoCapture {
                reason: format!("{} ({})", deviations::UNREVIEWED, result.note),
                decision: deviations::Decision::Unreviewed,
                reviewer: String::new(),
            });
            updated.save(&options.deviations)?;
        }
        return Ok(result);
    }
    let capture = load_rgba(&capture_path)?;
    let capture_scale = (capture.width() / name.width.max(1)).max(1);
    let mut scale = 1;
    if options.prefer_2x
        && capture_scale == 2
        && let Some(two) = manifest.board(key, 2)
    {
        reference = two;
        scale = 2;
    }
    let reference_path = options.refs.join(&reference.file);
    if !reference_path.is_file() {
        result.status = Status::NoReference;
        result.note = format!("reference {} missing in {}", reference.file, options.refs.display());
        return Ok(result);
    }
    let reference_image = load_rgba(&reference_path)?;
    let capture = if capture_scale > scale {
        compare::downsample(&capture, capture_scale / scale)
    } else {
        capture
    };
    let comparison = compare::compare(&reference_image, &capture, scale, options.compare);
    let (matched, resolved) = deviations::match_regions(&comparison, &expected);
    result.scale = scale;
    result.reference = reference.file.clone();
    result.capture_size = (capture.width() / scale, capture.height() / scale);
    result.compared_size = (comparison.width / scale, comparison.height / scale);
    result.crop = Some(comparison.crop);
    result.size_match = comparison.crop != Crop::Mismatch;
    result.diff_ratio = comparison.ratio();
    result.strict_ratio = comparison.strict_ratio();
    result.tint_ratio = comparison.tint_ratio();
    result.ssim = comparison.ssim;
    result.noise_regions = comparison.noise_regions;
    result.new_regions = matched.iter().filter(|m| !m.known).count();
    result.known_regions = matched.len() - result.new_regions;
    result.resolved = resolved.clone();
    let size_known = expected.size.as_ref().is_some_and(|s| {
        s.capture == format!("{}x{}", result.capture_size.0, result.capture_size.1)
    });
    result.status = if !result.size_match && !size_known || result.new_regions > 0 {
        Status::New
    } else if matched.is_empty() && result.size_match {
        Status::Pass
    } else {
        Status::Known
    };
    // Heat map with outlines, then the three-up.
    let mut heat = compare::heat_map(&capture, &comparison);
    for entry in &expected.deviations {
        let colour = if resolved.contains(&entry.id) { compare::RESOLVED } else { compare::KNOWN };
        compare::outline(&mut heat, entry.region, scale, colour);
    }
    for m in matched.iter().filter(|m| !m.known) {
        compare::outline(&mut heat, m.region.rect, scale, compare::NEW);
    }
    let three = compare::three_up(&reference_image, &capture, &heat, scale);
    let rel = format!("three-up/{}.png", name.stem());
    let path = options.out.join(&rel);
    three.save(&path).map_err(|source| Error::Image { path: path.clone(), source })?;
    result.three_up = Some(rel);
    if options.record {
        let mut updated =
            deviations::record(expected, &name.stem(), &reference.sha256, &matched, &resolved);
        if !result.size_match && !size_known {
            updated.size = Some(deviations::SizeDeviation {
                capture: format!("{}x{}", result.capture_size.0, result.capture_size.1),
                reason: deviations::UNREVIEWED.to_owned(),
                decision: deviations::Decision::Unreviewed,
                reviewer: String::new(),
            });
        }
        result.unreviewed = updated.unreviewed().count();
        updated.save(&options.deviations)?;
    }
    result.regions = matched;
    Ok(result)
}

/// The Markdown report, worst first.
pub fn markdown(run: &RunResult) -> String {
    let mut out = String::new();
    let captured: Vec<&BoardResult> = {
        let mut v: Vec<&BoardResult> = run.boards.iter().filter(|b| b.crop.is_some()).collect();
        v.sort_by(|a, b| b.diff_ratio.total_cmp(&a.diff_ratio));
        v
    };
    let others: Vec<&BoardResult> = run.boards.iter().filter(|b| b.crop.is_none()).collect();
    out.push_str("# Board comparison\n\n");
    out.push_str(&format!(
        "Design file sha256 `{}`. Threshold {}/255, neighbourhood {} design px, deviation above {} design px.\n\n",
        run.design_file_sha256, run.threshold, run.neighbourhood, run.min_extent
    ));
    if !run.capture_environment.is_empty() {
        out.push_str("Capture environment:\n\n");
        for line in &run.capture_environment {
            out.push_str(&format!("- {line}\n"));
        }
        out.push('\n');
    }
    out.push_str("| Boards | Compared | Pass | Known | NEW | No capture (known) | No capture (NEW) | No reference |\n");
    out.push_str("|---|---|---|---|---|---|---|---|\n");
    out.push_str(&format!(
        "| {} | {} | {} | {} | {} | {} | {} | {} |\n\n",
        run.boards.len(),
        captured.len(),
        run.count(Status::Pass),
        run.count(Status::Known),
        run.count(Status::New),
        run.count(Status::NoCaptureKnown),
        run.count(Status::NoCaptureNew),
        run.count(Status::NoReference),
    ));
    out.push_str("## Compared boards, worst first\n\n");
    out.push_str("Diff = share of pixels differing beyond the anti-aliasing tolerance; strict = beyond the channel threshold alone; tint = beyond 4/255 (close surface colours such as a wrong background token). SSIM 1.000 is identical. Regions are design pixels `x,y wxh`. Crop `top` = the top 900 px of a taller board.\n\n");
    out.push_str("| # | Board | Title | Status | Diff | Strict | Tint | SSIM | Regions new/known | Crop | Largest regions | Three-up |\n");
    out.push_str("|---|---|---|---|---|---|---|---|---|---|---|---|\n");
    for (i, b) in captured.iter().enumerate() {
        let largest: Vec<String> =
            b.regions.iter().take(3).map(|m| format!("`{}`", m.region.rect)).collect();
        out.push_str(&format!(
            "| {} | {} | {} | {} | {:.1}% | {:.1}% | {:.1}% | {:.3} | {}/{} | {} | {} | {} |\n",
            i + 1,
            b.key,
            b.title.replace('|', "/"),
            b.status.as_str(),
            b.diff_ratio * 100.0,
            b.strict_ratio * 100.0,
            b.tint_ratio * 100.0,
            b.ssim,
            b.new_regions,
            b.known_regions,
            match b.crop {
                Some(Crop::Full) => "full",
                Some(Crop::Top) => "top",
                Some(Crop::Mismatch) => "SIZE",
                None => "-",
            },
            largest.join(" "),
            b.three_up.as_ref().map_or_else(String::new, |p| format!("[png]({p})")),
        ));
    }
    if !others.is_empty() {
        out.push_str(
            "\n## Not compared\n\n| Board | Title | Status | Reason |\n|---|---|---|---|\n",
        );
        for b in others {
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                b.key,
                b.title.replace('|', "/"),
                b.status.as_str(),
                b.note.replace('|', "/")
            ));
        }
    }
    out
}

/// What needs a reviewer: unreviewed entries, NEW regions, resolved entries.
pub fn review(run: &RunResult, deviations_dir: &Path) -> Result<String> {
    let mut out = String::from("# Review\n\n");
    let mut items = 0;
    for b in &run.boards {
        let expected = BoardDeviations::load(deviations_dir, &b.key)?;
        let unreviewed: Vec<_> = expected.unreviewed().collect();
        let new: Vec<_> = b.regions.iter().filter(|m| !m.known).collect();
        let no_capture =
            expected.no_capture.as_ref().filter(|n| n.decision == deviations::Decision::Unreviewed);
        if unreviewed.is_empty() && new.is_empty() && b.resolved.is_empty() && no_capture.is_none()
        {
            continue;
        }
        out.push_str(&format!("## {} {} ({})\n\n", b.key, b.title, b.status.as_str()));
        if let Some(n) = no_capture {
            out.push_str(&format!("- no capture, unreviewed: {}\n", n.reason));
            items += 1;
        }
        for m in new {
            out.push_str(&format!("- NEW region `{}` ({} px)\n", m.region.rect, m.region.pixels));
            items += 1;
        }
        for e in unreviewed {
            out.push_str(&format!("- unreviewed {} `{}`\n", e.id, e.region));
            items += 1;
        }
        for id in &b.resolved {
            out.push_str(&format!("- resolved {id}: remove it from the board file\n"));
            items += 1;
        }
        if let Some(p) = &b.three_up {
            out.push_str(&format!("- three-up: {p}\n"));
        }
        out.push('\n');
    }
    out.push_str(&format!("{items} items need a decision.\n"));
    Ok(out)
}
