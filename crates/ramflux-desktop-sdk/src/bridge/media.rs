// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Running attachment jobs (see `plan::media`): each runs as its own task on
//! the scope's request connection and reports the bytes it moved as it
//! goes; a running job can be cancelled, which also cancels the daemon's
//! prepare or import handle.
//!
//! Prepare and import requests carry no retry key: the daemon makes them
//! idempotent by their own ids (`operation_id` in the body), and a replayed
//! answer would be stale. Only `conversation.send` carries one, fresh for
//! each attempt (a retry after a refusal is a new send of the same prepare,
//! which the daemon resumes under the job's operation).
//!
//! Downloaded files go to the desktop's private media folder (beside the
//! desktop settings, per account, owner-only), named by their object id;
//! the sender's file name is never a path. A partial download is a
//! temporary file removed on failure.

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use ramflux_desktop_core::{DomainEvent, ObjectId};
use serde_json::{Value, json};

use super::now;
use super::scope::{Internal, Task};
use crate::bus::client::BusClient;
use crate::bus::{BusError, BusRequest};
use crate::plan::media::{CHUNK, ImportJob, Job, MediaRef, SendJob};
use crate::translate::objects::{media_code, media_reason};
use crate::translate::reason::key;

/// The daemon handle a running job holds, to cancel.
#[derive(Debug, Clone, Default)]
enum Handle {
    #[default]
    None,
    Prepare {
        id: String,
        generation: u64,
    },
    Import {
        id: String,
    },
}

/// The running attachment jobs of a scope.
#[derive(Default)]
pub(super) struct MediaJobs {
    running: BTreeMap<ObjectId, (tokio::task::AbortHandle, Arc<Mutex<Handle>>)>,
}

/// What a running job reports.
pub(super) enum Note {
    Progress(ObjectId, u64),
    Sent(Box<SendJob>, Result<Value, BusError>),
    Imported(ObjectId, Result<(PathBuf, Option<String>), BusError>),
    Copied(ObjectId, Result<String, String>),
}

impl Task {
    /// Starts (or cancels) an attachment job.
    pub(super) fn media(&mut self, job: Job) {
        let Some(account) = self.scope.account().map(|a| a.as_str().to_owned()) else { return };
        match job {
            Job::Send(job) => {
                if self.media_jobs.running.contains_key(&job.object) {
                    return;
                }
                let events = self.translator.media_sending(&job, now());
                self.emit(events);
                let handle = Arc::new(Mutex::new(Handle::None));
                let (client, internal, held) =
                    (self.client.clone(), self.internal.clone(), handle.clone());
                let object = job.object.clone();
                let task = tokio::spawn(async move {
                    let result = send(&client, &account, &job, &held, &internal).await;
                    let _ = internal.send(Internal::Media(Note::Sent(job, result)));
                });
                self.media_jobs.running.insert(object, (task.abort_handle(), handle));
            }
            Job::Import(job) => {
                if self.media_jobs.running.contains_key(&job.object) {
                    return;
                }
                let Some(folder) = self.media_folder(&account) else {
                    let reason = crate::translate::reason::keyed(
                        key::FILE,
                        &[("reason", "no folder for downloaded files")],
                    );
                    let events = self.translator.media_import_failed(&job.object, &reason);
                    self.emit(events);
                    return;
                };
                let events = self.translator.media_progress(&job.object, 0);
                self.emit(events);
                let handle = Arc::new(Mutex::new(Handle::None));
                let (client, internal, held) =
                    (self.client.clone(), self.internal.clone(), handle.clone());
                let object = job.object.clone();
                let task = tokio::spawn(async move {
                    let result = import(&client, &account, &job, &folder, &held, &internal).await;
                    let _ = internal.send(Internal::Media(Note::Imported(job.object, result)));
                });
                self.media_jobs.running.insert(object, (task.abort_handle(), handle));
            }
            Job::Copy { object, from, to } => {
                let internal = self.internal.clone();
                tokio::spawn(async move {
                    let result = std::fs::copy(&from, &to)
                        .map(|_| to.display().to_string())
                        .map_err(|e| e.to_string());
                    let _ = internal.send(Internal::Media(Note::Copied(object, result)));
                });
            }
            Job::Cancel { object } => self.cancel_media(&account, &object),
        }
    }

    fn cancel_media(&mut self, account: &str, object: &ObjectId) {
        let Some((abort, handle)) = self.media_jobs.running.remove(object) else { return };
        abort.abort();
        let request = match handle.lock().clone() {
            Handle::None => None,
            Handle::Prepare { id, generation } => Some(BusRequest::new(
                Some(account),
                "attachment",
                "attachment.prepare.cancel",
                json!({ "prepare_id": id, "generation": generation }),
            )),
            Handle::Import { id } => Some(BusRequest::new(
                Some(account),
                "attachment",
                "attachment.import.cancel",
                json!({ "import_id": id }),
            )),
        };
        if let Some(request) = request {
            let client = self.client.clone();
            tokio::spawn(async move {
                let _ = client.call(&request).await;
            });
        }
        let events = match self.translator_media(object) {
            Some(MediaRef::Outgoing(job)) => {
                self.translator.media_send_failed(&job, key::MEDIA_CANCELLED, now())
            }
            _ => self.translator.media_import_failed(object, key::MEDIA_CANCELLED),
        };
        self.emit(events);
    }

    fn translator_media(&self, object: &ObjectId) -> Option<MediaRef> {
        crate::plan::Known::media(&self.translator, object).cloned()
    }

    /// Applies what a running job reported.
    pub(super) fn on_media(&mut self, note: Note) {
        let events: Vec<DomainEvent> = match note {
            Note::Progress(object, done) => {
                if !self.media_jobs.running.contains_key(&object) {
                    return;
                }
                self.translator.media_progress(&object, done)
            }
            Note::Sent(job, result) => {
                if self.media_jobs.running.remove(&job.object).is_none() {
                    return;
                }
                let events = match result {
                    Ok(answer) => self.translator.media_sent(&job, &answer, now()),
                    Err(error) => {
                        tracing_note(&job.object, &error);
                        self.translator.media_send_failed(&job, &media_reason(&error), now())
                    }
                };
                self.spawn_fetches(vec![
                    crate::plan::fetch::Fetch::Messages(job.context.clone()),
                    crate::plan::fetch::Fetch::first(crate::plan::fetch::Paged::Conversations),
                ]);
                events
            }
            Note::Imported(object, result) => {
                if self.media_jobs.running.remove(&object).is_none() {
                    return;
                }
                match result {
                    Ok((path, saved)) => self.translator.media_imported(&object, path, saved),
                    Err(error) => {
                        tracing_note(&object, &error);
                        self.translator.media_import_failed(&object, &media_reason(&error))
                    }
                }
            }
            Note::Copied(object, result) => match result {
                Ok(path) => self.translator.media_saved(&object, &path),
                Err(reason) => {
                    let failure = ramflux_desktop_core::EffectFailure {
                        method: ramflux_desktop_core::methods::OBJECT_SAVE.to_owned(),
                        code: ramflux_desktop_core::FailureCode::Daemon("io".to_owned()),
                        message: crate::translate::reason::keyed(key::FILE, &[("reason", &reason)]),
                    };
                    vec![DomainEvent::EffectFailed(failure)]
                }
            },
        };
        self.emit(events);
    }

    fn media_folder(&self, account: &str) -> Option<PathBuf> {
        media_folder(&self.config, account)
    }
}

/// The private media folder of `account`: beside the desktop settings
/// file, else in the desktop's state folder.
pub(super) fn media_folder(config: &crate::config::SdkConfig, account: &str) -> Option<PathBuf> {
    let base = config
        .settings_path
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .or_else(crate::local::paths::state_dir)?;
    Some(base.join("media").join(file_safe(account)))
}

/// Records why a transfer failed in the application log (no content).
fn tracing_note(object: &ObjectId, error: &BusError) {
    let code = media_code(error).unwrap_or_else(|| "untyped".to_owned());
    crate::local::app_log::warn(&format!("attachment {object} failed: {code}"));
}

/// A name made of letters, digits, `-` and `_` only.
pub(crate) fn file_safe(text: &str) -> String {
    text.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect()
}

fn request(account: &str, method: &str, body: Value) -> BusRequest {
    BusRequest::new(Some(account), "attachment", method.to_owned(), body)
}

/// Pins the account's attachment authority from its node, once; a pin
/// already made answers `duplicate_operation`.
async fn enable(client: &BusClient, account: &str) -> Result<(), BusError> {
    match client.call(&request(account, "attachment.enable", json!({ "from_node": true }))).await {
        Ok(_) => Ok(()),
        Err(error) if error.message().contains("duplicate_operation") => Ok(()),
        Err(error @ BusError::Daemon { .. })
            if error.reason_code() == Some("duplicate_operation") =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn io_error(error: &std::io::Error) -> BusError {
    BusError::daemon(
        "io",
        crate::translate::reason::keyed(key::FILE, &[("reason", &error.to_string())]),
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Streams `job`'s file into a prepare and sends it.
async fn send(
    client: &BusClient,
    account: &str,
    job: &SendJob,
    handle: &Mutex<Handle>,
    internal: &tokio::sync::mpsc::UnboundedSender<Internal>,
) -> Result<Value, BusError> {
    enable(client, account).await?;
    let mut begin = json!({
        "operation_id": format!("{}:0", job.operation),
        "file_name": job.name,
        "mime": job.mime,
    });
    match &job.group {
        Some(group) => {
            begin["kind"] = json!("group");
            begin["group_id"] = json!(group);
        }
        None => {
            begin["kind"] = json!("dm");
            begin["peer"] = json!(job.context);
        }
    }
    let begun = client.call(&request(account, "attachment.prepare.begin", begin)).await?;
    let prepare = begun["prepare_id"].as_str().unwrap_or_default().to_owned();
    let generation = begun["generation"].as_u64().unwrap_or(1);
    if prepare.is_empty() {
        return Err(BusError::daemon("layout_invalid", "the prepare has no id"));
    }
    *handle.lock() = Handle::Prepare { id: prepare.clone(), generation };
    if begun["state"].as_str() == Some("accepting") {
        let accepted = begun["accepted_through"].as_u64().unwrap_or(0);
        let chunk = begun["max_chunk"]
            .as_u64()
            .and_then(|c| usize::try_from(c).ok())
            .filter(|c| *c > 0)
            .map_or(CHUNK, |c| c.min(CHUNK));
        let mut file = std::fs::File::open(&job.path).map_err(|e| io_error(&e))?;
        let mut digest = ramflux_sdk::PlaintextDigester::new();
        let mut offset = 0_u64;
        let mut buffer = zeroize::Zeroizing::new(vec![0_u8; chunk]);
        loop {
            let read = file.read(&mut buffer).map_err(|e| io_error(&e))?;
            if read == 0 {
                break;
            }
            let bytes = &buffer[..read];
            digest.update(bytes);
            let end = offset + read as u64;
            if end > accepted {
                let skip = usize::try_from(accepted.saturating_sub(offset)).unwrap_or(0);
                let start = offset + skip as u64;
                let body = json!({
                    "prepare_id": prepare,
                    "generation": generation,
                    "offset": start,
                    "data_base64": ramflux_protocol::encode_base64url(&bytes[skip..]),
                });
                let chunk_request = request(account, "attachment.prepare.chunk", body);
                let result = client.call(&chunk_request).await;
                crate::bus::wipe(chunk_request.body);
                result?;
            }
            offset = end;
            let _ = internal
                .send(Internal::Media(Note::Progress(job.object.clone(), offset.min(job.size))));
        }
        let finish = json!({
            "prepare_id": prepare,
            "generation": generation,
            "plaintext_length": offset,
            "plaintext_digest": hex(&digest.finalize()),
        });
        client.call(&request(account, "attachment.prepare.finish", finish)).await?;
    }
    let mut body = json!({
        "conversation_id": job.context,
        "text": "",
        "prepares": [{ "prepare_id": prepare }],
    });
    if let Some(group) = &job.group {
        body["group_id"] = json!(group);
    }
    // Each attempt is its own send (a refused one is not replayed); a lost
    // answer is asked again under the same key by the client.
    let key = crate::bus::idempotency::fresh_key().map_err(|e| BusError::daemon("random", e))?;
    let send = BusRequest::new(Some(account), "conversation", "conversation.send", body)
        .with_operation_id(key);
    client.call(&send).await
}

/// Downloads `job`'s file into `folder` (and copies it where the user
/// chose); answers the private copy and the saved path.
async fn import(
    client: &BusClient,
    account: &str,
    job: &ImportJob,
    folder: &Path,
    handle: &Mutex<Handle>,
    internal: &tokio::sync::mpsc::UnboundedSender<Internal>,
) -> Result<(PathBuf, Option<String>), BusError> {
    enable(client, account).await?;
    let operation = crate::plan::message::fresh_id("desk-import-")
        .map_err(|e| BusError::daemon("random", e))?;
    let begun = client
        .call(&request(
            account,
            "attachment.import.begin",
            json!({ "message_id": job.message_id, "index": job.index, "operation_id": operation }),
        ))
        .await?;
    let id = begun["import_id"].as_str().unwrap_or_default().to_owned();
    if id.is_empty() {
        return Err(BusError::daemon("layout_invalid", "the import has no id"));
    }
    *handle.lock() = Handle::Import { id: id.clone() };
    let mut generation = begun["generation"].as_u64().unwrap_or(1);
    let mut length = begun["plaintext_length"].as_u64();
    let mut state = begun["state"].as_str().unwrap_or_default().to_owned();
    let mut delay = Duration::from_millis(100);
    while state != "ready" {
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(1));
        let status = client
            .call(&request(account, "attachment.import.status", json!({ "import_id": id })))
            .await?;
        generation = status["generation"].as_u64().unwrap_or(generation);
        length = status["plaintext_length"].as_u64().or(length);
        state = status["state"].as_str().unwrap_or_default().to_owned();
    }
    std::fs::create_dir_all(folder).map_err(|e| io_error(&e))?;
    restrict(folder, 0o700);
    let target = folder.join(file_safe(job.object.as_str()));
    let partial = folder.join(format!("{}.part", file_safe(job.object.as_str())));
    let written =
        copy_out(client, account, (&id, generation), &partial, &job.object, internal).await;
    let total = match written {
        Ok(total) => total,
        Err(error) => {
            let _ = std::fs::remove_file(&partial);
            return Err(error);
        }
    };
    if length.is_some_and(|l| l != total) {
        let _ = std::fs::remove_file(&partial);
        return Err(BusError::daemon("layout_invalid", "the download is not the file's length"));
    }
    std::fs::rename(&partial, &target).map_err(|e| io_error(&e))?;
    let _ = client
        .call(&request(account, "attachment.import.finish", json!({ "import_id": id })))
        .await;
    let saved = match &job.save_to {
        Some(to) => {
            std::fs::copy(&target, to).map_err(|e| io_error(&e))?;
            Some(to.display().to_string())
        }
        None => None,
    };
    Ok((target, saved))
}

/// Reads an import handle into `partial`; answers the bytes written.
async fn copy_out(
    client: &BusClient,
    account: &str,
    (id, generation): (&str, u64),
    partial: &Path,
    object: &ObjectId,
    internal: &tokio::sync::mpsc::UnboundedSender<Internal>,
) -> Result<u64, BusError> {
    let mut file = std::fs::File::create(partial).map_err(|e| io_error(&e))?;
    restrict(partial, 0o600);
    let mut offset = 0_u64;
    loop {
        let read = client
            .call(&request(
                account,
                "attachment.import.read",
                json!({ "import_id": id, "generation": generation, "offset": offset, "max_bytes": CHUNK }),
            ))
            .await?;
        let encoded = read["bytes"].as_str().unwrap_or_default();
        let bytes = zeroize::Zeroizing::new(
            ramflux_protocol::decode_base64url(encoded)
                .map_err(|_| BusError::daemon("layout_invalid", "the chunk is not base64url"))?,
        );
        let eof = read["eof"].as_bool().unwrap_or(false);
        if bytes.is_empty() && !eof {
            return Err(BusError::daemon("layout_invalid", "an empty chunk before the end"));
        }
        file.write_all(&bytes).map_err(|e| io_error(&e))?;
        offset += bytes.len() as u64;
        let _ = internal.send(Internal::Media(Note::Progress(object.clone(), offset)));
        if eof {
            file.sync_all().map_err(|e| io_error(&e))?;
            return Ok(offset);
        }
    }
}

#[cfg(unix)]
fn restrict(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) {}
