// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Right-to-left guards: views lay rows out with the direction-aware
//! primitives of `ramflux_desktop_navigation::direction` and never name a
//! physical edge (left, right) or a fixed-direction chevron, so the whole
//! interface mirrors in a right-to-left locale. Identifiers use
//! `h_flex_ltr`/`text_ltr`, which are fixed on purpose.

use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("test: dir reads").flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The production part of a source file (before its unit tests).
fn production(text: &str) -> &str {
    text.find("#[cfg(test)]").map_or(text, |end| &text[..end])
}

/// Style methods that name a physical edge: a following `(`, or `_` and a
/// size (`pl_2`, `left_0`, `border_r_1`), not a longer name (`left_behind`).
const EDGES: [&str; 10] = [
    ".pl",
    ".pr",
    ".ml",
    ".mr",
    ".left",
    ".right",
    ".border_l",
    ".border_r",
    ".rounded_l",
    ".rounded_r",
];

/// Calls that fix a direction.
const FIXED: [&str; 9] = [
    ".justify_start()",
    ".justify_end()",
    ".text_left(",
    ".text_right(",
    ".flex_row()",
    "IconName::ChevronLeft",
    "IconName::ChevronRight",
    "IconName::ArrowLeft",
    "IconName::ArrowRight",
];

fn names_an_edge(rest: &str) -> bool {
    let mut chars = rest.chars();
    match chars.next() {
        Some('(') => true,
        Some('_') => chars.next().is_some_and(|c| c.is_ascii_digit() || "afpn".contains(c)),
        _ => false,
    }
}

#[test]
fn views_use_direction_aware_layout() {
    // The scanner itself.
    assert!(names_an_edge("_0()") && names_an_edge("(px(1.))") && names_an_edge("_auto()"));
    assert!(!names_an_edge("_behind(cx)") && !names_an_edge("ing"));
    let mut files = Vec::new();
    rust_files(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    let mut found = Vec::new();
    for file in files {
        let source = fs::read_to_string(&file).expect("test: source reads");
        let code = production(&source);
        let line = |at: usize| code[..at].lines().count();
        for edge in EDGES {
            for (at, _) in code.match_indices(edge) {
                if names_an_edge(&code[at + edge.len()..]) {
                    found.push(format!("{}:{}: {edge}", file.display(), line(at)));
                }
            }
        }
        for fixed in FIXED {
            for (at, _) in code.match_indices(fixed) {
                found.push(format!("{}:{}: {fixed}", file.display(), line(at)));
            }
        }
        let kit_row = code.contains("gpui_kit::component::h_flex")
            || code.lines().any(|l| {
                l.trim_start().starts_with("use gpui_kit::component::{")
                    && l.split(['{', '}', ',']).any(|item| item.trim() == "h_flex")
            });
        if kit_row {
            found.push(format!("{}: uses Kit's h_flex (use direction::h_flex)", file.display()));
        }
    }
    assert!(
        found.is_empty(),
        "physical edges or fixed directions (use ramflux_desktop_navigation::direction):\n{}",
        found.join("\n")
    );
}
