// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The daemon's message read model (`message.read` with `read_model: true`,
//! schema `ramflux.message.read_model.v1`) for the fake daemons: stored
//! message records, as the fakes keep them, served as one newest-first page.
//!
//! A record names its `message_id`, its `sender_id` (a device), its text
//! (`body_utf8`, else the body bytes), `created_at` in seconds, `deleted`
//! its `receipts` and, for another kind than a message, its
//! `content_type`. A record from `own_device` is this account's own.

use serde_json::{Value, json};

/// The read-model page of `conversation` holding `records` (oldest first,
/// as the fakes append them).
#[must_use]
pub fn page(conversation: &str, records: &[Value], own_device: &str) -> Value {
    let rows: Vec<Value> = records.iter().rev().map(|record| row(record, own_device)).collect();
    json!({
        "schema": "ramflux.message.read_model.v1",
        "conversation_id": conversation,
        "messages": rows,
        "next_before": null,
        "snapshot_sequence": records.len(),
        "read_through_message_id": null,
        "first_unread_message_id": null,
    })
}

/// One record as a read-model row.
#[must_use]
pub fn row(record: &Value, own_device: &str) -> Value {
    let sender = record["sender_id"].as_str().unwrap_or_default();
    let deleted = record["deleted"].as_bool().unwrap_or(false);
    let text = record["body_utf8"].as_str().map(str::to_owned).or_else(|| {
        let bytes: Option<Vec<u8>> = record["encrypted_body"]
            .as_array()?
            .iter()
            .map(|b| b.as_u64().and_then(|v| u8::try_from(v).ok()))
            .collect();
        bytes.and_then(|b| String::from_utf8(b).ok())
    });
    let at_ms = record["created_at"].as_i64().unwrap_or(0).saturating_mul(1000);
    json!({
        "id": record["message_id"],
        "conversation_id": record["conversation_id"],
        "author": { "account_anchor": "", "device_id": sender, "device_epoch": 0 },
        "device_sequence": "00000000000000000000",
        "hlc_ms": at_ms,
        "hlc_logical": 0,
        "group_id": null,
        "journal_sequence": 0,
        "is_own": sender == own_device,
        "state": if deleted { "tombstone" } else { "content" },
        "kind": if deleted { "tombstone" } else { record["content_type"].as_str().unwrap_or("message") },
        "text": if deleted { Value::Null } else { json!(text) },
        "content": null,
        "unsupported": null,
        "erasure": null,
        "revision": 0,
        "edited": false,
        "reactions": [],
        "receipts": record["receipts"].as_array().cloned().unwrap_or_default(),
    })
}
