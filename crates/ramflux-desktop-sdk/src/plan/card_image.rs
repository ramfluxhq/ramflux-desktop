// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The read of an image slot's bytes (`card.image.load`).
//!
//! The runtime hands an image slot out as an object selector; the desktop
//! reads the bytes through the object API (see `crate::bridge::card_image`).
//! The job names the object and its digest as the held tree's slot gave them;
//! nothing here reads a URL, a path or a data URI from a card.

use std::path::PathBuf;

use serde_json::Value;

/// One image read.
#[derive(Debug, Clone)]
pub struct Job {
    /// The account the object belongs to.
    pub account: String,
    /// The object the slot selects.
    pub object_id: String,
    /// The manifest digest the slot names.
    pub digest: String,
    /// The transfer state the slot reported (`available` when the bytes are
    /// local already).
    pub transfer_state: String,
    /// The size the slot reported, when it did.
    pub size: Option<u64>,
    /// The private folder the verified file is kept in (set by the bridge).
    pub folder: PathBuf,
}

/// The job a `card.image.load` effect asks for; `None` when its body does not
/// follow the contract.
#[must_use]
pub fn job(account: Option<&str>, body: &Value) -> Option<Job> {
    Some(Job {
        account: account?.to_owned(),
        object_id: body["object_id"].as_str().filter(|id| !id.is_empty())?.to_owned(),
        digest: body["manifest_digest"].as_str().filter(|d| !d.is_empty())?.to_owned(),
        transfer_state: body["transfer_state"].as_str().unwrap_or("not_fetched").to_owned(),
        size: body["size"].as_u64(),
        folder: PathBuf::new(),
    })
}
