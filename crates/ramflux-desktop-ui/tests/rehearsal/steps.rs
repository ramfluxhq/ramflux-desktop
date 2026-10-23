// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The step table: each step runs once, timed, with the notes it made; a
//! step whose prerequisite did not pass is skipped, never run on a broken
//! state. The table renders as Markdown for the report.

use std::time::{Duration, Instant};

/// How a step ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Every assertion held.
    Pass,
    /// An assertion failed, with why.
    Fail(String),
    /// Not run: a prerequisite did not pass.
    Skipped(String),
}

/// One row of the table.
#[derive(Debug, Clone)]
pub struct Row {
    /// Step number.
    pub number: u8,
    /// What the step does.
    pub title: String,
    /// How it ended.
    pub outcome: Outcome,
    /// How long it took.
    pub duration: Duration,
    /// What it observed (states, fallbacks, screenshots).
    pub notes: Vec<String>,
}

/// Notes a running step makes.
#[derive(Debug, Default)]
pub struct Notes(Vec<String>);

impl Notes {
    /// Adds a note.
    pub fn add(&mut self, note: impl Into<String>) {
        self.0.push(note.into());
    }
}

/// The steps run so far.
#[derive(Debug, Default)]
pub struct Steps {
    rows: Vec<Row>,
}

impl Steps {
    /// Runs step `number` unless one of `needs` did not pass.
    pub fn run(
        &mut self,
        number: u8,
        title: &str,
        needs: &[u8],
        step: impl FnOnce(&mut Notes) -> Result<(), String>,
    ) -> bool {
        let missing: Vec<String> =
            needs.iter().filter(|n| !self.passed(**n)).map(|n| format!("step {n}")).collect();
        let mut notes = Notes::default();
        let start = Instant::now();
        let outcome = if missing.is_empty() {
            match step(&mut notes) {
                Ok(()) => Outcome::Pass,
                Err(why) => Outcome::Fail(why),
            }
        } else {
            Outcome::Skipped(format!("needs {}", missing.join(", ")))
        };
        let passed = outcome == Outcome::Pass;
        println!("step {number} {title}: {outcome:?}");
        self.rows.push(Row {
            number,
            title: title.to_owned(),
            outcome,
            duration: start.elapsed(),
            notes: notes.0,
        });
        passed
    }

    /// Whether step `number` ran and passed.
    pub fn passed(&self, number: u8) -> bool {
        self.rows.iter().any(|r| r.number == number && r.outcome == Outcome::Pass)
    }

    /// Whether every step passed.
    pub fn all_passed(&self) -> bool {
        !self.rows.is_empty() && self.rows.iter().all(|r| r.outcome == Outcome::Pass)
    }

    /// The rows.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The table as Markdown.
    pub fn markdown(&self) -> String {
        let mut out =
            String::from("| # | Step | Result | Duration | Notes |\n|---|---|---|---|---|\n");
        for row in &self.rows {
            let (result, why) = match &row.outcome {
                Outcome::Pass => ("pass", None),
                Outcome::Fail(why) => ("FAIL", Some(why.as_str())),
                Outcome::Skipped(why) => ("skipped", Some(why.as_str())),
            };
            let notes: Vec<&str> =
                why.into_iter().chain(row.notes.iter().map(String::as_str)).collect();
            out.push_str(&format!(
                "| {} | {} | {} | {:.1} s | {} |\n",
                row.number,
                cell(&row.title),
                result,
                row.duration.as_secs_f64(),
                cell(&notes.join("; "))
            ));
        }
        out
    }
}

/// Text safe inside one Markdown table cell.
fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}
