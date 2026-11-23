// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Files through the desktop: sending one with the composer's attach button
//! and downloading a received one with its Download button, as a person
//! does; the bytes are compared with what was sent.

use std::path::{Path, PathBuf};

use ramflux_desktop_core::{ContextId, Direction, MomentKind, Object, Store, TransferState};

use super::Demo;
use crate::rehearsal::daemon::DemoDaemon;
use crate::rehearsal::host::Host;
use crate::rehearsal::steps::Notes;

/// Writes a text file of `size` bytes named `name` in `dir`.
pub fn text_file(dir: &Path, name: &str, size: usize) -> Result<(PathBuf, Vec<u8>), String> {
    let bytes: Vec<u8> = (0..size).map(|i| b"ramflux rehearsal file\n"[i % 23]).collect();
    let path = dir.join(name);
    std::fs::write(&path, &bytes).map_err(|e| format!("cannot write {name}: {e}"))?;
    Ok((path, bytes))
}

/// Writes the rehearsal image named `name` in `dir`.
pub fn image_file(dir: &Path, name: &str) -> Result<(PathBuf, Vec<u8>), String> {
    let path = dir.join(name);
    let png = ramflux_desktop_test::v1_fixtures::REAL_PNG;
    std::fs::write(&path, png).map_err(|e| format!("cannot write {name}: {e}"))?;
    Ok((path, png.to_vec()))
}

/// The object of the file named `name` in `context` in `direction`.
pub fn object_named(
    store: &Store,
    context: &ContextId,
    name: &str,
    direction: Direction,
) -> Option<Object> {
    store.context_moments(context).find_map(|moment| match &moment.kind {
        MomentKind::Message(payload) => payload.media.iter().find_map(|media| {
            store
                .object(&media.object_id)
                .filter(|o| o.direction == direction && o.name.as_deref() == Some(name))
                .cloned()
        }),
        _ => None,
    })
}

/// The moment carrying the object `object` and the file's index in it.
fn carrier(store: &Store, context: &ContextId, object: &Object) -> Option<(String, usize)> {
    store.context_moments(context).find_map(|moment| match &moment.kind {
        MomentKind::Message(payload) => payload
            .media
            .iter()
            .position(|m| m.object_id == object.id)
            .map(|ix| (moment.id.to_string(), ix)),
        _ => None,
    })
}

/// Side `ix` sends the file at `path` in `context` with the attach button
/// and Send; waits until its object is sent (or says why not).
pub fn send_file<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    ix: usize,
    context: &ContextId,
    path: &Path,
    notes: &mut Notes,
) -> Result<(), String> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
    let who = demo.sides[ix].name;
    let desk = demo.sides[ix].desk()?;
    desk.show_context(demo.host, context)?;
    let chosen = desk.attach_file(demo.host, path)?;
    if !desk.exists(demo.host, "composer-attachment") {
        return Err(format!("{who}: the chosen file {name} is not staged in the composer"));
    }
    desk.click(demo.host, None, "composer-send")?;
    let limit = demo.config.limits.answer;
    let context_id = context.clone();
    let wanted = name.clone();
    let mut moved = Vec::new();
    let waited = demo.wait(ix, limit, "the file sent", |desk, host| {
        desk.store(host, |s| match object_named(s, &context_id, &wanted, Direction::Outbound) {
            Some(Object { transfer: TransferState::Transferring { transferred }, .. }) => {
                if moved.last() != Some(&transferred) {
                    moved.push(transferred);
                }
                false
            }
            Some(o) => !matches!(o.transfer, TransferState::Queued),
            None => false,
        })
    });
    let sent = demo.read(ix, |s| object_named(s, context, &name, Direction::Outbound))?;
    match sent.as_ref().map(|o| &o.transfer) {
        Some(TransferState::Complete) => {}
        Some(TransferState::Failed { reason }) => {
            return Err(format!("{who}: {name} was not sent: {reason}"));
        }
        other => {
            return Err(format!(
                "{who}: {name} did not finish ({other:?}): {}",
                waited.err().unwrap_or_default()
            ));
        }
    }
    notes.add(format!(
        "{who}: sent {name} ({} bytes, {chosen}); progress seen {moved:?}",
        sent.and_then(|o| o.size).unwrap_or(0)
    ));
    Ok(())
}

/// Side `ix` waits for the received file named `name` in `context`,
/// presses its Download button, waits until it is downloaded and returns
/// the downloaded bytes.
pub fn receive_file<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    ix: usize,
    context: &ContextId,
    name: &str,
    notes: &mut Notes,
) -> Result<Vec<u8>, String> {
    let who = demo.sides[ix].name;
    let limit = demo.config.limits.answer;
    let context_id = context.clone();
    let wanted = name.to_owned();
    demo.wait(ix, limit, "the received file listed", |desk, host| {
        desk.store(host, |s| object_named(s, &context_id, &wanted, Direction::Inbound).is_some())
    })?;
    demo.sides[ix].desk()?.show_context(demo.host, context)?;
    let object = demo
        .read(ix, |s| object_named(s, context, name, Direction::Inbound))?
        .ok_or_else(|| format!("{who}: no received file {name}"))?;
    let (moment, index) = demo
        .read(ix, |s| carrier(s, context, &object))?
        .ok_or_else(|| format!("{who}: no message carries {name}"))?;
    notes.add(format!(
        "{who}: {name} listed as received ({:?}, {:?} bytes)",
        object.transfer, object.size
    ));
    let scope = format!("moment-{moment}");
    let button = format!("message-media-download-{index}");
    // The row is drawn once the conversation shows it.
    let drawn = demo
        .wait(ix, limit, "the Download button", |desk, host| desk.exists_in(host, &scope, &button));
    if drawn.is_err() {
        return Err(format!("{who}: no Download button on {name} ({:?})", object.transfer));
    }
    demo.sides[ix].desk()?.click(demo.host, Some(&scope), &button)?;
    let mut moved = Vec::new();
    let fetch = demo.config.limits.fetch.unwrap_or(limit);
    let id = object.id.clone();
    demo.wait(ix, fetch, "the file downloaded", |desk, host| {
        desk.store(host, |s| match s.object(&id).map(|o| &o.transfer) {
            Some(TransferState::Transferring { transferred }) => {
                if moved.last() != Some(transferred) {
                    moved.push(*transferred);
                }
                false
            }
            Some(TransferState::Complete | TransferState::Failed { .. }) => true,
            _ => false,
        })
    })?;
    let done = demo.read(ix, |s| s.object(&id).cloned())?.ok_or("the object is gone")?;
    if let TransferState::Failed { reason } = &done.transfer {
        return Err(format!("{who}: the download of {name} failed: {reason}"));
    }
    let path = done.local_path.clone().ok_or_else(|| format!("{who}: {name} has no local copy"))?;
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {path}: {e}"))?;
    notes.add(format!(
        "{who}: downloaded {name} with its Download button ({} bytes; progress seen {moved:?})",
        bytes.len()
    ));
    Ok(bytes)
}

/// Side `ix` presses Retry on its failed file named `name` in `context` and
/// waits until it is sent (or says why not).
pub fn retry_file<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    ix: usize,
    context: &ContextId,
    name: &str,
    notes: &mut Notes,
) -> Result<(), String> {
    let who = demo.sides[ix].name;
    demo.sides[ix].desk()?.show_context(demo.host, context)?;
    let object = demo
        .read(ix, |s| object_named(s, context, name, Direction::Outbound))?
        .ok_or_else(|| format!("{who}: no file {name} to retry"))?;
    let (moment, index) = demo
        .read(ix, |s| carrier(s, context, &object))?
        .ok_or_else(|| format!("{who}: no message carries {name}"))?;
    let scope = format!("moment-{moment}");
    let button = format!("message-media-retry-{index}");
    demo.sides[ix].desk()?.click(demo.host, Some(&scope), &button)?;
    let id = object.id.clone();
    let limit = demo.config.limits.answer;
    let _ = demo.wait(ix, limit, "the retried file sent", |desk, host| {
        desk.store(host, |s| {
            matches!(
                s.object(&id).map(|o| &o.transfer),
                Some(TransferState::Complete | TransferState::Failed { .. })
            )
        })
    });
    match demo.read(ix, |s| s.object(&id).map(|o| o.transfer.clone()))? {
        Some(TransferState::Complete) => {
            notes.add(format!("{who}: Retry sent {name}"));
            Ok(())
        }
        other => Err(format!("{who}: {name} still not sent after Retry: {other:?}")),
    }
}
