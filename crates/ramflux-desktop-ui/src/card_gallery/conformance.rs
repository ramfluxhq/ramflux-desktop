// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Conformance of the desktop card renderer against the standalone runtime.
//!
//! - A: every index row opens to the expected tree (session rewritten to 1,
//!   the README rule); refused rows refuse.
//! - B: the production renderer draws every expected tree: no placeholder,
//!   every reachable node and every host slot drawn (closed Modal content and
//!   hidden Tabs panels are not drawn by design).
//! - C: for each interactive node, the host event the renderer builds is sent
//!   to the runtime; the returned batch, applied to the held tree, equals the
//!   runtime's tree after the event.
//!
//! Records are one JSON line per fixture and check, written to stdout and to
//! `card-conformance.json` in the temp directory; every failure, with its
//! full expected and actual values, goes to `card-conformance-failures.json`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use ramflux_card::{CardSession, CardSpec, HostEvent, Presentation, SignedPackage, Tree};
use ramflux_desktop_core::{CardBatch, CardInput, CardNode, CardTree, parse_batch};
use serde_json::{Value, json};

use super::fixtures::{Case, Trust};
use crate::moments::card::trace::Drawn;

/// One checked fixture and check.
#[derive(Clone, Debug)]
pub struct Record {
    pub id: String,
    pub kind: String,
    pub variant: Option<String>,
    pub check: &'static str,
    pub pass: bool,
    pub detail: String,
    /// Check C only: the events, results and batches of the run.
    pub trace: Option<Value>,
}

impl Record {
    /// A record that is not about one fixture (for example, kind coverage).
    #[must_use]
    pub fn summary(id: &str, check: &'static str, pass: bool, detail: String) -> Self {
        Self {
            id: id.to_owned(),
            kind: String::new(),
            variant: None,
            check,
            pass,
            detail,
            trace: None,
        }
    }

    fn new(case: &Case, check: &'static str, pass: bool, detail: String) -> Self {
        Self {
            id: case.id.clone(),
            kind: case.kind.clone(),
            variant: case.variant.clone(),
            check,
            pass,
            detail,
            trace: None,
        }
    }

    /// The machine-readable line of this record.
    #[must_use]
    pub fn line(&self) -> Value {
        let mut value = json!({
            "id": self.id,
            "kind": self.kind,
            "variant": self.variant,
            "check": self.check,
            "pass": self.pass,
            "detail": self.detail,
        });
        if let (Some(trace), Some(map)) = (&self.trace, value.as_object_mut()) {
            map.insert("trace".to_owned(), trace.clone());
        }
        value
    }
}

/// A full run: the records, and the failures with their full values.
#[derive(Default)]
pub struct Report {
    pub records: Vec<Record>,
    pub failures: Vec<Value>,
}

impl Report {
    /// Pass and total counts of one check.
    #[must_use]
    pub fn count(&self, check: &str) -> (usize, usize) {
        let rows: Vec<&Record> = self.records.iter().filter(|r| r.check == check).collect();
        (rows.iter().filter(|r| r.pass).count(), rows.len())
    }

    /// Whether every record passed.
    /// Writes the NDJSON records and the failure artifact into `dir`.
    ///
    /// # Errors
    /// Returns the first file that could not be written.
    pub fn write(&self, dir: &Path) -> Result<(), String> {
        let lines: Vec<String> = self.records.iter().map(|r| r.line().to_string()).collect();
        for line in &lines {
            println!("{line}");
        }
        let mut text = String::new();
        for line in &lines {
            text.push_str(line);
            text.push('\n');
        }
        std::fs::write(dir.join("card-conformance.json"), text)
            .map_err(|error| error.to_string())?;
        let failures = serde_json::to_string_pretty(&self.failures).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("card-conformance-failures.json"), failures + "\n")
            .map_err(|error| error.to_string())
    }
}

/// An opened fixture: the live runtime session and the held tree that the
/// desktop builds from the open batch.
pub struct Opened {
    pub session: CardSession,
    pub held: CardTree,
}

/// Opens `case` through the standalone runtime with the presentation `key`.
///
/// # Errors
/// Returns the refusal or failure, as text.
pub fn open(trust: &Trust, case: &Case, key: &str) -> Result<Opened, String> {
    let bytes = trust.presentations.get(key).ok_or_else(|| format!("no presentation {key}"))?;
    let presentation = Presentation::parse(bytes).map_err(|error| error.to_string())?;
    let spec = CardSpec::parse(&case.input)
        .and_then(|spec| spec.with_presentation(presentation))
        .map_err(|error| error.to_string())?;
    let package: SignedPackage = trust.package.clone();
    let (session, batch) = CardSession::open(spec, package).map_err(|error| error.to_string())?;
    let held = held_from(session.id(), &serde_json::to_vec(&batch).map_err(|e| e.to_string())?)?;
    Ok(Opened { session, held })
}

/// A fresh held tree of `session` built from one mutation batch's bytes.
fn held_from(session: u64, bytes: &[u8]) -> Result<CardTree, String> {
    let batch: CardBatch = parse_batch(bytes).map_err(|error| error.to_string())?;
    let mut tree = CardTree::empty(session);
    tree.apply(&batch).map_err(|error| error.to_string())?;
    Ok(tree)
}

/// Replaces the live session id by 1 everywhere it appears.
fn rewrite_session(value: &mut Value, live: u64) {
    match value {
        // Some fields carry a serialized token: normalise it as the fixture does.
        Value::String(text) if text.starts_with('{') => {
            if let Ok(mut inner) = serde_json::from_str::<Value>(text) {
                rewrite_session(&mut inner, live);
                *text = canonical(&inner);
            }
        }
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if key == "session" && child.as_u64() == Some(live) {
                    *child = json!(1);
                } else {
                    rewrite_session(child, live);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| rewrite_session(item, live)),
        _ => {}
    }
}

/// The presentation the conformance runs with.
pub const PRESENTATION: &str = "en_light";

/// Check A for one case. Returns the opened session when it opened, for
/// checks B and C.
#[must_use]
pub fn check_a(trust: &Trust, case: &Case, report: &mut Report) -> Option<Opened> {
    let opened = open(trust, case, PRESENTATION);
    if case.refused {
        let pass = opened.is_err();
        let detail = match &opened {
            Err(error) => format!("refused: {error}"),
            Ok(_) => "admitted; the index expects a refusal".to_owned(),
        };
        report.records.push(Record::new(case, "A", pass, detail));
        return None;
    }
    let live = match opened {
        Ok(opened) => opened,
        Err(error) => {
            report.records.push(Record::new(case, "A", false, error.clone()));
            report.failures.push(json!({"id": case.id, "check": "A", "error": error}));
            return None;
        }
    };
    let mut actual = serde_json::to_value(live.session.tree()).unwrap_or(Value::Null);
    rewrite_session(&mut actual, live.session.id());
    let pass = actual == case.expected;
    let detail = if pass { "tree equal".to_owned() } else { "tree differs".to_owned() };
    if !pass {
        report.failures.push(json!({
            "id": case.id,
            "check": "A",
            "expected": case.expected,
            "actual": actual,
        }));
    }
    report.records.push(Record::new(case, "A", pass, detail));
    // B and C run on the session even when A differs: they check other things.
    Some(live)
}

/// The hidden and reachable nodes of a held tree: what the renderer draws.
fn walk(tree: &CardTree) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut reachable = BTreeSet::new();
    let mut hidden = BTreeSet::new();
    let Some(root) = tree.root() else { return (reachable, hidden) };
    let mut stack = vec![(root.to_owned(), false)];
    while let Some((key, inside_hidden)) = stack.pop() {
        let Some(node) = tree.node(&key) else { continue };
        if inside_hidden {
            hidden.insert(key.clone());
        } else {
            reachable.insert(key.clone());
        }
        let modal = node.kind == "Modal";
        let tabs = node.kind == "Tabs";
        for (index, child) in node.children.iter().enumerate() {
            let child_hidden = inside_hidden || modal || (tabs && index > 0);
            stack.push((child.clone(), child_hidden));
        }
    }
    (reachable, hidden)
}

/// Check B for one case, from the trace the renderer left for it.
pub fn check_b(case: &Case, held: &CardTree, drawn: &[Drawn], report: &mut Report) {
    let (reachable, hidden) = walk(held);
    let drawn_keys: BTreeSet<&str> = drawn.iter().map(|d| d.key.as_str()).collect();
    let placeholders: Vec<&Drawn> = drawn.iter().filter(|d| !d.known).collect();
    let missing: Vec<&String> =
        reachable.iter().filter(|key| !drawn_keys.contains(key.as_str())).collect();
    let slots_missing: Vec<&str> = held
        .host_slots()
        .iter()
        .map(|slot| slot.key.as_str())
        .filter(|key| !drawn_keys.contains(key) && !hidden.contains(*key))
        .collect();
    let root_drawn = held.root().is_some_and(|root| drawn_keys.contains(root));
    let pass =
        placeholders.is_empty() && missing.is_empty() && slots_missing.is_empty() && root_drawn;
    let detail = format!(
        "drawn {}, reachable {}, hidden {}, placeholders {}, host slots {}",
        drawn.len(),
        reachable.len(),
        hidden.len(),
        placeholders.len(),
        held.host_slots().len(),
    );
    if !pass {
        report.failures.push(json!({
            "id": case.id,
            "check": "B",
            "expected": {"reachable": reachable, "root_drawn": root_drawn},
            "actual": {
                "drawn": drawn.iter().map(|d| json!({"key": d.key, "kind": d.kind, "known": d.known})).collect::<Vec<_>>(),
                "missing": missing,
                "placeholders": placeholders.iter().map(|d| d.key.clone()).collect::<Vec<_>>(),
                "slots_missing": slots_missing,
            },
        }));
    }
    report.records.push(Record::new(case, "B", pass, detail));
}

/// The scripted host events of one case: one per interactive node, in
/// preorder, with the input shape the core's components take.
fn script(tree: &CardTree) -> Vec<(String, CardInput, &'static str)> {
    let mut events = Vec::new();
    let Some(root) = tree.root() else { return events };
    let mut order = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(key) = stack.pop() {
        let Some(node) = tree.node(&key) else { continue };
        order.push(key);
        for child in node.children.iter().rev() {
            stack.push(child.clone());
        }
    }
    for key in order {
        let Some(node) = tree.node(&key) else { continue };
        // The renderer sends nothing for a disabled control.
        if node.attrs.get("disabled").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        // A Modal opens through its trigger: nothing is sent for a disabled one.
        let trigger_disabled = node.kind == "Modal"
            && node.children.first().and_then(|key| tree.node(key)).is_some_and(|trigger| {
                trigger.attrs.get("disabled").and_then(Value::as_bool).unwrap_or(false)
            });
        if trigger_disabled {
            continue;
        }
        for (input, kind) in events_for(node) {
            events.push((key.clone(), input, kind));
        }
    }
    events
}

/// The rows, one after another, with `sep` between them.
fn separated(rows: &[String], sep: char) -> String {
    let mut out = String::new();
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            out.push(sep);
        }
        out.push_str(row);
    }
    out
}

/// JSON text with object keys in sorted order, as the fixtures are written.
fn canonical(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let rows: Vec<String> = keys
                .into_iter()
                .map(|key| format!("{}:{}", Value::String(key.clone()), canonical(&map[key])))
                .collect();
            format!("{{{}}}", separated(&rows, ','))
        }
        Value::Array(items) => {
            let rows: Vec<String> = items.iter().map(canonical).collect();
            format!("[{}]", separated(&rows, ','))
        }
        other => other.to_string(),
    }
}

/// The events the renderer sends for one interactive node.
fn events_for(node: &CardNode) -> Vec<(CardInput, &'static str)> {
    let attr = |name: &str| node.attrs.get(name);
    match node.kind.as_str() {
        "TextField" => {
            let text = if attr("variant").and_then(Value::as_str) == Some("number") {
                "42"
            } else {
                "Ada"
            };
            vec![(CardInput::Input { value: json!(text) }, "input")]
        }
        "CheckBox" => {
            let checked = attr("value").and_then(Value::as_bool).unwrap_or(false);
            vec![(CardInput::Input { value: json!(!checked) }, "input")]
        }
        "ChoicePicker" => {
            let first = attr("options")
                .and_then(Value::as_array)
                .and_then(|rows| rows.first())
                .and_then(|row| row.get("value"))
                .and_then(Value::as_str);
            first
                .map(|value| vec![(CardInput::Input { value: json!([value]) }, "input")])
                .unwrap_or_default()
        }
        "Slider" => {
            let min = attr("min").and_then(Value::as_f64).unwrap_or(0.0);
            let max = attr("max").and_then(Value::as_f64).unwrap_or(100.0).max(min);
            let middle = ((min + max) / 2.0).round() as i64;
            vec![(CardInput::Input { value: json!(middle) }, "input")]
        }
        "DateTimeInput" => {
            let date = attr("enableDate").and_then(Value::as_bool).unwrap_or(false);
            let time = attr("enableTime").and_then(Value::as_bool).unwrap_or(false);
            let text = match (date, time) {
                (true, true) => "2026-11-23T12:00:00Z",
                (true, false) => "2026-11-23",
                (false, true) => "12:00:00",
                (false, false) => return Vec::new(),
            };
            vec![(CardInput::Input { value: json!(text) }, "input")]
        }
        "Button" => vec![(CardInput::Activate, "activate")],
        "Tabs" => {
            let index = if node.children.len() > 1 { 1 } else { 0 };
            vec![(CardInput::SelectTab { index }, "select_tab")]
        }
        "Modal" => {
            vec![(CardInput::ModalOpen, "modal_open"), (CardInput::ModalClose, "modal_close")]
        }
        _ => Vec::new(),
    }
}

/// Whether the held tree equals the runtime's tree in every field the
/// renderer draws or addresses.
fn same_tree(held: &CardTree, tree: &Tree) -> Result<(), String> {
    if held.root() != Some(tree.root_key.as_str()) {
        return Err("root differs".to_owned());
    }
    if held.len() != tree.nodes.len() {
        return Err(format!("node count {} != {}", held.len(), tree.nodes.len()));
    }
    if held.signed_hash() != tree.signed_hash
        || held.locale() != tree.locale
        || held.format_locale() != tree.format_locale
        || held.direction() != tree.direction
    {
        return Err("tree fields differ".to_owned());
    }
    for node in &tree.nodes {
        let Some(mine) = held.node(&node.key) else {
            return Err(format!("missing node {}", node.key));
        };
        let kind = serde_json::to_value(node.kind).map_err(|e| e.to_string())?;
        if kind.as_str() != Some(mine.kind.as_str())
            || mine.attrs != node.attrs
            || mine.children != node.children
            || mine.parent_key != node.parent_key
        {
            return Err(format!("node differs at {}", node.key));
        }
    }
    let slots: Vec<&str> = held.host_slots().iter().map(|s| s.id.as_str()).collect();
    let theirs: Vec<&str> = tree.host_slots.iter().map(|s| s.id.as_str()).collect();
    if slots != theirs {
        return Err(format!("host slots differ: {slots:?} != {theirs:?}"));
    }
    if let Some(root) = held.root().map(str::to_owned) {
        let token = held.token_for(&root).ok_or("no token for root")?;
        if token.session != tree.session
            || token.signed_version != tree.signed_version
            || token.edit_seq != tree.edit_seq
            || token.tree_version != tree.tree_version
        {
            return Err("tokens differ".to_owned());
        }
    }
    Ok(())
}

/// Check C for one case: drives every scripted event through the runtime and
/// checks, after each one, that the held tree equals the runtime's tree.
pub fn check_c(case: &Case, opened: &mut Opened, report: &mut Report) {
    let events = script(&opened.held);
    let approval = opened.held.nodes().find(|node| node.kind == "Approval").map(|n| n.key.clone());
    if events.is_empty() && approval.is_none() {
        return;
    }
    let mut steps = Vec::new();
    let mut failure: Option<String> = None;
    for (key, input, label) in &events {
        if let Err(error) = send(opened, key, input, label, &mut steps) {
            failure = Some(error);
            break;
        }
    }
    if failure.is_none()
        && let Some(key) = approval
    {
        // An approval is answered by its host slot, which the SDK owns; the
        // runtime exposes the slot and its token, and nothing else to drive.
        let slot = opened.held.host_slots().iter().find(|slot| slot.key == key);
        if slot.is_none() {
            failure = Some(format!("approval {key} has no host slot"));
        }
    }
    let pass = failure.is_none();
    let detail = match &failure {
        None => format!("{} events, all self-consistent", events.len()),
        Some(message) => message.clone(),
    };
    let mut record = Record::new(case, "C", pass, detail.clone());
    record.trace = Some(json!({"steps": steps}));
    if !pass {
        report.failures.push(json!({
            "id": case.id,
            "check": "C",
            "error": detail,
            "steps": steps,
        }));
    }
    report.records.push(record);
}

/// Sends one host event through the runtime, applies the returned batch to
/// the held tree and compares the held tree with the runtime's tree.
///
/// # Errors
/// Returns the refusal, the batch that does not apply, or the first field
/// that differs.
pub fn send(
    opened: &mut Opened,
    key: &str,
    input: &CardInput,
    label: &str,
    steps: &mut Vec<Value>,
) -> Result<(), String> {
    let token = opened.held.token_for(key).ok_or_else(|| format!("no token for {key}"))?;
    let event = json!({
        "token": {
            "session": token.session,
            "signedVersion": token.signed_version,
            "editSeq": token.edit_seq,
            "treeVersion": token.tree_version,
            "key": token.key,
        },
        "event": serde_json::to_value(input).map_err(|e| e.to_string())?,
    });
    let host: HostEvent = serde_json::from_value(event.clone()).map_err(|e| e.to_string())?;
    if let Err(error) = opened.session.enqueue(host) {
        steps.push(json!({"event": event, "refused": error.to_string()}));
        return Err(format!("{label} on {key} refused: {error}"));
    }
    let result = opened
        .session
        .next_event()
        .map_err(|error| format!("{label} on {key}: {error}"))?
        .ok_or_else(|| format!("{label} on {key} produced no result"))?;
    let result = serde_json::to_value(&result).map_err(|e| e.to_string())?;
    steps.push(json!({"event": event, "result": result}));
    let outcome = result.get("result").and_then(Value::as_str).unwrap_or_default().to_owned();
    if outcome == "refused" || outcome == "stale_dropped" {
        return Err(format!("{label} on {key}: the runtime answered {outcome}"));
    }
    if let Some(batch) = result.get("batch") {
        apply_batch(&mut opened.held, batch.to_string().as_bytes())
            .map_err(|error| format!("{label} on {key}: batch does not apply: {error}"))?;
    }
    let live = opened.session.tree().ok_or("the runtime has no tree")?;
    same_tree(&opened.held, live).map_err(|error| format!("{label} on {key}: {error}"))
}

/// Applies a batch's bytes to a held tree.
fn apply_batch(held: &mut CardTree, bytes: &[u8]) -> Result<(), String> {
    let batch = parse_batch(bytes).map_err(|error| error.to_string())?;
    held.apply(&batch).map_err(|error| error.to_string())
}

/// Check A and check C for every case. Returns, per case, the held tree the
/// runtime opened with (the starting point of check B). A case that does not
/// open fails B and C here, so a failed A never hides the other checks.
pub fn run_ac(trust: &Trust, cases: &[Case], report: &mut Report) -> BTreeMap<String, CardTree> {
    let mut initial = BTreeMap::new();
    for case in cases {
        match check_a(trust, case, report) {
            Some(mut opened) => {
                initial.insert(case.id.clone(), opened.held.clone());
                check_c(case, &mut opened, report);
            }
            None if !case.refused => {
                let detail = "not run: the fixture did not open".to_owned();
                report.records.push(Record::new(case, "B", false, detail.clone()));
                report.records.push(Record::new(case, "C", false, detail));
            }
            None => {}
        }
    }
    initial
}
