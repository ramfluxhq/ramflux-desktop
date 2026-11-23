// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! `rf-design`: board references, capture comparison, deviation review and
//! the token check. See `docs/design-instrument.md`.

#![cfg_attr(
    not(test),
    deny(clippy::disallowed_types, clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ramflux_desktop_design::compare::Options;
use ramflux_desktop_design::deviations::DEVIATIONS_DIR;
use ramflux_desktop_design::manifest::{self, MANIFEST_PATH, Manifest, Scope};
use ramflux_desktop_design::report::{self, RunOptions};
use ramflux_desktop_design::{Error, Result, tokens};

const USAGE: &str = "\
rf-design: design instruments of the desktop client

USAGE:
  rf-design refs import --from <boards-desktop dir>
      Rewrite design/boards/manifest.json from an exported reference set.
  rf-design refs verify [--refs <dir>] [--priority]
      Check the local reference set (RAMFLUX_DESIGN_REFS) against the manifest.
  rf-design compare --captures <dir> --out <dir> [--refs <dir>] [--boards K1,K2]
                    [--deviations <dir>] [--no-2x] [--threshold N]
                    [--check] [--review] [--record] [--classify]
      Compare captures with the references (default: the priority set).
      --check   exit 1 on NEW deviations only (CI)
      --review  also write review.md listing what needs a decision
      --record  write NEW regions into the per-board files as unreviewed
      --classify  attribute unreviewed entries to classes by measurement
                (accepts only window chrome, glyph rasterisation and stroke alignment)
  rf-design kit --captures <dir> --out <dir> [--refs <dir>]
      Compare the kit component captures with the component boards.
  rf-design tokens [--out <dir>] [--check] [--worklist <file>]
      Compare the code's design tokens with design/pencil/variables.json.

Common: --root <repository> (default: the nearest ancestor with design/boards).
";

struct Args {
    words: Vec<String>,
}

impl Args {
    fn flag(&mut self, name: &str) -> bool {
        if let Some(i) = self.words.iter().position(|w| w == name) {
            self.words.remove(i);
            true
        } else {
            false
        }
    }

    fn value(&mut self, name: &str) -> Result<Option<String>> {
        let Some(i) = self.words.iter().position(|w| w == name) else { return Ok(None) };
        if i + 1 >= self.words.len() {
            return Err(Error::Invalid(format!("{name} needs a value")));
        }
        let value = self.words.remove(i + 1);
        self.words.remove(i);
        Ok(Some(value))
    }

    fn path(&mut self, name: &str) -> Result<Option<PathBuf>> {
        Ok(self.value(name)?.map(PathBuf::from))
    }

    fn finish(&self) -> Result<()> {
        if self.words.is_empty() {
            Ok(())
        } else {
            Err(Error::Invalid(format!("unexpected arguments: {}", self.words.join(" "))))
        }
    }
}

fn find_root(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(root) = explicit {
        return Ok(root);
    }
    let start = std::env::current_dir().map_err(|e| Error::Invalid(e.to_string()))?;
    let mut dir: Option<&Path> = Some(&start);
    while let Some(d) = dir {
        if d.join("design/boards").is_dir() {
            return Ok(d.to_path_buf());
        }
        dir = d.parent();
    }
    Err(Error::Invalid("no repository root with design/boards found; pass --root".into()))
}

fn main() -> ExitCode {
    let words: Vec<String> = std::env::args().skip(1).collect();
    match run(Args { words }) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("rf-design: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(mut args: Args) -> Result<ExitCode> {
    if args.words.is_empty() || args.flag("--help") || args.flag("-h") {
        print!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    }
    let command = args.words.remove(0);
    let root_arg = args.path("--root")?;
    match command.as_str() {
        "refs" => {
            let sub = if args.words.is_empty() { String::new() } else { args.words.remove(0) };
            match sub.as_str() {
                "import" => {
                    let from = args
                        .path("--from")?
                        .ok_or_else(|| Error::Invalid("refs import needs --from".into()))?;
                    args.finish()?;
                    let root = find_root(root_arg)?;
                    let manifest = Manifest::import(&from)?;
                    ramflux_desktop_design::write_json(&root.join(MANIFEST_PATH), &manifest)?;
                    println!(
                        "wrote {MANIFEST_PATH}: {} references, {} priority boards",
                        manifest.boards.len(),
                        manifest.priority_keys().len()
                    );
                    Ok(ExitCode::SUCCESS)
                }
                "verify" => {
                    let refs = args.path("--refs")?;
                    let scope = if args.flag("--priority") { Scope::Priority } else { Scope::All };
                    args.finish()?;
                    let root = find_root(root_arg)?;
                    let manifest = Manifest::load(&root)?;
                    manifest.validate()?;
                    let refs = manifest::refs_dir(refs.as_deref())?;
                    let (checked, findings) = manifest::verify(&manifest, &refs, scope)?;
                    for finding in &findings {
                        println!("{finding}");
                    }
                    println!(
                        "{checked} references checked in {}: {} problems",
                        refs.display(),
                        findings.len()
                    );
                    Ok(if findings.is_empty() { ExitCode::SUCCESS } else { ExitCode::from(1) })
                }
                _ => Err(Error::Invalid(format!("unknown refs command {sub:?}\n{USAGE}"))),
            }
        }
        "compare" => {
            let captures = args
                .path("--captures")?
                .ok_or_else(|| Error::Invalid("compare needs --captures".into()))?;
            let out =
                args.path("--out")?.ok_or_else(|| Error::Invalid("compare needs --out".into()))?;
            let refs = args.path("--refs")?;
            let deviations = args.path("--deviations")?;
            let boards: Vec<String> = args
                .value("--boards")?
                .map(|b| {
                    b.split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let mut compare = Options::default();
            if let Some(t) = args.value("--threshold")? {
                compare.threshold =
                    t.parse().map_err(|_| Error::Invalid(format!("bad --threshold {t}")))?;
            }
            let prefer_2x = !args.flag("--no-2x");
            let check = args.flag("--check");
            let review = args.flag("--review");
            let record = args.flag("--record");
            let classify = args.flag("--classify");
            args.finish()?;
            let root = find_root(root_arg)?;
            let manifest = Manifest::load(&root)?;
            let refs = manifest::refs_dir(refs.as_deref())?;
            let deviations = deviations.unwrap_or_else(|| root.join(DEVIATIONS_DIR));
            let options = RunOptions {
                refs,
                captures,
                out: out.clone(),
                deviations: deviations.clone(),
                boards,
                prefer_2x,
                record,
                classify,
                compare,
            };
            let result = report::run(&manifest, &options)?;
            println!(
                "{} boards: {} pass, {} known, {} NEW, {} no capture (known), {} no capture (NEW), {} no reference; report {}",
                result.boards.len(),
                result.count(report::Status::Pass),
                result.count(report::Status::Known),
                result.count(report::Status::New),
                result.count(report::Status::NoCaptureKnown),
                result.count(report::Status::NoCaptureNew),
                result.count(report::Status::NoReference),
                out.join("report.md").display()
            );
            if review {
                let text = report::review(&result, &deviations)?;
                ramflux_desktop_design::write_text(&out.join("review.md"), &text)?;
                print!("{text}");
            }
            Ok(if check && result.fails_check() { ExitCode::from(1) } else { ExitCode::SUCCESS })
        }
        "kit" => {
            let captures = args
                .path("--captures")?
                .ok_or_else(|| Error::Invalid("kit needs --captures".into()))?;
            let out =
                args.path("--out")?.ok_or_else(|| Error::Invalid("kit needs --out".into()))?;
            let refs = args.path("--refs")?;
            args.finish()?;
            let root = find_root(root_arg)?;
            let manifest = Manifest::load(&root)?;
            let refs = manifest::refs_dir(refs.as_deref())?;
            let results =
                ramflux_desktop_design::kit::run(&root, &manifest, &refs, &captures, &out)?;
            let compared = results.iter().filter(|r| r.three_up.is_some()).count();
            println!(
                "{} components, {compared} compared; report {}",
                results.len(),
                out.join("kit.md").display()
            );
            Ok(ExitCode::SUCCESS)
        }
        "tokens" => {
            let out = args.path("--out")?;
            let check = args.flag("--check");
            let worklist = args.path("--worklist")?;
            args.finish()?;
            let root = find_root(root_arg)?;
            let report = tokens::check(&root)?;
            if let Some(path) = worklist {
                ramflux_desktop_design::write_text(&path, &tokens::worklist(&root)?)?;
                println!("wrote {}", path.display());
            }
            let text = tokens::markdown(&report);
            if let Some(out) = out {
                ramflux_desktop_design::write_json(&out.join("tokens.json"), &report)?;
                ramflux_desktop_design::write_text(&out.join("tokens.md"), &text)?;
            }
            println!(
                "{} variables, {} token x mode comparisons, {} equal, {} failing findings, {} findings in all",
                report.variables,
                report.compared,
                report.equal,
                report.mismatches(),
                report.findings.len()
            );
            Ok(if check && report.mismatches() > 0 { ExitCode::from(1) } else { ExitCode::SUCCESS })
        }
        other => Err(Error::Invalid(format!("unknown command {other:?}\n{USAGE}"))),
    }
}
