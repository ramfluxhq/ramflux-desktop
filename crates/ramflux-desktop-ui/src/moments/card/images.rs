// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Decoding of the pictures cards select (image slots).
//!
//! The bridge reads an image through the object API, checks it against the
//! slot's object, digest and the decoder limits, and keeps it in a private
//! file ([`ramflux_desktop_core::CardImage::Ready`]). This module decodes that
//! file, as PNG or JPEG only, within the README limits:
//!
//! - each side at most 4096 pixels and at most 4,194,304 pixels in all, from
//!   the picture's own header and again from what the decoder produced;
//! - at most 32 MiB decoded at once (the least recently drawn picture is
//!   released first);
//! - one decode in flight; the next waits for it.
//!
//! Nothing here reads a URL, a path or a data URI from a card tree.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use gpui_kit::{App, AppContext as _, Global, ImageFormat, RenderImage, Window};

/// The most bytes of an encoded picture.
const MAX_ENCODED: u64 = 8 * 1024 * 1024;
/// The largest side of a picture.
const MAX_SIDE: u32 = 4096;
/// The most pixels of a picture.
const MAX_PIXELS: u64 = 4_194_304;
/// The most decoded bytes held at once.
const MAX_DECODED: u64 = 32 * 1024 * 1024;

/// A picture's place in the cache.
enum Entry {
    /// Decoded, with its decoded size in bytes and when it was last drawn.
    Ready { image: Arc<RenderImage>, bytes: u64, used: u64 },
    /// Refused or not decodable.
    Failed,
}

/// What the cache says of a picture.
pub enum Decoded {
    /// The picture, ready to draw.
    Ready(Arc<RenderImage>),
    /// Waiting for its decode (this one's or the one in flight).
    Pending,
    /// The picture cannot be shown.
    Failed,
}

/// The decoded pictures of the window, by file.
#[derive(Default)]
struct Cache {
    entries: BTreeMap<String, Entry>,
    decoding: bool,
    clock: u64,
}

impl Global for Cache {}

impl Cache {
    fn decoded_bytes(&self) -> u64 {
        self.entries
            .values()
            .map(|entry| match entry {
                Entry::Ready { bytes, .. } => *bytes,
                Entry::Failed => 0,
            })
            .sum()
    }

    /// Releases the least recently drawn pictures until `bytes` more fit.
    fn make_room(&mut self, bytes: u64) {
        while self.decoded_bytes() + bytes > MAX_DECODED {
            let oldest = self
                .entries
                .iter()
                .filter_map(|(path, entry)| match entry {
                    Entry::Ready { used, .. } => Some((*used, path.clone())),
                    Entry::Failed => None,
                })
                .min();
            match oldest {
                Some((_, path)) => {
                    self.entries.remove(&path);
                }
                None => break,
            }
        }
    }
}

/// The codec of encoded picture bytes: PNG and JPEG only.
fn format(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some(ImageFormat::Png)
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(ImageFormat::Jpeg)
    } else {
        None
    }
}

/// Whether a picture of this size may be decoded.
fn within(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= MAX_SIDE
        && height <= MAX_SIDE
        && u64::from(width) * u64::from(height) <= MAX_PIXELS
}

/// Reads and decodes one file, refusing what is over the limits.
fn decode(path: &str, renderer: gpui_kit::SvgRenderer) -> Option<(Arc<RenderImage>, u64)> {
    let length = std::fs::metadata(path).ok()?.len();
    if length == 0 || length > MAX_ENCODED {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let codec = format(&bytes)?;
    let image = gpui_kit::Image::from_bytes(codec, bytes).to_image_data(renderer).ok()?;
    let size = image.size(0);
    let (width, height) = (u32::try_from(size.width.0).ok()?, u32::try_from(size.height.0).ok()?);
    within(width, height).then_some((image, u64::from(width) * u64::from(height) * 4))
}

/// The picture of `path` (`width` by `height`, as the bridge found it), for
/// drawing now. A picture not decoded yet starts its decode if none is in
/// flight; the window is drawn again when it ends.
pub fn lookup(path: &str, width: u32, height: u32, window: &mut Window, cx: &mut App) -> Decoded {
    let cache = cx.default_global::<Cache>();
    cache.clock = cache.clock.saturating_add(1);
    let now = cache.clock;
    match cache.entries.get_mut(path) {
        Some(Entry::Ready { image, used, .. }) => {
            *used = now;
            return Decoded::Ready(image.clone());
        }
        Some(Entry::Failed) => return Decoded::Failed,
        None => {}
    }
    if !within(width, height) {
        cache.entries.insert(path.to_owned(), Entry::Failed);
        return Decoded::Failed;
    }
    if cache.decoding {
        return Decoded::Pending;
    }
    cache.decoding = true;
    let path = path.to_owned();
    let renderer = cx.svg_renderer();
    let work = {
        let path = path.clone();
        cx.background_spawn(async move { decode(&path, renderer) })
    };
    window
        .spawn(cx, async move |cx| {
            let result = work.await;
            cx.update(|window, cx| {
                let cache = cx.default_global::<Cache>();
                cache.decoding = false;
                match result {
                    Some((image, bytes)) => {
                        cache.make_room(bytes);
                        cache.clock = cache.clock.saturating_add(1);
                        let used = cache.clock;
                        cache.entries.insert(path, Entry::Ready { image, bytes, used });
                    }
                    None => {
                        cache.entries.insert(path, Entry::Failed);
                    }
                }
                window.refresh();
            })
            .ok();
        })
        .detach();
    Decoded::Pending
}

/// Releases every picture whose file no card holds any more (its card was
/// disposed or forgotten).
pub fn retain(cx: &mut App, keep: &BTreeSet<String>) {
    let cache = cx.default_global::<Cache>();
    cache.entries.retain(|path, _| keep.contains(path));
}
