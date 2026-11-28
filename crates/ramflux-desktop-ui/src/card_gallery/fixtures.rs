// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The card fixtures the gallery opens, and the bundled trust package they
//! are opened with. Both come from the copies `scripts/sync-card-fixtures.sh`
//! takes at the pinned core revision; the gallery refuses any copy that does
//! not match `MANIFEST.json`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ramflux_card::{AcceptancePolicy, HOST_ABI, SignedPackage};
use serde_json::Value;

/// The fixture copies in this checkout.
pub const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../card-fixtures");
/// The core revision the copies were taken at; the workspace pins the same.
pub const CORE_REV: &str = "a48b182dd6cd5a6ff8e69056240b0a019f9513dc";
/// The scenario batches that ship next to the host fixtures.
pub const SCENARIOS: [&str; 5] = ["approval", "chart", "complete_form", "form", "task"];
const MANIFEST: &str = "MANIFEST.json";
const TRUST_FORMAT: &str = "ramflux.sdk.card.trust.v1";

/// One fixture the gallery lists and checks.
#[derive(Clone, Debug)]
pub struct Case {
    /// `<kind>/<variant>` in fixture form, for example `button/primary`.
    pub id: String,
    /// The catalog kind (`Scenario` for the scenario batches).
    pub kind: String,
    /// The catalog variant, when the row has one.
    pub variant: Option<String>,
    /// The card envelope the runtime opens.
    pub input: Vec<u8>,
    /// The expected tree (`outcome: tree`) or refusal document.
    pub expected: Value,
    /// Whether the index row expects the runtime to refuse the input.
    pub refused: bool,
}

/// The bundled trust package, decoded from `trust.json`.
pub struct Trust {
    /// The verified signed library.
    pub package: SignedPackage,
    /// Presentation documents by `<locale>_<theme>` key, as bytes.
    pub presentations: BTreeMap<String, Vec<u8>>,
}

/// Reads one file of the copies.
fn read(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|error| format!("{}: {error}", path.display()))
}

/// Parses one JSON file of the copies.
fn json(path: &Path) -> Result<Value, String> {
    serde_json::from_slice(&read(path)?).map_err(|error| format!("{}: {error}", path.display()))
}

/// A relative path of the copies, refusing anything that could leave them.
fn relative(dir: &Path, rel: &str) -> Result<PathBuf, String> {
    let path = Path::new(rel);
    if path.components().any(|part| !matches!(part, Component::Normal(_))) {
        return Err(format!("manifest path escapes the copies: {rel}"));
    }
    Ok(dir.join(path))
}

/// Every regular file under `dir`, relative to it, with `/` separators.
fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.is_dir() {
            walk(root, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push(rel.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"));
        }
    }
    Ok(())
}

/// Checks every copy against `MANIFEST.json`. Refuses drift, a missing file,
/// an unlisted file and a manifest taken at another core revision.
///
/// # Errors
/// Returns the first mismatch, as text.
pub fn verify(dir: &Path) -> Result<usize, String> {
    let manifest = json(&dir.join(MANIFEST))?;
    if manifest["core_rev"].as_str() != Some(CORE_REV) {
        return Err(format!("MANIFEST.json is not at core revision {CORE_REV}"));
    }
    let rows = manifest["files"].as_array().ok_or("MANIFEST.json has no files")?;
    let mut listed = BTreeSet::new();
    for row in rows {
        let rel = row["path"].as_str().ok_or("MANIFEST.json row without a path")?;
        let expected = row["blake3"].as_str().ok_or("MANIFEST.json row without a hash")?;
        let actual = blake3::hash(&read(&relative(dir, rel)?)?).to_hex();
        if actual.as_str() != expected {
            return Err(format!("drift: {rel} differs from MANIFEST.json"));
        }
        listed.insert(rel.to_owned());
    }
    let mut found = Vec::new();
    walk(dir, dir, &mut found)?;
    for rel in found {
        if rel != MANIFEST && !listed.contains(&rel) {
            return Err(format!("unlisted file in the copies: {rel}"));
        }
    }
    Ok(listed.len())
}

/// The fixture cases: every `index.json` row, then the five scenarios.
///
/// # Errors
/// Returns the first unreadable or malformed file.
pub fn cases(dir: &Path) -> Result<Vec<Case>, String> {
    let index = json(&dir.join("host-fixtures/index.json"))?;
    let rows = index["cases"].as_array().ok_or("index.json has no cases")?;
    let mut out = Vec::with_capacity(rows.len() + SCENARIOS.len());
    for row in rows {
        let fixture = row["fixture"].as_str().ok_or("index row without a fixture")?;
        let stem = fixture.strip_suffix(".input.json").ok_or("fixture is not an input")?;
        let expected_path = row["expected"].as_str().ok_or("index row without expected")?;
        let refused = match row["outcome"].as_str() {
            Some("tree") => false,
            Some("refused") => true,
            _ => return Err(format!("unknown outcome for {fixture}")),
        };
        out.push(Case {
            id: stem.to_owned(),
            kind: row["kind"].as_str().unwrap_or_default().to_owned(),
            variant: row["variant"].as_str().map(str::to_owned),
            input: read(&relative(&dir.join("host-fixtures"), fixture)?)?,
            expected: json(&relative(&dir.join("host-fixtures"), expected_path)?)?,
            refused,
        });
    }
    for name in SCENARIOS {
        let input = relative(dir, &format!("fixtures/{name}.json"))?;
        let tree = relative(dir, &format!("fixtures/{name}.tree.json"))?;
        out.push(Case {
            id: format!("scenario/{name}"),
            kind: "Scenario".to_owned(),
            variant: Some(name.to_owned()),
            input: read(&input)?,
            expected: json(&tree)?,
            refused: false,
        });
    }
    Ok(out)
}

/// Decodes a base64url field of the trust package (padding optional).
fn decode(value: &Value, field: &str) -> Result<Vec<u8>, String> {
    let text = value[field].as_str().ok_or_else(|| format!("trust.json has no {field}"))?;
    URL_SAFE_NO_PAD
        .decode(text.trim_end_matches('='))
        .map_err(|_| format!("trust.json {field} is not base64url"))
}

/// Opens the bundled trust package: the verified library, its presentations.
///
/// # Errors
/// Refuses another format, another host ABI or a package that fails
/// verification against the pinned key and policy.
pub fn trust(dir: &Path) -> Result<Trust, String> {
    let value = json(&dir.join("trust.json"))?;
    if value["format"].as_str() != Some(TRUST_FORMAT) {
        return Err("trust.json has another format".to_owned());
    }
    if value["card_abi_major"].as_u64() != Some(u64::from(HOST_ABI)) {
        return Err(format!("trust.json is not card ABI {HOST_ABI}"));
    }
    let bytecode = decode(&value, "bytecode_base64url")?;
    let signature: [u8; 64] =
        decode(&value, "signature_base64url")?.try_into().map_err(|_| "signature length")?;
    let key: [u8; 32] =
        decode(&value, "trusted_key_base64url")?.try_into().map_err(|_| "key length")?;
    let descriptor = value["descriptor"].as_str().ok_or("trust.json has no descriptor")?;
    let policy: AcceptancePolicy = serde_json::from_value(value["policy"].clone())
        .map_err(|error| format!("trust.json policy: {error}"))?;
    let package =
        SignedPackage::verify(&bytecode, descriptor.as_bytes(), &signature, &key, &policy)
            .map_err(|error| error.to_string())?;
    let mut presentations = BTreeMap::new();
    let rows = value["presentations"].as_object().ok_or("trust.json has no presentations")?;
    for (name, presentation) in rows {
        let bytes = serde_json::to_vec(presentation).map_err(|error| error.to_string())?;
        presentations.insert(name.clone(), bytes);
    }
    Ok(Trust { package, presentations })
}
