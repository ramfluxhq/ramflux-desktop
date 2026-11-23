// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Board captures: renders each mapped screen over the mock source,
//! offscreen, at the board's frame size and the window's scale (2x on the
//! headless platform), one capture per board key and matrix cell, named
//! `<key>__<locale>__<theme>__<w>x<h>.png`, plus `captures.json`.
//!
//! ```sh
//! TZ=UTC cargo run -p ramflux-desktop-design --example board_capture -- \
//!     --out /tmp/board-captures [--boards 02,B01|priority] \
//!     [--themes board|light,dark] [--locales board|zh-CN,en,en-XA,ar-XB] \
//!     [--sizes board|1280x820,1024x768,800x900,640x480]
//! ```
//!
//! `board` (the default for each dimension) means the board's own theme,
//! language and frame size (taller canvases are captured 900 high).

mod harness;
mod kit;
mod recipes;

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ramflux_desktop_design::manifest::Manifest;
use ramflux_desktop_design::matrix::{CaptureLog, CaptureName, CaptureRecord, WindowSize};

use harness::{Run, shipped_locale};
use recipes::Plan;

struct Options {
    out: PathBuf,
    kit: bool,
    boards: Vec<String>,
    themes: Option<Vec<String>>,
    locales: Option<Vec<String>>,
    sizes: Vec<WindowSize>,
}

fn list(value: &str) -> Option<Vec<String>> {
    if value == "board" {
        return None;
    }
    Some(value.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect())
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut out = std::env::var_os("RAMFLUX_CAPTURE_DIR").map(PathBuf::from);
    let mut boards = vec!["priority".to_owned()];
    let mut themes = None;
    let mut locales = None;
    let mut sizes = vec![WindowSize::Board];
    let mut kit = false;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--kit" {
            kit = true;
            i += 1;
            continue;
        }
        let value = args.get(i + 1).cloned().ok_or(format!("{} needs a value", args[i]))?;
        match args[i].as_str() {
            "--out" => out = Some(PathBuf::from(value)),
            "--boards" => boards = list(&value).unwrap_or_default(),
            "--themes" => themes = list(&value),
            "--locales" => locales = list(&value),
            "--sizes" => {
                sizes = value
                    .split(',')
                    .map(|s| WindowSize::parse(s.trim()).ok_or(format!("bad size {s}")))
                    .collect::<Result<_, _>>()?;
            }
            other => return Err(format!("unknown argument {other}")),
        }
        i += 2;
    }
    let out = out.ok_or("pass --out <dir> or set RAMFLUX_CAPTURE_DIR")?;
    Ok(Options { out, kit, boards, themes, locales, sizes })
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn main() -> ExitCode {
    harness::run_as_failing_daemon();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", include_str!("usage.txt"));
        return ExitCode::SUCCESS;
    }
    let outcome = run(&args);
    // The first-run entry's socket directory (board A16).
    let _ = std::fs::remove_dir_all("/tmp/rf-board");
    match outcome {
        Ok(0) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::from(1),
        Err(e) => {
            eprintln!("board_capture: {e}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> Result<usize, String> {
    let options = parse(args)?;
    if options.kit {
        return capture_kit(&options.out);
    }
    let manifest = Manifest::load(&repo_root()).map_err(|e| e.to_string())?;
    let keys: Vec<String> = if options.boards.iter().any(|b| b == "priority") {
        manifest.priority_keys().into_iter().map(str::to_owned).collect()
    } else {
        options.boards.clone()
    };
    std::fs::create_dir_all(&options.out).map_err(|e| format!("{}: {e}", options.out.display()))?;
    let log_path = options.out.join("captures.json");
    // A run over a few boards adds to the log of earlier batches.
    let mut log: CaptureLog = if log_path.is_file() {
        ramflux_desktop_design::read_json(&log_path).map_err(|e| e.to_string())?
    } else {
        CaptureLog::default()
    };
    log.environment = environment();
    let mut failed = 0;
    for key in &keys {
        let board = manifest.board(key, 1).ok_or(format!("board {key} is not in the manifest"))?;
        let themes = options.themes.clone().unwrap_or_else(|| {
            vec![if board.theme == "dark" { "dark".to_owned() } else { "light".to_owned() }]
        });
        let locales = options.locales.clone().unwrap_or_else(|| vec![board.language.clone()]);
        for size in &options.sizes {
            let (width, height) = size.resolve(board.frame_width, board.frame_height);
            for theme in &themes {
                for locale in &locales {
                    let name = CaptureName {
                        key: key.clone(),
                        locale: locale.clone(),
                        theme: theme.clone(),
                        width,
                        height,
                    };
                    let record = capture_one(&options.out, &name);
                    println!("{:8} {} {}", record.outcome, name.stem(), record.note);
                    if record.outcome == "failed" {
                        failed += 1;
                    }
                    log.records.retain(|r| r.name != record.name);
                    log.records.push(record);
                }
            }
        }
    }
    log.records.sort_by_key(|a| a.name.stem());
    ramflux_desktop_design::write_json(&log_path, &log).map_err(|e| e.to_string())?;
    println!("log {}", log_path.display());
    Ok(failed)
}

fn environment() -> Vec<String> {
    vec![
        format!("TZ={}", std::env::var("TZ").unwrap_or_else(|_| "(unset: day labels follow the machine)".into())),
        "renderer: GPUI headless renderer of the test platform, scale factor 2".into(),
        "fonts: bundled Hanken Grotesk and Noto Sans SC (assets::load_fonts); system fallback for other scripts".into(),
        "clock: mock scenario clock or test dispatcher clock (fixture epoch 2026-10-01T00:00Z), never the wall clock".into(),
        format!("os: {} {}", std::env::consts::OS, std::env::consts::ARCH),
    ]
}

fn skipped(name: &CaptureName, note: impl Into<String>, outcome: &str) -> CaptureRecord {
    CaptureRecord {
        name: name.clone(),
        scale: 0,
        outcome: outcome.to_owned(),
        note: note.into(),
        missing_landmarks: Vec::new(),
    }
}

fn capture_one(out: &Path, name: &CaptureName) -> CaptureRecord {
    let Some(plan) = recipes::plan(&name.key) else {
        return skipped(
            name,
            "no recipe: the board has no mapped screen in the capture program",
            "skipped",
        );
    };
    let recipe = match plan {
        Plan::Skip(reason) => return skipped(name, reason, "skipped"),
        Plan::Capture(recipe) => recipe,
    };
    let Some(locale) = shipped_locale(&name.locale) else {
        return skipped(name, format!("locale {} is not shipped yet", name.locale), "skipped");
    };
    let path = out.join(name.file_name());
    let theme = name.theme.clone();
    let (width, height) = (name.width, name.height);
    // A recipe that panics inside the UI fails this capture only.
    let attempt =
        std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<CaptureRecord, String> {
            let mut run = Run::open(recipe.start, width, height, &theme, locale)?;
            (recipe.steps)(&mut run)?;
            if !recipe.keep {
                run.rest()?;
            }
            if run.dev_panel_visible() {
                return Err("the developer panel is open".into());
            }
            let missing: Vec<String> = recipe
                .landmarks
                .iter()
                .filter(|id| !run.has(id))
                .map(|id| (*id).to_owned())
                .collect();
            let image = run.capture()?;
            let scale = image.width() / width.max(1);
            image.save(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(CaptureRecord {
                name: name.clone(),
                scale,
                outcome: "captured".into(),
                note: recipe.note.to_owned(),
                missing_landmarks: missing,
            })
        }));
    match attempt {
        Ok(Ok(record)) => record,
        Ok(Err(error)) => {
            let _ = std::fs::remove_file(&path);
            skipped(name, format!("recipe failed: {error} ({})", recipe.note), "failed")
        }
        Err(panic) => {
            let _ = std::fs::remove_file(&path);
            let message = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_else(|| "panic".into());
            skipped(name, format!("recipe panicked: {message} ({})", recipe.note), "failed")
        }
    }
}

/// Captures every kit component alone (`--kit`).
fn capture_kit(out: &Path) -> Result<usize, String> {
    use ramflux_desktop_design::kit::{KitRecord, capture_name, load};
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let listed = load(&repo_root()).map_err(|e| e.to_string())?;
    let mut records = Vec::new();
    let mut failed = 0;
    for component in kit::components() {
        let Some(entry) = listed.iter().find(|c| c.slug == component.slug) else {
            return Err(format!(
                "kit component {} is not in design/kit/components.toml",
                component.slug
            ));
        };
        let (source, note, render) = match component.how {
            kit::How::App(note, render) => ("app", note, Some(render)),
            kit::How::Replica(note, render) => ("replica", note, Some(render)),
            kit::How::Skip(reason) => ("skipped", reason, None),
        };
        let mut record = KitRecord {
            slug: component.slug.to_owned(),
            source: source.to_owned(),
            note: note.to_owned(),
        };
        if let Some(render) = render {
            let path = out.join(capture_name(component.slug));
            let attempt = std::panic::catch_unwind(AssertUnwindSafe(|| {
                kit::capture(&entry.board, entry.rect.w, entry.rect.h, render)
            }));
            match attempt {
                Ok(Ok(image)) => {
                    image.save(&path).map_err(|e| format!("{}: {e}", path.display()))?
                }
                Ok(Err(error)) => {
                    failed += 1;
                    record.source = "skipped".into();
                    record.note = format!("capture failed: {error}");
                }
                Err(_) => {
                    failed += 1;
                    record.source = "skipped".into();
                    record.note = "capture panicked".into();
                }
            }
        }
        println!("{:8} {} {}", record.source, record.slug, record.note);
        records.push(record);
    }
    ramflux_desktop_design::write_json(&out.join("kit.json"), &records)
        .map_err(|e| e.to_string())?;
    Ok(failed)
}
