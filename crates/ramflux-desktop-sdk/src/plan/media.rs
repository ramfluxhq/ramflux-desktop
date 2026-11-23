// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Attachments on the prepared-media path: what to send, download, retry or
//! cancel, as a job the bridge runs step by step (see `bridge::media`).
//!
//! - `object.send` `{context_id, path}`: the file is streamed into the
//!   daemon with `attachment.prepare.begin`, `.chunk` (unpadded base64url,
//!   at most the chunk size `begin` answers) and `.finish` (length and the
//!   unkeyed BLAKE3 digest), then sent as one `conversation.send` naming the
//!   finished prepare (with the group for a group conversation). The
//!   message is shown at once, its file transferring; the bytes moved are
//!   reported as they go.
//! - `object.get` `{object_id}`: a received file is downloaded with
//!   `attachment.import.begin` (the message id the read model lists and the
//!   file's index in it), `.status` until ready, `.read` chunk by chunk into
//!   the desktop's private media folder, then `.finish`.
//! - `object.save` `{object_id, path}`: the downloaded copy is copied to the
//!   chosen path (downloaded first when it is not here yet).
//! - `object.transfer.resume` `{object_id}`: a failed send is sent again from
//!   the same file under the same operation (the daemon resumes the prepare
//!   it holds); a failed download starts again.
//! - `object.transfer.cancel` `{object_id}`: the running job stops and the
//!   daemon's prepare or import handle is cancelled.
//!
//! Both ends need the account's attachment authority pinned once
//! (`attachment.enable {from_node: true}`); a repeated pin answers
//! `duplicate_operation`, which counts as pinned.

use std::path::{Path, PathBuf};

use ramflux_desktop_core::{ContextId, ContextKind, ObjectId, methods};
use serde_json::Value;

use super::{Known, Plan, refuse, refuse_invalid, text};
use crate::capabilities;
use crate::translate::reason::{key, keyed};

/// The largest chunk the desktop streams (the daemon's own maximum).
pub const CHUNK: usize = 524_288;

/// The largest file the desktop offers to send, in bytes (the owner's
/// media cap for the first release).
pub const SEND_LIMIT: u64 = 200 * 1024 * 1024;

/// What the source knows about one attachment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaRef {
    /// A file this device is sending (or failed to send).
    Outgoing(Box<SendJob>),
    /// A file a listed message carries: the daemon's message id and the
    /// file's index in it.
    Listed {
        /// The read model's message id.
        message_id: String,
        /// Index in the message's media.
        index: u64,
    },
}

/// A send of one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendJob {
    /// The object shown on the message (`<message>#0`).
    pub object: ObjectId,
    /// The id this device gave the message.
    pub message: String,
    /// The conversation.
    pub context: ContextId,
    /// The group, for a group conversation.
    pub group: Option<String>,
    /// The chosen file.
    pub path: PathBuf,
    /// Its name as recipients see it.
    pub name: String,
    /// Its media type.
    pub mime: String,
    /// Its size in bytes.
    pub size: u64,
    /// The operation id of the send; the prepare is `<operation>:0`.
    pub operation: String,
}

/// A download of one received file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportJob {
    /// The object.
    pub object: ObjectId,
    /// The read model's message id.
    pub message_id: String,
    /// Index in the message's media.
    pub index: u64,
    /// Where the user chose to save it, besides the private copy.
    pub save_to: Option<PathBuf>,
}

/// One attachment job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    /// Prepare and send a file.
    Send(Box<SendJob>),
    /// Download a received file.
    Import(ImportJob),
    /// Copy a downloaded file to the chosen path.
    Copy {
        /// The object.
        object: ObjectId,
        /// The private copy.
        from: PathBuf,
        /// The chosen path.
        to: PathBuf,
    },
    /// Stop a running job and cancel its daemon handle.
    Cancel {
        /// The object.
        object: ObjectId,
    },
}

/// The object id of file `index` of message `message`.
pub fn object_id(message: &str, index: u64) -> ObjectId {
    ObjectId::new(format!("{message}#{index}"))
}

/// Plans `object.send`.
pub(super) fn send(body: &Value, known: &impl Known) -> Plan {
    let context = ContextId::new(text(body, "context_id"));
    let group = match known.context_kind(&context) {
        Some(ContextKind::Direct) => None,
        Some(ContextKind::Group) => match known.directory().and_then(|d| d.group_of(&context)) {
            Some(group) => Some(group.to_owned()),
            None => return refuse(key::NO_GROUP_LINK),
        },
        Some(kind) => {
            return refuse(
                capabilities::reason_for(&methods::in_context(methods::ATTACH_OBJECT, kind))
                    .unwrap_or(key::NO_ROUTE),
            );
        }
        None => return refuse_invalid("unknown conversation"),
    };
    let path = PathBuf::from(text(body, "path"));
    let size = match std::fs::metadata(&path) {
        Ok(meta) if !meta.is_file() => {
            return refuse(&keyed(key::FILE, &[("reason", "not a file")]));
        }
        Ok(meta) if meta.len() > SEND_LIMIT => {
            let limit = SEND_LIMIT.to_string();
            return refuse(&keyed(key::ATTACHMENT_TOO_LARGE, &[("limit", &limit)]));
        }
        Ok(meta) => meta.len(),
        Err(error) => return refuse(&keyed(key::FILE, &[("reason", &error.to_string())])),
    };
    let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
        return refuse(&keyed(key::FILE, &[("reason", "the file name is not readable")]));
    };
    let mime = match media_type(&path) {
        Ok(mime) => mime.to_owned(),
        Err(error) => return refuse(&keyed(key::FILE, &[("reason", &error.to_string())])),
    };
    let message = match super::message::fresh_id("desk-msg-") {
        Ok(id) => id,
        Err(error) => return refuse_invalid(&error),
    };
    let operation = match super::message::fresh_id("desk-media-") {
        Ok(id) => id,
        Err(error) => return refuse_invalid(&error),
    };
    Plan::Media(Job::Send(Box::new(SendJob {
        object: object_id(&message, 0),
        message,
        context,
        group,
        path,
        name,
        mime,
        size,
        operation,
    })))
}

/// Plans `object.get`, `object.save`, `object.transfer.resume` and
/// `object.transfer.cancel`.
pub(super) fn object_call(method: &str, body: &Value, known: &impl Known) -> Plan {
    let object = ObjectId::new(text(body, "object_id"));
    if object.as_str().is_empty() {
        return refuse_invalid("no object named");
    }
    if method == methods::OBJECT_CANCEL {
        return Plan::Media(Job::Cancel { object });
    }
    let save_to = (method == methods::OBJECT_SAVE).then(|| PathBuf::from(text(body, "path")));
    if save_to.as_ref().is_some_and(|p| p.as_os_str().is_empty()) {
        return refuse_invalid("no path chosen");
    }
    if let (Some(to), Some(from)) = (&save_to, known.media_file(&object)) {
        return Plan::Media(Job::Copy { object, from: from.to_path_buf(), to: to.clone() });
    }
    match known.media(&object) {
        Some(MediaRef::Outgoing(job)) if method == methods::OBJECT_RETRY => {
            Plan::Media(Job::Send(job.clone()))
        }
        Some(MediaRef::Listed { message_id, index }) => Plan::Media(Job::Import(ImportJob {
            object,
            message_id: message_id.clone(),
            index: *index,
            save_to,
        })),
        Some(MediaRef::Outgoing(_)) | None => refuse(key::NO_DIRECTION),
    }
}

/// The media type of a file: from its first bytes, else from its
/// extension, else `application/octet-stream`.
fn media_type(path: &Path) -> std::io::Result<&'static str> {
    let mut file = std::fs::File::open(path)?;
    let mut header = [0_u8; 16];
    let read = std::io::Read::read(&mut file, &mut header)?;
    let header = &header[..read];
    let sniffed = if header.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if header.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("image/png")
    } else if header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if header.starts_with(b"RIFF") && header.get(8..12) == Some(b"WEBP".as_slice()) {
        Some("image/webp")
    } else if header.starts_with(b"%PDF-") {
        Some("application/pdf")
    } else {
        None
    };
    if let Some(mime) = sniffed {
        return Ok(mime);
    }
    let extension =
        path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    Ok(match extension.as_str() {
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    })
}
