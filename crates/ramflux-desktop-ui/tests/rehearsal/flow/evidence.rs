// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Live step: what each desktop's frame tap saw over the whole run. With
//! polling off the desktop must be on the event stream (`SyncMode::Stream`)
//! and must never have pulled messages (`message.receive`); every message
//! it showed came from the stream's events and the reads they called for.

use std::collections::BTreeMap;

use ramflux_desktop_core::SyncMode;

use super::Demo;
use crate::rehearsal::daemon::DemoDaemon;
use crate::rehearsal::host::Host;
use crate::rehearsal::steps::Notes;
use crate::rehearsal::tap;

/// Pulls a desktop on the stream never makes.
const PULLS: [&str; 2] = ["message.receive", "dm.read"];

/// The live stream-evidence step.
pub fn stream<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    notes: &mut Notes,
) -> Result<(), String> {
    let mut problems: Vec<String> =
        demo.fallbacks.iter().map(|f| format!("stream did not deliver in time: {f}")).collect();
    for ix in 0..2 {
        let name = demo.sides[ix].name;
        let status = demo.read(ix, |s| s.sync_status().cloned())?;
        let mode = status.as_ref().map(|s| s.mode);
        notes.add(format!("{name}: sync {status:?}"));
        if mode != Some(SyncMode::Stream) {
            problems.push(format!("{name} is not on the stream ({mode:?})"));
        }
        let Some(dir) = demo.sides[ix].daemon.socket().parent().map(std::path::Path::to_path_buf)
        else {
            continue;
        };
        let lines = tap::read_log(&dir.join("tap.jsonl"));
        let mut methods: BTreeMap<String, usize> = BTreeMap::new();
        for line in lines.iter().filter(|l| l["dir"] == "up") {
            if let Some(m) = line["method"].as_str() {
                *methods.entry(m.to_owned()).or_default() += 1;
            }
        }
        let mut events: BTreeMap<String, usize> = BTreeMap::new();
        for line in lines.iter().filter(|l| l["dir"] == "down" && l["kind"] == "event") {
            if let Some(m) = line["method"].as_str() {
                *events.entry(m.to_owned()).or_default() += 1;
            }
        }
        let subscribes: Vec<String> = lines
            .iter()
            .filter(|l| l["dir"] == "up" && l["method"] == "bus.subscribe")
            .map(|l| format!("since_seq={}", l["since_seq"]))
            .collect();
        notes.add(format!("{name}: bus.subscribe {subscribes:?}"));
        notes.add(format!("{name}: events received {events:?}"));
        notes.add(format!("{name}: requests sent {methods:?}"));
        for pull in PULLS {
            if let Some(n) = methods.get(pull) {
                problems.push(format!("{name}'s desktop called {pull} {n} times"));
            }
        }
        if events.get("message.received").is_none_or(|n| *n == 0) {
            problems.push(format!("{name}'s desktop got no message.received event"));
        }
    }
    if problems.is_empty() { Ok(()) } else { Err(problems.join("; ")) }
}
