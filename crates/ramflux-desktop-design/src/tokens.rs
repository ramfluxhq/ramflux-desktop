// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Design tokens of the code compared by value with the design variables.
//!
//! Design side: `design/pencil/variables.json`, the 100 variables of the
//! design file (one theme axis `mode`, light and dark).
//!
//! Code side, as the application resolves it:
//! 1. the palette `RFTheme::light()`/`dark()` in
//!    `crates/ramflux-desktop-navigation/src/generated/rf_tokens.rs`,
//! 2. overridden per mode by `design/tokens/rf-tokens.json` (what
//!    `theme::palette_from_tokens` does at start-up),
//! 3. the GPUI Kit themes `design/generated/{light,dark}.json`, whose values
//!    must each be a design colour of the same mode,
//! 4. utility calls in the UI sources that carry their own values (Kit type
//!    and spacing steps in rem, literal colours), resolved at the Kit rem
//!    size (`font.size` of the Kit theme).
//!
//! Values are compared, not file hashes: colours as 8-bit RGBA, numbers
//! exactly, strings exactly.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

pub const VARIABLES_PATH: &str = "design/pencil/variables.json";
pub const PALETTE_PATH: &str = "crates/ramflux-desktop-navigation/src/generated/rf_tokens.rs";
pub const TOKENS_PATH: &str = "design/tokens/rf-tokens.json";
pub const KIT_THEMES: [(&str, &str); 2] =
    [("light", "design/generated/light.json"), ("dark", "design/generated/dark.json")];
pub const UI_SOURCES: [&str; 2] =
    ["crates/ramflux-desktop-ui/src", "crates/ramflux-desktop-navigation/src"];

/// A token value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Color(String),
    Number(f64),
}

impl Value {
    fn show(&self) -> String {
        match self {
            Self::Color(s) => s.clone(),
            Self::Number(n) => format_number(*n),
        }
    }
}

fn format_number(n: f64) -> String {
    if n.fract() == 0.0 { format!("{n:.0}") } else { format!("{n:.3}") }
}

/// One design variable.
#[derive(Debug, Clone, Deserialize)]
pub struct Variable {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub themed: bool,
    pub light: serde_json::Value,
    pub dark: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
struct Variables {
    design_file_sha256: String,
    variables: Vec<Variable>,
}

/// Normalises `#RGB[A]`-style hex to `#RRGGBBAA` uppercase.
pub fn normalise_hex(text: &str) -> Option<String> {
    let hex = text.trim().trim_start_matches('#');
    let full = match hex.len() {
        6 => format!("{hex}FF"),
        8 => hex.to_owned(),
        _ => return None,
    };
    if !full.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("#{}", full.to_uppercase()))
}

fn json_value(kind: &str, value: &serde_json::Value) -> Option<ValueOrText> {
    match kind {
        "color" => {
            value.as_str().and_then(normalise_hex).map(|c| ValueOrText::Value(Value::Color(c)))
        }
        "number" => value.as_f64().map(|n| ValueOrText::Value(Value::Number(n))),
        _ => value.as_str().map(|s| ValueOrText::Text(s.to_owned())),
    }
}

#[derive(Debug, Clone, PartialEq)]
enum ValueOrText {
    Value(Value),
    Text(String),
}

impl ValueOrText {
    fn show(&self) -> String {
        match self {
            Self::Value(v) => v.show(),
            Self::Text(t) => format!("{t:?}"),
        }
    }
}

/// HSLA (0..1) to `#RRGGBBAA`.
pub fn hsla_to_hex(h: f64, s: f64, l: f64, a: f64) -> String {
    let hue = |p: f64, q: f64, mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    let (r, g, b) = if s == 0.0 {
        (l, l, l)
    } else {
        let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
        let p = 2.0 * l - q;
        (hue(p, q, h + 1.0 / 3.0), hue(p, q, h), hue(p, q, h - 1.0 / 3.0))
    };
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02X}{:02X}{:02X}{:02X}", byte(r), byte(g), byte(b), byte(a))
}

/// The palette fields of `RFTheme::light()` and `dark()`.
pub fn parse_palette(source: &str) -> BTreeMap<String, BTreeMap<String, ValueOrTextPub>> {
    let mut modes: BTreeMap<String, BTreeMap<String, ValueOrTextPub>> = BTreeMap::new();
    let mut mode: Option<&str> = None;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("pub fn light()") {
            mode = Some("light");
            continue;
        }
        if trimmed.starts_with("pub fn dark()") {
            mode = Some("dark");
            continue;
        }
        let Some(current) = mode else { continue };
        let Some((name, rest)) = trimmed.split_once(": ") else { continue };
        if name.contains(' ') || name.starts_with("pub") {
            continue;
        }
        let rest = rest.trim_end_matches(',');
        let value = if let Some(args) = rest.strip_prefix("hsla(").and_then(|r| r.strip_suffix(')'))
        {
            let n: Vec<f64> = args.split(',').filter_map(|v| v.trim().parse().ok()).collect();
            if n.len() != 4 {
                continue;
            }
            ValueOrTextPub::Value(Value::Color(hsla_to_hex(n[0], n[1], n[2], n[3])))
        } else if let Some(text) = rest.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
            ValueOrTextPub::Text(text.to_owned())
        } else if let Ok(n) = rest.parse::<f64>() {
            ValueOrTextPub::Value(Value::Number(n))
        } else {
            continue;
        };
        modes.entry(current.to_owned()).or_default().insert(name.to_owned(), value);
    }
    modes
}

/// A palette value (public mirror of the internal value type).
#[derive(Debug, Clone, PartialEq)]
pub enum ValueOrTextPub {
    Value(Value),
    Text(String),
}

impl From<ValueOrTextPub> for ValueOrText {
    fn from(v: ValueOrTextPub) -> Self {
        match v {
            ValueOrTextPub::Value(v) => Self::Value(v),
            ValueOrTextPub::Text(t) => Self::Text(t),
        }
    }
}

/// Applies the `rf-tokens.json` set of `mode` over a palette.
fn apply_token_set(palette: &mut BTreeMap<String, ValueOrText>, set: &serde_json::Value) {
    let Some(map) = set.as_object() else { return };
    for (name, token) in map {
        let field = name.replace('-', "_");
        let kind = token.get("$type").and_then(|t| t.as_str()).unwrap_or_default();
        let value = &token["$value"];
        let parsed = match kind {
            "color" => {
                let c: Vec<f64> = value["components"]
                    .as_array()
                    .map(|a| a.iter().filter_map(serde_json::Value::as_f64).collect())
                    .unwrap_or_default();
                let alpha = value["alpha"].as_f64().unwrap_or(1.0);
                if c.len() != 3 {
                    continue;
                }
                let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                ValueOrText::Value(Value::Color(format!(
                    "#{:02X}{:02X}{:02X}{:02X}",
                    byte(c[0]),
                    byte(c[1]),
                    byte(c[2]),
                    byte(alpha)
                )))
            }
            "dimension" => match value["value"].as_f64() {
                Some(n) => ValueOrText::Value(Value::Number(n)),
                None => continue,
            },
            "fontFamily" => match value.as_str() {
                Some(s) => ValueOrText::Text(s.to_owned()),
                None => continue,
            },
            _ => continue,
        };
        palette.insert(field, parsed);
    }
}

/// One finding of the token check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// Failing: `mismatch`, `missing-in-code`, `kit-value-not-in-design`,
    /// `type-size-not-in-design`, `literal-colour-not-in-design`.
    /// Informational: `legacy-not-in-code`, `code-only-token`,
    /// `spacing-utility`, `literal-colour`, `rem-divisor`.
    pub kind: String,
    pub token: String,
    pub mode: String,
    pub design: String,
    pub code: String,
    pub source: String,
    /// Uses in the UI sources (utility findings).
    pub uses: usize,
}

impl Finding {
    /// Fails the check (value disagreements and missing design tokens).
    pub fn is_mismatch(&self) -> bool {
        matches!(
            self.kind.as_str(),
            "mismatch"
                | "missing-in-code"
                | "kit-value-not-in-design"
                | "type-size-not-in-design"
                | "literal-colour-not-in-design"
        )
    }
}

/// The token check result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenReport {
    pub design_file_sha256: String,
    pub variables: usize,
    pub compared: usize,
    pub equal: usize,
    pub rem: f64,
    pub findings: Vec<Finding>,
}

impl TokenReport {
    pub fn mismatches(&self) -> usize {
        self.findings.iter().filter(|f| f.is_mismatch()).count()
    }

    pub fn count(&self, kind: &str) -> usize {
        self.findings.iter().filter(|f| f.kind == kind).count()
    }
}

/// Kit utility steps (rem multiples) of GPUI's styled helpers.
const TEXT_STEPS: [(&str, f64); 7] = [
    ("text_xs", 0.75),
    ("text_sm", 0.875),
    ("text_base", 1.0),
    ("text_lg", 1.125),
    ("text_xl", 1.25),
    ("text_2xl", 1.5),
    ("text_3xl", 1.875),
];
const SPACING_PREFIXES: [&str; 18] = [
    "gap_x", "gap_y", "gap", "px", "py", "pt", "pb", "pl", "pr", "p", "mx", "my", "mt", "mb", "ml",
    "mr", "m", "size",
];

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<()> {
    let entries = std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// The value of a spacing step like `2`, `0p5`, `12` in rem (Tailwind
/// scale: one step is 0.25 rem).
fn spacing_step(step: &str) -> Option<f64> {
    let n: f64 = step.replace('p', ".").parse().ok()?;
    Some(n * 0.25)
}

/// Counts `.<name>()` utility calls in `text`, keyed by the call name.
fn utility_calls(text: &str, counts: &mut BTreeMap<String, usize>) {
    for (index, _) in text.match_indices('.') {
        let rest = &text[index + 1..];
        let Some(end) = rest.find("()") else { continue };
        let name = &rest[..end];
        if name.is_empty()
            || name.len() > 12
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            continue;
        }
        let known = TEXT_STEPS.iter().any(|(t, _)| *t == name)
            || SPACING_PREFIXES.iter().any(|p| {
                name.strip_prefix(p)
                    .and_then(|s| s.strip_prefix('_'))
                    .is_some_and(|s| spacing_step(s).is_some())
            });
        if known {
            *counts.entry(name.to_owned()).or_default() += 1;
        }
    }
}

/// Runs the token check over the repository at `root`.
pub fn check(root: &Path) -> Result<TokenReport> {
    let variables: Variables = crate::read_json(&root.join(VARIABLES_PATH))?;
    let palette_source = crate::read_text(&root.join(PALETTE_PATH))?;
    let token_sets: serde_json::Value = crate::read_json(&root.join(TOKENS_PATH))?;
    let mut palettes: BTreeMap<String, BTreeMap<String, ValueOrText>> =
        parse_palette(&palette_source)
            .into_iter()
            .map(|(mode, map)| (mode, map.into_iter().map(|(k, v)| (k, v.into())).collect()))
            .collect();
    for mode in ["light", "dark"] {
        let palette = palettes.entry(mode.to_owned()).or_default();
        apply_token_set(palette, &token_sets[mode]);
    }
    let mut findings = Vec::new();
    let mut compared = 0;
    let mut equal = 0;
    let mut design_colours: BTreeMap<&str, BTreeSet<String>> = BTreeMap::new();
    let mut design_fields = BTreeSet::new();
    for variable in &variables.variables {
        for mode in ["light", "dark"] {
            let raw = if mode == "light" { &variable.light } else { &variable.dark };
            let Some(design) = json_value(&variable.kind, raw) else {
                return Err(Error::Invalid(format!(
                    "variable {} has a bad {mode} value",
                    variable.name
                )));
            };
            if let ValueOrText::Value(Value::Color(c)) = &design {
                design_colours.entry(mode).or_default().insert(c.clone());
            }
            let Some(field) = variable.name.strip_prefix("rf-").map(|n| n.replace('-', "_")) else {
                if mode == "light" {
                    findings.push(Finding {
                        kind: "legacy-not-in-code".into(),
                        token: variable.name.clone(),
                        mode: "all".into(),
                        design: design.show(),
                        code: String::new(),
                        source: String::new(),
                        uses: 0,
                    });
                }
                continue;
            };
            design_fields.insert(field.clone());
            compared += 1;
            match palettes.get(mode).and_then(|p| p.get(&field)) {
                None => findings.push(Finding {
                    kind: "missing-in-code".into(),
                    token: variable.name.clone(),
                    mode: mode.into(),
                    design: design.show(),
                    code: String::new(),
                    source: PALETTE_PATH.into(),
                    uses: 0,
                }),
                Some(code) if *code == design => equal += 1,
                Some(code) => findings.push(Finding {
                    kind: "mismatch".into(),
                    token: variable.name.clone(),
                    mode: mode.into(),
                    design: design.show(),
                    code: code.show(),
                    source: format!("{PALETTE_PATH} + {TOKENS_PATH}"),
                    uses: 0,
                }),
            }
        }
    }
    // Palette fields the design does not define.
    for (mode, palette) in &palettes {
        for (field, value) in palette {
            if !design_fields.contains(field) {
                findings.push(Finding {
                    kind: "code-only-token".into(),
                    token: field.clone(),
                    mode: mode.clone(),
                    design: String::new(),
                    code: value.show(),
                    source: PALETTE_PATH.into(),
                    uses: 0,
                });
            }
        }
    }
    if let Some(geometry) = token_sets["geometry"].as_object() {
        for (name, token) in geometry {
            findings.push(Finding {
                kind: "code-only-token".into(),
                token: format!("geometry.{name}"),
                mode: "all".into(),
                design: String::new(),
                code: token["$value"]["value"].as_f64().map(format_number).unwrap_or_default(),
                source: TOKENS_PATH.into(),
                uses: 0,
            });
        }
    }
    // Kit theme values.
    let mut rem = 15.0;
    let type_sizes: BTreeSet<String> = variables
        .variables
        .iter()
        .filter(|v| v.name.starts_with("rf-size-"))
        .filter_map(|v| v.light.as_f64().map(format_number))
        .collect();
    let radii: BTreeSet<String> = variables
        .variables
        .iter()
        .filter(|v| v.name.starts_with("rf-radius-"))
        .filter_map(|v| v.light.as_f64().map(format_number))
        .collect();
    for (mode, path) in KIT_THEMES {
        let theme: serde_json::Value = crate::read_json(&root.join(path))?;
        if let Some(size) = theme["font.size"].as_f64() {
            rem = size;
            if !type_sizes.contains(&format_number(size)) {
                findings.push(kit_finding("font.size", mode, &format_number(size), path));
            }
        }
        for key in ["radius", "radius.lg"] {
            if let Some(r) = theme[key].as_f64()
                && !radii.contains(&format_number(r))
            {
                findings.push(kit_finding(key, mode, &format_number(r), path));
            }
        }
        let colours = design_colours.get(mode).cloned().unwrap_or_default();
        if let Some(map) = theme["colors"].as_object() {
            for (name, value) in map {
                let Some(hex) = value.as_str().and_then(normalise_hex) else { continue };
                if !colours.contains(&hex) {
                    findings.push(kit_finding(&format!("colors.{name}"), mode, &hex, path));
                }
            }
        }
    }
    // Utility calls and literal colours in the UI sources.
    let mut files = Vec::new();
    for dir in UI_SOURCES {
        rust_files(&root.join(dir), &mut files)?;
    }
    let mut counts = BTreeMap::new();
    let mut divisors = 0;
    let mut literals: BTreeMap<String, usize> = BTreeMap::new();
    for file in &files {
        if file.components().any(|c| c.as_os_str() == "generated") {
            continue;
        }
        let text = crate::read_text(file)?;
        // Production code only: comments and the inline test module (by
        // convention the last item of a file) are left out.
        let code: String = text
            .lines()
            .take_while(|l| l.trim() != "#[cfg(test)]")
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        utility_calls(&code, &mut counts);
        divisors += code.matches("/ 16.0").count();
        for prefix in ["rgb(0x", "rgba(0x", "hsla("] {
            for (index, _) in code.match_indices(prefix) {
                if index > 0 && code.as_bytes()[index - 1].is_ascii_alphanumeric() {
                    continue;
                }
                let rest = &code[index..];
                if let Some(end) = rest.find(')') {
                    *literals.entry(rest[..=end].to_owned()).or_default() += 1;
                }
            }
        }
    }
    for (name, uses) in counts {
        let (value, allowed, what) =
            if let Some((_, r)) = TEXT_STEPS.iter().find(|(t, _)| *t == name) {
                (r * rem, &type_sizes, "type size")
            } else {
                let step = name.rsplit('_').next().and_then(spacing_step).unwrap_or(0.0);
                (step * rem, &BTreeSet::new(), "spacing")
            };
        let shown = format_number(value);
        if allowed.contains(&shown) || value == 0.0 {
            continue;
        }
        let kind = if what == "type size" { "type-size-not-in-design" } else { "spacing-utility" };
        findings.push(Finding {
            kind: kind.into(),
            token: format!(".{name}()"),
            mode: "all".into(),
            design: if what == "type size" {
                format!("type sizes {}", type_sizes.iter().cloned().collect::<Vec<_>>().join("/"))
            } else {
                "no spacing variable (rf-gap 16 only)".into()
            },
            code: format!("{shown} px {what} at rem {}", format_number(rem)),
            source: "crates/ramflux-desktop-ui/src".into(),
            uses,
        });
    }
    let all_colours: BTreeSet<String> = design_colours.values().flatten().cloned().collect();
    for (literal, uses) in literals {
        let hex = literal_hex(&literal);
        let known = hex.as_ref().is_some_and(|h| all_colours.contains(h));
        findings.push(Finding {
            kind: if known {
                "literal-colour".into()
            } else {
                "literal-colour-not-in-design".into()
            },
            token: literal,
            mode: "all".into(),
            design: String::new(),
            code: hex.unwrap_or_default(),
            source: "crates/ramflux-desktop-ui/src".into(),
            uses,
        });
    }
    if divisors > 0 {
        findings.push(Finding {
            kind: "rem-divisor".into(),
            token: "/ 16.0".into(),
            mode: "all".into(),
            design: format!("rem {}", format_number(rem)),
            code: "design pixels divided by 16 instead of the rem size".into(),
            source: "crates/ramflux-desktop-ui/src".into(),
            uses: divisors,
        });
    }
    Ok(TokenReport {
        design_file_sha256: variables.design_file_sha256,
        variables: variables.variables.len(),
        compared,
        equal,
        rem,
        findings,
    })
}

/// The `#RRGGBBAA` value of an `rgb(0x…)`, `rgba(0x…)` or `hsla(…)` literal.
fn literal_hex(literal: &str) -> Option<String> {
    let inner = literal.split_once('(')?.1.strip_suffix(')')?;
    if let Some(hex) = inner.strip_prefix("0x") {
        let digits: String = hex.chars().filter(char::is_ascii_hexdigit).collect();
        let value = u32::from_str_radix(&digits, 16).ok()?;
        return Some(if literal.starts_with("rgba") {
            format!("#{value:08X}")
        } else {
            format!("#{:06X}FF", value & 0x00FF_FFFF)
        });
    }
    let n: Vec<f64> = inner.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    (n.len() == 4).then(|| hsla_to_hex(n[0], n[1], n[2], n[3]))
}

fn kit_finding(token: &str, mode: &str, value: &str, path: &str) -> Finding {
    Finding {
        kind: "kit-value-not-in-design".into(),
        token: token.to_owned(),
        mode: mode.to_owned(),
        design: String::new(),
        code: value.to_owned(),
        source: path.to_owned(),
        uses: 0,
    }
}

/// The token report as Markdown.
pub fn markdown(report: &TokenReport) -> String {
    let mut out = String::from("# Token check\n\n");
    out.push_str(&format!(
        "Design variables: {} (design file sha256 `{}`). Themed comparisons: {} (token × mode), equal {}. Kit rem size {} px.\n\n",
        report.variables, report.design_file_sha256, report.compared, report.equal, format_number(report.rem)
    ));
    out.push_str("| Kind | Count |\n|---|---|\n");
    for kind in [
        "mismatch",
        "missing-in-code",
        "kit-value-not-in-design",
        "legacy-not-in-code",
        "code-only-token",
        "type-size-not-in-design",
        "literal-colour-not-in-design",
        "spacing-utility",
        "literal-colour",
        "rem-divisor",
    ] {
        out.push_str(&format!("| {kind} | {} |\n", report.count(kind)));
    }
    out.push_str(&format!(
        "\nFailing findings (value mismatch, missing in code, Kit or utility or literal value the design does not define): {}\n\n",
        report.mismatches()
    ));
    out.push_str(
        "| Kind | Token | Mode | Design | Code | Uses | Source |\n|---|---|---|---|---|---|---|\n",
    );
    for f in &report.findings {
        out.push_str(&format!(
            "| {} | `{}` | {} | {} | {} | {} | {} |\n",
            f.kind,
            f.token,
            f.mode,
            f.design,
            f.code,
            if f.uses > 0 { f.uses.to_string() } else { String::new() },
            f.source
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_and_hsla_conversions() {
        assert_eq!(normalise_hex("#fcfbf9"), Some("#FCFBF9FF".into()));
        assert_eq!(normalise_hex("#0000004D"), Some("#0000004D".into()));
        assert_eq!(normalise_hex("nope"), None);
        // rf-bg light #FCFBF9 as the generated palette writes it.
        assert_eq!(hsla_to_hex(0.111111111111, 0.333333333333, 0.982352941176, 1.0), "#FCFBF9FF");
        assert_eq!(hsla_to_hex(0.0, 0.0, 0.0, 0.301960784314), "#0000004D");
    }

    #[test]
    fn palette_parsing_reads_both_modes() {
        let source = "impl RFTheme {\n    pub fn light() -> Self { Self {\n        bg: hsla(0.0, 0.0, 1.0, 1.0),\n        gap: 16.0,\n        font: \"Noto Sans SC\",\n    } }\n    pub fn dark() -> Self { Self {\n        bg: hsla(0.0, 0.0, 0.0, 1.0),\n    } }\n}\n";
        let modes = parse_palette(source);
        assert_eq!(modes["light"]["bg"], ValueOrTextPub::Value(Value::Color("#FFFFFFFF".into())));
        assert_eq!(modes["light"]["gap"], ValueOrTextPub::Value(Value::Number(16.0)));
        assert_eq!(modes["light"]["font"], ValueOrTextPub::Text("Noto Sans SC".into()));
        assert_eq!(modes["dark"]["bg"], ValueOrTextPub::Value(Value::Color("#000000FF".into())));
    }

    #[test]
    fn utility_calls_are_counted_by_name() {
        let mut counts = BTreeMap::new();
        utility_calls(
            "div().gap_2().px_3().text_xs().gap_2().child(x).p_0p5().foo_2()",
            &mut counts,
        );
        assert_eq!(counts.get("gap_2"), Some(&2));
        assert_eq!(counts.get("px_3"), Some(&1));
        assert_eq!(counts.get("text_xs"), Some(&1));
        assert_eq!(counts.get("p_0p5"), Some(&1));
        assert_eq!(counts.get("foo_2"), None);
        assert_eq!(spacing_step("2"), Some(0.5));
        assert_eq!(spacing_step("0p5"), Some(0.125));
        assert_eq!(literal_hex("rgb(0x0022_c55e)"), Some("#22C55EFF".into()));
        assert_eq!(literal_hex("hsla(0., 0., 1., 1.)"), Some("#FFFFFFFF".into()));
    }

    #[test]
    fn the_repository_check_runs() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let report = check(&root).unwrap();
        assert_eq!(report.variables, 100);
        // 88 rf-* variables in two modes.
        assert_eq!(report.compared, 176);
        assert_eq!(report.count("legacy-not-in-code"), 12);
    }
}
