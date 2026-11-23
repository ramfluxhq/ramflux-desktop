// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Attachments as objects: the files a message carries, and what the
//! desktop did with them (sending, downloading, saved).
//!
//! A read-model row carries its files in `content.message.media` (generated
//! content: protobuf JSON, camelCase; the snake_case spelling is read too):
//! name, declared type, size and the large manifest's expiry. The read
//! model holds no key slot, grant or read capability; a file is opened
//! through the daemon's importer only. Each file becomes an object named
//! `<message>#<index>` (the message id this device gave a message it sent,
//! else the daemon's) and a media row of its message:
//!
//! | file | object |
//! |---|---|
//! | a send or download running here | transferring, with the bytes moved |
//! | downloaded here (or sent from this device) | complete, with the private copy |
//! | received, not downloaded, still in its window | queued (Download) |
//! | received, its window ended | failed: no longer available |
//! | this account's own, sent from another device | complete |
//!
//! A file being sent is shown before the daemon lists its message: the
//! message (receipt "sending") carries it as a media row and its object
//! counts the bytes streamed into the daemon. A refusal marks both failed
//! with the daemon's reason (a missing home node of the contact says to
//! exchange contact cards).

use std::path::PathBuf;

use ramflux_desktop_core::{
    ContextId, Direction, DomainEvent, Lifecycle, MediaState, MediaView, MessagePayload, Moment,
    MomentKind, Object, ObjectId, Receipt, Timestamp, TransferState,
};
use serde_json::Value;

use super::Translator;
use super::conversation::message_moment_id;
use super::reason::{key, keyed};
use crate::bus::BusError;
use crate::plan::media::{MediaRef, SendJob, object_id};

/// Refusal codes of the attachment API the desktop explains.
const KNOWN_CODES: [&str; 16] = [
    "authority_unavailable",
    "file_too_large",
    "manifest_too_large",
    "quota_exceeded",
    "disk_full",
    "peer_upgrade_required",
    "recipient_limit",
    "recipient_empty",
    "not_grantee",
    "upload_rejected",
    "expired",
    "cancelled",
    "attachment_deleted",
    "large_attachments",
    "digest_mismatch",
    "seal_failed",
];

/// The attachment refusal code of a failure: from the typed reason, the
/// error code, `details.code`, or a known code at the start of the message.
pub fn media_code(error: &BusError) -> Option<String> {
    let BusError::Daemon { code, message, reason_code, details, .. } = error else {
        return None;
    };
    let candidates = [
        reason_code.as_deref(),
        Some(code.as_str()),
        details.as_ref().and_then(|d| d["code"].as_str()),
        details.as_ref().and_then(|d| d["reason"].as_str()),
        message.split([':', ' ']).next(),
    ];
    candidates.into_iter().flatten().find(|c| KNOWN_CODES.contains(c)).map(str::to_owned)
}

/// The reason shown for an attachment that failed with `error`.
pub fn media_reason(error: &BusError) -> String {
    match media_code(error).as_deref() {
        Some("authority_unavailable") => key::MEDIA_NO_HOME.to_owned(),
        Some("expired") => key::MEDIA_EXPIRED.to_owned(),
        Some("attachment_deleted") => key::MEDIA_DELETED.to_owned(),
        Some("cancelled") => key::MEDIA_CANCELLED.to_owned(),
        Some("peer_upgrade_required") => key::MEDIA_PEER_UPGRADE.to_owned(),
        Some(code) => keyed(key::MEDIA_REFUSED, &[("code", code)]),
        None => error.message(),
    }
}

/// A string or number field of generated JSON (64-bit integers are
/// strings there).
fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

/// A field under its camelCase or snake_case name.
fn either<'a>(value: &'a Value, camel: &str, snake: &str) -> &'a Value {
    match &value[camel] {
        Value::Null => &value[snake],
        found => found,
    }
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Translator {
    /// The media rows of a read-model row whose message this device names
    /// `message` (the daemon's id is `daemon`), with their objects.
    pub(super) fn row_media(
        &mut self,
        message: &str,
        daemon: &str,
        own: bool,
        row: &Value,
    ) -> (Vec<MediaView>, Vec<DomainEvent>) {
        let items = match either(&row["content"], "message", "message") {
            Value::Object(_) => either(&row["content"]["message"], "media", "media"),
            _ => &Value::Null,
        };
        let Some(items) = items.as_array() else { return (Vec::new(), Vec::new()) };
        let mut views = Vec::new();
        let mut events = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let index = u64::try_from(index).unwrap_or(u64::MAX);
            let id = object_id(message, index);
            let name = either(item, "fileName", "file_name").as_str().unwrap_or_default();
            let mime = either(item, "mimeType", "mime_type").as_str().unwrap_or_default();
            let manifest = either(item, "largeManifest", "large_manifest");
            let size = number(&item["size"])
                .or_else(|| number(either(manifest, "plaintextLength", "plaintext_length")));
            let expires = number(either(manifest, "expiresAt", "expires_at")).filter(|at| *at > 0);
            if !matches!(self.media.get(&id), Some(MediaRef::Outgoing(_))) {
                self.media
                    .insert(id.clone(), MediaRef::Listed { message_id: daemon.to_owned(), index });
            }
            // A file downloaded before (another run of the desktop) is in
            // the private media folder under its object id.
            if !self.media_files.contains_key(&id)
                && let Some(kept) = self
                    .media_folder
                    .as_ref()
                    .map(|dir| dir.join(crate::bridge::file_safe(id.as_str())))
                    .filter(|path| path.is_file())
            {
                self.media_files.insert(id.clone(), kept);
            }
            let local = self.media_files.get(&id).map(|p| p.display().to_string());
            let known = self.objects.get(&id);
            let running = known.filter(|o| {
                matches!(o.transfer, TransferState::Transferring { .. })
                    || (matches!(o.transfer, TransferState::Failed { .. }) && local.is_none())
            });
            let transfer = match (running, &local) {
                (Some(object), _) => object.transfer.clone(),
                (None, Some(_)) => TransferState::Complete,
                (None, None) if own => TransferState::Complete,
                (None, None) if expires.is_some_and(|at| at <= now_seconds()) => {
                    TransferState::Failed { reason: key::MEDIA_EXPIRED.to_owned() }
                }
                (None, None) => TransferState::Queued,
            };
            let object = Object {
                id: id.clone(),
                name: (!name.is_empty()).then(|| name.to_owned()),
                size,
                direction: if own { Direction::Outbound } else { Direction::Inbound },
                transfer,
                saved_to: known.and_then(|o| o.saved_to.clone()),
                preview: None,
                mime: (!mime.is_empty()).then(|| mime.to_ascii_lowercase()),
                local_path: local,
            };
            events.extend(self.put_object(object));
            views.push(MediaView {
                object_id: id,
                name: name.to_owned(),
                mime: mime.to_ascii_lowercase(),
                size: size.unwrap_or(0),
                state: MediaState::Available,
                thumbnail: None,
            });
        }
        (views, events)
    }

    /// A send is starting: its message (receipt "sending") with the file
    /// as its media row, and the file's object, transferring.
    pub fn media_sending(&mut self, job: &SendJob, now: Timestamp) -> Vec<DomainEvent> {
        self.media.insert(job.object.clone(), MediaRef::Outgoing(Box::new(job.clone())));
        self.media_files.insert(job.object.clone(), job.path.clone());
        self.sending.insert(job.message.clone());
        let object = Object {
            id: job.object.clone(),
            name: Some(job.name.clone()),
            size: Some(job.size),
            direction: Direction::Outbound,
            transfer: TransferState::Transferring { transferred: 0 },
            saved_to: None,
            preview: None,
            mime: Some(job.mime.clone()),
            local_path: Some(job.path.display().to_string()),
        };
        let mut events = self.put_object(object);
        events.extend(self.media_message(job, Receipt::Sending, now));
        events
    }

    fn media_message(
        &mut self,
        job: &SendJob,
        receipt: Receipt,
        now: Timestamp,
    ) -> Option<DomainEvent> {
        let moment = Moment {
            order: None,
            id: message_moment_id(&job.context, &job.message),
            context_id: job.context.clone(),
            origin: self.own_origin(),
            at: now,
            lifecycle: match receipt {
                Receipt::Failed { .. } => Lifecycle::Failed,
                Receipt::Sending => Lifecycle::InProgress,
                _ => Lifecycle::Done,
            },
            kind: MomentKind::Message(MessagePayload {
                receipt,
                media: vec![MediaView {
                    object_id: job.object.clone(),
                    name: job.name.clone(),
                    mime: job.mime.clone(),
                    size: job.size,
                    state: MediaState::Available,
                    thumbnail: None,
                }],
                ..Default::default()
            }),
        };
        self.report_moment(moment)
    }

    /// Bytes moved for an attachment's transfer.
    pub fn media_progress(&mut self, object: &ObjectId, transferred: u64) -> Vec<DomainEvent> {
        let Some(mut known) = self.objects.get(object).cloned() else { return Vec::new() };
        known.transfer = TransferState::Transferring { transferred };
        self.put_object(known)
    }

    /// The daemon answered a send (`conversation.send`): submitted, or
    /// failed with the first delivery's refusal.
    pub fn media_sent(
        &mut self,
        job: &SendJob,
        answer: &Value,
        now: Timestamp,
    ) -> Vec<DomainEvent> {
        if let Some(daemon) = answer["message_id"].as_str() {
            self.sent_as(&job.message, daemon);
        }
        self.sending.remove(&job.message);
        if answer["delivery_state"].as_str() == Some("submitted") {
            let mut events = Vec::new();
            if let Some(mut object) = self.objects.get(&job.object).cloned() {
                object.transfer = TransferState::Complete;
                events.extend(self.put_object(object));
            }
            events.extend(self.media_message(job, Receipt::Sent, now));
            return events;
        }
        let code = answer["deliveries"]
            .as_array()
            .and_then(|rows| rows.iter().find_map(|row| row["error_code"].as_str()))
            .unwrap_or("upload_rejected");
        let reason = media_reason(&BusError::daemon(code, code));
        self.media_send_failed(job, &reason, now)
    }

    /// A send failed with `reason`: the message and its file show it.
    pub fn media_send_failed(
        &mut self,
        job: &SendJob,
        reason: &str,
        now: Timestamp,
    ) -> Vec<DomainEvent> {
        self.sending.remove(&job.message);
        let mut events = Vec::new();
        if let Some(mut object) = self.objects.get(&job.object).cloned() {
            object.transfer = TransferState::Failed { reason: reason.to_owned() };
            events.extend(self.put_object(object));
        }
        // Not the message's own retry (a text send): the file row has its
        // Retry, which sends the file again.
        events.extend(self.media_message(job, Receipt::Failed { retryable: false }, now));
        events
    }

    /// A download failed with `reason`.
    pub fn media_import_failed(&mut self, object: &ObjectId, reason: &str) -> Vec<DomainEvent> {
        let Some(mut known) = self.objects.get(object).cloned() else { return Vec::new() };
        known.transfer = TransferState::Failed { reason: reason.to_owned() };
        self.put_object(known)
    }

    /// A download finished into the private copy at `path`; `saved` is
    /// where the user chose to save it too.
    pub fn media_imported(
        &mut self,
        object: &ObjectId,
        path: PathBuf,
        saved: Option<String>,
    ) -> Vec<DomainEvent> {
        let Some(mut known) = self.objects.get(object).cloned() else { return Vec::new() };
        known.transfer = TransferState::Complete;
        known.local_path = Some(path.display().to_string());
        if saved.is_some() {
            known.saved_to = saved;
        }
        self.media_files.insert(object.clone(), path);
        self.put_object(known)
    }

    /// A downloaded file was copied to `path`, where the user chose.
    pub fn media_saved(&mut self, object: &ObjectId, path: &str) -> Vec<DomainEvent> {
        let Some(mut known) = self.objects.get(object).cloned() else { return Vec::new() };
        known.saved_to = Some(path.to_owned());
        self.put_object(known)
    }

    /// Where this account's downloaded files are kept.
    pub fn set_media_folder(&mut self, folder: Option<std::path::PathBuf>) {
        self.media_folder = folder;
    }

    /// Emits the object when it differs from what was reported.
    pub(super) fn put_object(&mut self, object: Object) -> Vec<DomainEvent> {
        if self.objects.get(&object.id) == Some(&object) {
            return Vec::new();
        }
        self.objects.insert(object.id.clone(), object.clone());
        vec![DomainEvent::ObjectUpserted(object)]
    }

    /// The context of an attachment this device is sending.
    pub fn media_context(&self, object: &ObjectId) -> Option<ContextId> {
        match self.media.get(object) {
            Some(MediaRef::Outgoing(job)) => Some(job.context.clone()),
            _ => None,
        }
    }
}
