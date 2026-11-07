// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Media rows of a v1 message.
//!
//! A row carries the file facts the desktop shows (name, type, size) and
//! never a key slot, read capability or manifest. An inline thumbnail is
//! shown only under the local opt-in (off by default) and only after the
//! cheap checks, in this order: opt-in, codec, byte size, width, height. The
//! bytes are decoded (header-checked against the declared codec) last, so a
//! thumbnail that fails an earlier check is never decoded and its bytes are
//! not kept. An object refusal later turns the row `Revoked` (the typed
//! revoke refusal) or `Unavailable` (any other failure) and drops what was
//! kept of the object.

use ramflux_desktop_core::{
    DomainEvent, MediaState, MediaView, MessagePayload, MomentId, MomentKind, ObjectId, Thumbnail,
};
use ramflux_protocol::message::{MediaRef, Thumbnail as WireThumbnail};

use crate::bus::BusError;
use crate::translate::Translator;

/// The longest inline thumbnail, in bytes.
pub const THUMBNAIL_MAX_BYTES: usize = 16_384;
/// The longest side of an inline thumbnail, in pixels.
pub const THUMBNAIL_MAX_SIDE: u32 = 512;
/// The typed reason the daemon gives when an object's access was revoked.
pub const REVOKED_REASON: &str = "object_revoked";

/// Codecs a thumbnail may use.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Codec {
    Png,
    Jpeg,
    Webp,
}

pub(super) fn views(translator: &mut Translator, media: &[MediaRef]) -> Vec<MediaView> {
    media
        .iter()
        .map(|reference| MediaView {
            object_id: ObjectId::new(&reference.object_id),
            name: reference.file_name.clone().unwrap_or_default(),
            mime: reference.mime_type.clone(),
            size: reference.size,
            state: MediaState::Available,
            thumbnail: reference
                .inline_thumbnail
                .as_ref()
                .and_then(|thumbnail| accepted(translator, thumbnail)),
        })
        .collect()
}

/// The thumbnail to show, or `None` for a plain file row.
fn accepted(translator: &mut Translator, wire: &WireThumbnail) -> Option<Thumbnail> {
    if !translator.inline_thumbnails {
        return None;
    }
    let codec = codec_of(&wire.mime_type)?;
    if wire.encoded_image.len() > THUMBNAIL_MAX_BYTES {
        return None;
    }
    if !(1..=THUMBNAIL_MAX_SIDE).contains(&wire.width) {
        return None;
    }
    if !(1..=THUMBNAIL_MAX_SIDE).contains(&wire.height) {
        return None;
    }
    decode(translator, codec, &wire.encoded_image).then(|| Thumbnail {
        width: wire.width,
        height: wire.height,
        bytes: wire.encoded_image.clone(),
    })
}

fn codec_of(mime: &str) -> Option<Codec> {
    match mime {
        "image/png" => Some(Codec::Png),
        "image/jpeg" => Some(Codec::Jpeg),
        "image/webp" => Some(Codec::Webp),
        _ => None,
    }
}

/// Checks that the bytes are the declared codec (its signature); the image
/// itself is decoded only when it is painted.
fn decode(translator: &mut Translator, codec: Codec, bytes: &[u8]) -> bool {
    #[cfg(feature = "test-support")]
    {
        translator.thumbnail_decodes += 1;
    }
    #[cfg(not(feature = "test-support"))]
    let _ = &translator;
    match codec {
        Codec::Png => bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]),
        Codec::Jpeg => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        Codec::Webp => bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
    }
}

/// Whether `failure` is the typed revoke refusal.
fn revoked(failure: &BusError) -> bool {
    failure.reason_code() == Some(REVOKED_REASON)
}

/// An object could not be fetched: its rows become `Revoked` for the typed
/// revoke refusal, `Unavailable` for any other failure, and keep no
/// thumbnail.
pub fn refuse(
    translator: &mut Translator,
    object_id: &str,
    failure: &BusError,
) -> Vec<DomainEvent> {
    let object = ObjectId::new(object_id);
    let state = if revoked(failure) { MediaState::Revoked } else { MediaState::Unavailable };
    let affected: Vec<MomentId> = translator
        .media_moments
        .iter()
        .filter(|(_, moment)| rows(&moment.kind).iter().any(|row| row.object_id == object))
        .map(|(id, _)| id.clone())
        .collect();
    affected
        .into_iter()
        .filter_map(|id| {
            let mut moment = translator.media_moments.get(&id)?.clone();
            if let MomentKind::Message(payload) = &mut moment.kind {
                for row in payload.media.iter_mut().filter(|row| row.object_id == object) {
                    row.state = state;
                    row.thumbnail = None;
                }
            }
            translator.media_moments.insert(id, moment.clone());
            translator.report_moment(moment)
        })
        .collect()
}

fn rows(kind: &MomentKind) -> &[MediaView] {
    match kind {
        MomentKind::Message(MessagePayload { media, .. }) => media,
        _ => &[],
    }
}

#[cfg(feature = "test-support")]
impl Translator {
    /// How many thumbnails were decoded so far (a test probe).
    pub fn thumbnail_decodes(&self) -> usize {
        self.thumbnail_decodes
    }

    /// Thumbnail bytes kept for a message's media rows (a test probe).
    pub fn cached_thumbnail_bytes(&self, moment: &MomentId) -> usize {
        self.media_moments.get(moment).map_or(0, |m| {
            rows(&m.kind)
                .iter()
                .filter_map(|row| row.thumbnail.as_ref())
                .map(|t| t.bytes.len())
                .sum()
        })
    }
}
