// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The compiled catalogues: `build.rs` turns each `locales/<tag>.json` into a
//! static table of its translated entries, sorted by key. A key that is not
//! translated yet (`null` in the file) is absent, so a lookup falls through
//! to the next locale of the chain.

include!(concat!(env!("OUT_DIR"), "/catalog.rs"));

/// The translated entries of catalogue `tag` (empty for an unknown tag).
pub(crate) fn entries(tag: &str) -> &'static [(&'static str, &'static str)] {
    CATALOGUES.iter().find(|(t, _)| *t == tag).map_or(&[], |(_, entries)| entries)
}

/// The text of `key` in catalogue `tag`, if it is translated there.
pub(crate) fn get(tag: &str, key: &str) -> Option<&'static str> {
    let entries = entries(tag);
    entries
        .binary_search_by(|(k, _)| (*k).cmp(key))
        .ok()
        .and_then(|i| entries.get(i))
        .map(|(_, t)| *t)
}
