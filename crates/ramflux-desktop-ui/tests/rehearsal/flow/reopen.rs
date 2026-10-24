// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Live step: Bob quits his desktop, Alice writes to him while it is
//! closed, Bob opens it again. The new desktop must resume the event stream
//! from the cursor the old one saved (`bus.subscribe` with `since_seq`, as
//! the frame tap shows), and show every message exactly once: the ones from
//! before the restart and the ones written while it was closed.

use ramflux_desktop_core::{ContextId, LockState, MomentKind, Store, SyncMode};
use ramflux_desktop_ui::route::Page;
use serde_json::Value;

use super::{Demo, probe};
use crate::rehearsal::daemon::DemoDaemon;
use crate::rehearsal::desk::Desk;
use crate::rehearsal::host::Host;
use crate::rehearsal::steps::Notes;
use crate::rehearsal::tap;

/// How many messages in `context` have the text `text`.
fn copies(store: &Store, context: &ContextId, text: &str) -> usize {
    store
        .context_moments(context)
        .filter(|m| matches!(m.kind, MomentKind::Message(_)) && store.body(&m.id) == Some(text))
        .count()
}

/// The stream cursor the settings file holds (one account, one socket).
fn saved_cursor(settings: &std::path::Path) -> Option<u64> {
    let text = std::fs::read_to_string(settings).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    value["stream_cursors"]
        .as_object()?
        .values()
        .find_map(|accounts| accounts.as_object()?.values().find_map(Value::as_u64))
}

/// The live desktop-restart step, on Bob's side.
pub fn reopen<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    notes: &mut Notes,
) -> Result<(), String> {
    let limits = demo.config.limits;
    let run = demo.config.run_id.clone();
    let dm = demo.conversation.clone();
    let dir = demo.sides[1].daemon.socket().parent().map(std::path::Path::to_path_buf);
    let dir = dir.ok_or("Bob's daemon has no folder")?;
    let (settings, log) = (dir.join("desktop-settings.json"), dir.join("tap.jsonl"));

    let live = demo.read(1, |s| s.sync_status().cloned())?;
    notes.add(format!("Bob before quitting: {live:?}"));
    let desk = demo.sides[1].desk.take().ok_or("Bob's desktop is not open")?;
    desk.close(demo.host)?;
    // Let the dropped source finish writing its cursor and close its streams.
    let closed = std::time::Instant::now();
    while closed.elapsed() < std::time::Duration::from_secs(2) {
        demo.host.tick();
    }
    let before = saved_cursor(&settings);
    let marker = tap::read_log(&log).len();
    notes.add(format!("Bob's desktop closed; cursor saved in desktop-settings.json: {before:?}"));
    let before = before.ok_or("the closed desktop left no stream cursor")?;

    let offline = [
        format!("Sent while Bob's desktop was closed, 1 ({run})"),
        format!("Sent while Bob's desktop was closed, 2 ({run})"),
    ];
    let alice = demo.sides[0].facts()?.principal.clone();
    let desk = demo.sides[0].desk()?;
    desk.show_context(demo.host, &dm)?;
    for text in &offline {
        let desk = demo.sides[0].desk()?;
        desk.type_into(demo.host, "composer-input", text)?;
        desk.click(demo.host, None, "composer-send")?;
        let context = dm.clone();
        demo.wait(0, limits.receipt, "the offline message delivered", |desk, host| {
            desk.store(host, |s| probe::delivered(probe::receipt(s, &context, text, &alice)))
        })?;
    }
    notes.add("Alice sent two messages while Bob's desktop was closed; both Delivered");

    let options = (demo.options)(&demo.sides[1].daemon.socket());
    let reopened = std::time::Instant::now();
    demo.sides[1].desk = Some(Desk::open(demo.host, demo.sides[1].tag, options));
    demo.wait(1, limits.answer, "the reopened desktop", |desk, host| {
        let page = desk.page(host);
        page == Page::Workspace || matches!(page, Page::Entry(_))
    })?;
    if demo.sides[1].desk()?.page(demo.host) != Page::Workspace {
        let locked =
            demo.read(1, |s| probe::account(s).is_some_and(|a| a.lock_state == LockState::Locked))?;
        if locked {
            let secret = demo.sides[1].passphrase.clone();
            let desk = demo.sides[1].desk()?;
            desk.type_into(demo.host, "passphrase", &secret)?;
            desk.click(demo.host, None, "unlock")?;
            notes.add("the reopened desktop showed the account locked; unlocked");
        }
    }
    demo.wait(1, limits.answer, "the workspace on the stream", |desk, host| {
        desk.page(host) == Page::Workspace
            && desk.store(host, |s| s.sync_status().is_some_and(|x| x.mode == SyncMode::Stream))
    })?;
    notes.add(format!(
        "Bob's desktop reopened in {:.1} s on the stream",
        reopened.elapsed().as_secs_f32()
    ));
    let context = dm.clone();
    demo.wait(1, limits.answer, "the DM conversation", |desk, host| {
        desk.store(host, |s| s.context(&context).is_some())
    })?;
    demo.sides[1].desk()?.show_context(demo.host, &dm)?;
    let mut all: Vec<(ContextId, String)> =
        demo.texts.iter().map(|t| (dm.clone(), t.clone())).collect();
    all.extend(offline.iter().map(|t| (dm.clone(), t.clone())));
    if let Some((group, texts)) = demo.group.clone() {
        all.extend(texts.into_iter().map(|t| (group.clone(), t)));
    }
    let wanted = all.clone();
    demo.wait(1, limits.answer, "every message after the restart", |desk, host| {
        desk.store(host, |s| wanted.iter().all(|(c, t)| copies(s, c, t) >= 1))
    })?;
    // Give a late replay time to add a duplicate, then count.
    let settle = std::time::Instant::now();
    while settle.elapsed() < std::time::Duration::from_secs(3) {
        demo.host.tick();
    }
    let counts = demo
        .read(1, |s| all.iter().map(|(c, t)| (t.clone(), copies(s, c, t))).collect::<Vec<_>>())?;
    let wrong: Vec<_> = counts.iter().filter(|(_, n)| *n != 1).collect();
    if !wrong.is_empty() {
        return Err(format!(
            "after the restart some messages are not shown exactly once: {wrong:?}"
        ));
    }
    notes.add(format!(
        "after the restart all {} messages (DM before, DM while closed, group) shown exactly once",
        counts.len()
    ));
    let after = demo.read(1, |s| s.sync_status().and_then(|x| x.cursor))?;

    let lines = tap::read_log(&log);
    let new = &lines[marker.min(lines.len())..];
    let subscribes: Vec<&Value> =
        new.iter().filter(|l| l["dir"] == "up" && l["method"] == "bus.subscribe").collect();
    let since: Vec<Option<u64>> = subscribes.iter().map(|l| l["since_seq"].as_u64()).collect();
    let resync = new.iter().any(|l| l["code"].as_str().is_some_and(|c| c.contains("resync")));
    let replayed: Vec<u64> = new
        .iter()
        .filter(|l| l["dir"] == "down" && l["kind"] == "event")
        .filter_map(|l| l["seq"].as_u64())
        .collect();
    let polled: Vec<&str> = new
        .iter()
        .filter(|l| l["dir"] == "up")
        .filter_map(|l| l["method"].as_str())
        .filter(|m| *m == "message.receive" || *m == "dm.read")
        .collect();
    notes.add(format!(
        "tap after reopen: bus.subscribe since_seq {since:?}; resync_required: {resync}; events \
         delivered: {} (first seq {:?}, last {:?}); message.receive calls: {}",
        replayed.len(),
        replayed.first(),
        replayed.last(),
        polled.len()
    ));
    notes.add(format!("cursor before restart {before}, after {after:?}"));
    if !since.contains(&Some(before)) {
        return Err(format!(
            "the reopened desktop did not resume from its cursor {before}: subscribed with {since:?}"
        ));
    }
    if resync {
        return Err("the daemon asked for a resync after the restart".to_owned());
    }
    if !replayed.first().is_some_and(|seq| *seq > before) {
        return Err(format!("the replay did not start after the cursor: {replayed:?}"));
    }
    demo.shots(1, "08_reopened", notes)?;
    Ok(())
}
