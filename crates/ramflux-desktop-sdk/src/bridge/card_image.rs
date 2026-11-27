// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Reads the bytes of a card's image slot through the object API.
//!
//! The runtime hands an image out as `{object_id, manifest_digest,
//! transfer_state, size, mime}`; no reply carries a URL, a path or bytes. The
//! host fetches the object when it is not local yet (`object.fetch`, waiting
//! at most [`WAIT`]), then reads it with `object.get.begin`, `.read` and
//! `.finish`, and checks what it read before anything draws it:
//!
//! - the object the daemon answers for is the one the slot names, and the
//!   directory row's manifest digest (when it has one) is the slot's;
//! - at most [`MAX_BYTES`] bytes, and their hash is the one `begin` named;
//! - a PNG or JPEG whose own header says each side is at most
//!   [`MAX_SIDE`] pixels and the whole at most [`MAX_PIXELS`].
//!
//! The verified bytes are kept in a private (0600) file; the answer names the
//! file and the size from its header. Any failure is one answer: the image is
//! unavailable. Neither bytes nor ids are logged.

use std::io::Write as _;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::bus::client::BusClient;
use crate::bus::{BusError, BusRequest};
use crate::plan::card_image::Job;

/// The most bytes read for one image.
const MAX_BYTES: usize = 8 * 1024 * 1024;
/// The largest side of a picture the decoder accepts.
const MAX_SIDE: u32 = 4096;
/// The most pixels of a picture the decoder accepts.
const MAX_PIXELS: u64 = 4_194_304;
/// How long a fetch is waited for.
const WAIT: Duration = Duration::from_secs(60);
/// The slice read at a time.
const CHUNK: usize = 256 * 1024;
/// The directory pages looked through for the object's row.
const DIRECTORY_PAGES: usize = 20;
/// The object protocol version of `object.get.begin`.
const PROTOCOL_VERSION: u32 = 1;

/// A picture format the decoder accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Png,
    Jpeg,
}

impl Format {
    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
        }
    }

    fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }
}

/// The format and size of a PNG or JPEG, read from its own header; `None`
/// for anything else or a header that does not hold.
pub(crate) fn picture(bytes: &[u8]) -> Option<(Format, u32, u32)> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        // The first chunk is IHDR: length, "IHDR", width, height.
        if bytes.get(12..16)? != b"IHDR" {
            return None;
        }
        let width = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
        let height = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
        return Some((Format::Png, width, height));
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return jpeg_size(bytes).map(|(width, height)| (Format::Jpeg, width, height));
    }
    None
}

/// The size from the first start-of-frame marker of a JPEG.
fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2;
    while at + 4 <= bytes.len() {
        if bytes[at] != 0xff {
            return None;
        }
        let marker = bytes[at + 1];
        if marker == 0xff {
            at += 1;
            continue;
        }
        // Markers without a length.
        if marker == 0x01 || (0xd0..=0xd8).contains(&marker) {
            at += 2;
            continue;
        }
        let length = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
        if length < 2 {
            return None;
        }
        // Start-of-frame markers (not DHT, JPG and DAC).
        if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
            let frame = bytes.get(at + 5..at + 9)?;
            let height = u32::from(u16::from_be_bytes([frame[0], frame[1]]));
            let width = u32::from(u16::from_be_bytes([frame[2], frame[3]]));
            return Some((width, height));
        }
        at += 2 + length;
    }
    None
}

/// Whether a picture of this size is one the decoder accepts.
pub(crate) fn within_limits(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= MAX_SIDE
        && height <= MAX_SIDE
        && u64::from(width) * u64::from(height) <= MAX_PIXELS
}

fn request(account: &str, method: &str, body: Value) -> BusRequest {
    BusRequest::new(Some(account), "object", method.to_owned(), body)
}

fn refused(reason: &str) -> BusError {
    BusError::daemon("card_image_unavailable", reason)
}

/// Reads, checks and keeps the image `job` names. The answer is
/// `{path, width, height, mime}`.
pub(super) async fn load(client: &BusClient, job: &Job) -> Result<Value, BusError> {
    if job.size.is_some_and(|size| size > MAX_BYTES as u64) {
        return Err(refused("too large"));
    }
    let operation = super::media::file_safe(&format!("{}-{}", job.object_id, job.digest));
    let operation: String = operation.chars().take(200).collect();
    if job.transfer_state != "available" {
        let fetched = client
            .call(&request(
                &job.account,
                "object.fetch",
                json!({ "object_id": job.object_id, "operation_id": format!("desk-card-{operation}") }),
            ))
            .await?;
        if fetched["transfer_state"].as_str() == Some("failed") {
            return Err(refused("fetch failed"));
        }
    }
    let bytes = read(client, job, &operation).await?;
    let (format, width, height) = picture(&bytes).ok_or_else(|| refused("not a picture"))?;
    if !within_limits(width, height) {
        return Err(refused("over the decoder limits"));
    }
    directory_matches(client, job).await?;
    let path = keep(&job.folder, &job.object_id, format, &bytes)?;
    Ok(json!({
        "path": path,
        "width": width,
        "height": height,
        "mime": format.mime(),
    }))
}

/// Reads the whole object through `object.get.begin`, `.read` and `.finish`,
/// waiting for a fetch that is still running. The bytes are the object the
/// slot names and match the hash `begin` reported.
async fn read(
    client: &BusClient,
    job: &Job,
    operation: &str,
) -> Result<Zeroizing<Vec<u8>>, BusError> {
    let read_operation = format!("desk-card-get-{operation}");
    let deadline = tokio::time::Instant::now() + WAIT;
    let mut delay = Duration::from_millis(250);
    let begin = loop {
        let begun = client
            .call(&request(
                &job.account,
                "object.get.begin",
                json!({
                    "object_id": job.object_id,
                    "operation_id": read_operation,
                    "protocol_version": PROTOCOL_VERSION,
                }),
            ))
            .await;
        match begun {
            Ok(begin) => break begin,
            Err(error) if tokio::time::Instant::now() < deadline && !error.is_connection_lost() => {
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(2));
            }
            Err(error) => return Err(error),
        }
    };
    if begin["object_id"].as_str().is_some_and(|id| id != job.object_id) {
        return Err(refused("another object"));
    }
    let total = begin["total_len"]
        .as_u64()
        .and_then(|len| usize::try_from(len).ok())
        .filter(|len| *len > 0 && *len <= MAX_BYTES)
        .ok_or_else(|| refused("size"))?;
    let expected = begin["plaintext_hash"].as_str().ok_or_else(|| refused("no hash"))?.to_owned();
    let mut bytes = Zeroizing::new(Vec::with_capacity(total));
    let mut hasher = ramflux_crypto::Blake3DomainHasher::new(ramflux_protocol::domain::OBJECT);
    while bytes.len() < total {
        let len = (total - bytes.len()).min(CHUNK);
        let slice = client
            .call(&request(
                &job.account,
                "object.get.read",
                json!({ "operation_id": read_operation, "offset": bytes.len(), "len": len }),
            ))
            .await?;
        let data = Zeroizing::new(
            ramflux_protocol::decode_base64url(slice["data_base64"].as_str().unwrap_or_default())
                .map_err(|_| refused("slice"))?,
        );
        if data.len() != len {
            return Err(refused("slice length"));
        }
        hasher.update(&data);
        bytes.extend_from_slice(&data);
    }
    // The daemon's spool is released whatever the check says.
    let _ = client
        .call(&request(
            &job.account,
            "object.get.finish",
            json!({ "operation_id": read_operation }),
        ))
        .await;
    if hasher.finalize_base64url() != expected {
        return Err(refused("hash"));
    }
    Ok(bytes)
}

/// The directory row of the object carries the manifest digest the slot
/// names. A row with another digest, or no row among the first pages, is a
/// refusal: the bytes are not drawn.
async fn directory_matches(client: &BusClient, job: &Job) -> Result<(), BusError> {
    let mut cursor: Option<String> = None;
    for _ in 0..DIRECTORY_PAGES {
        let mut body = json!({ "limit": 100 });
        if let Some(cursor) = &cursor {
            body["cursor"] = json!(cursor);
        }
        let page = client.call(&request(&job.account, "object.directory", body)).await?;
        if let Some(row) = page["objects"]
            .as_array()
            .and_then(|rows| rows.iter().find(|row| row["id"].as_str() == Some(&job.object_id)))
        {
            return match row["manifest_hash"].as_str() {
                Some(digest) if digest == job.digest => Ok(()),
                Some(_) => Err(refused("digest")),
                // A row the daemon keeps no digest for: the runtime already
                // checked the slot's digest against the conversation's own.
                None => Ok(()),
            };
        }
        match page["next_cursor"].as_str() {
            Some(next) => cursor = Some(next.to_owned()),
            None => break,
        }
    }
    Err(refused("no directory row"))
}

/// Keeps the verified bytes in a private file of `folder`, replacing an
/// earlier copy atomically; answers its path.
fn keep(folder: &Path, object: &str, format: Format, bytes: &[u8]) -> Result<String, BusError> {
    let io = |_: std::io::Error| refused("file");
    let folder = folder.join("card-images");
    std::fs::create_dir_all(&folder).map_err(io)?;
    super::media::restrict(&folder, 0o700);
    let name = super::media::file_safe(object);
    let target = folder.join(format!("{name}.{}", format.extension()));
    let partial = folder.join(format!("{name}.part"));
    let mut file = std::fs::File::create(&partial).map_err(io)?;
    super::media::restrict(&partial, 0o600);
    file.write_all(bytes).map_err(io)?;
    file.sync_all().map_err(io)?;
    std::fs::rename(&partial, &target).map_err(io)?;
    Ok(target.display().to_string())
}
