// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The fake daemon's agent surfaces: what `rfd` holds and answers about
//! surfaces, on the protocol's own state machine.
//!
//! An author's surface events enter as the protocol's wire bytes and are
//! applied with the protocol's [`SurfaceState::apply`] (version, base hash,
//! limits, resulting hash, stream finality), so every hash and stream state
//! the fake reports is the protocol's. Surfaces are scoped by conversation,
//! author and surface id (`surface_ref`, derived as the daemon derives it).
//! The fake answers as the pinned daemon (K3b as merged, 6fcf0e9) does:
//!
//! - `surface.get` / `surface.list`: the view with `streams` (`open` /
//!   `final` per text component), `posted_at` / `created_at` /
//!   `updated_at` (the author's seconds; a fake clock that advances 30 s per
//!   event, or null for an undated surface);
//! - `surface.action`: validated with the protocol's `validate_action` on
//!   the current state (the daemon's preflight); a refusal carries the
//!   daemon's `reason_code`;
//! - `surface.get` `actions[]`: this account's actions once the author's
//!   creator receipt for them arrived ([`FakeSurfaces::deliver_receipts`]),
//!   as the K3b daemon lists them: `action_receipt_id`,
//!   `receipt_verified: true`, the sender, version, the action's
//!   `wire_base64` and its signed `task_binding` (or null); the view's
//!   `action_receipt_id` / metadata `last_action_id` name the latest.
//!   Before its receipt, an action bound to a task step is listed as the
//!   requester's own retained outgoing action (`receipt_verified: false`,
//!   sender [`FakeSurfaces::acting_as`]), under the same receipt id;
//! - `surface.request_resync`: `not_found` / `not_out_of_sync` error
//!   frames, one request per surface per 60-second window (a second one is
//!   answered `rate_limited`), else submitted to the author's device.
//!
//! What a test publishes as `surface.changed` is the metadata
//! [`FakeSurfaces::meta`] returns: ids, version, hash, flags, streams and
//! times, never content; a removed surface's metadata is
//! `{surface_id, surface_ref, removed: true}`.

use std::collections::BTreeMap;
use std::sync::Mutex;

use ramflux_protocol::{A2uiConversationEvent, A2uiSurfaceAction, SurfaceReject, SurfaceState};
use serde_json::{Value, json};

use crate::fake_bus::Reply;

/// One surface as the fake daemon holds it.
#[derive(Debug, Clone)]
struct Held {
    conversation: String,
    author: String,
    surface_id: String,
    state: SurfaceState,
    out_of_sync: bool,
    renderer_error: Value,
    posted_at: Option<i64>,
    updated_at: Option<i64>,
}

/// An action the fake accepted, as the author would receive it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Acted {
    /// The surface's scoped reference.
    pub surface_ref: String,
    /// The version the action was taken on.
    pub version: u64,
    /// The action id.
    pub action_id: String,
    /// The author the request named.
    pub author: String,
}

#[derive(Debug, Default)]
struct Inner {
    surfaces: BTreeMap<String, Held>,
    acted: Vec<Acted>,
    resyncs: Vec<String>,
    /// Surfaces asked for in the current rate window.
    window: Vec<String>,
    /// The author's clock (seconds), `None` for undated carriers.
    clock: Option<i64>,
    removed: Vec<String>,
    /// Accepted actions, by surface, with their receipt once delivered.
    actions: Vec<HeldAction>,
    /// This account's principal (the sender of its retained actions).
    acting: Option<String>,
}

/// An accepted action and the author's receipt for it.
#[derive(Debug, Clone)]
struct HeldAction {
    /// The delivery receipt id (the same before and after the receipt).
    id: String,
    surface_ref: String,
    message_id: Value,
    version: u64,
    wire_base64: String,
    /// The receipt id and the action's sender, once the receipt arrived.
    receipt: Option<(String, String)>,
}

/// The fake's first author time (seconds).
pub const FIRST_AUTHOR_TIME: i64 = 1_790_000_000;
/// Seconds the author's clock advances per event.
pub const AUTHOR_TICK: i64 = 30;

/// Agent surfaces of the fake daemon (one account).
#[derive(Debug)]
pub struct FakeSurfaces {
    inner: Mutex<Inner>,
}

impl Default for FakeSurfaces {
    fn default() -> Self {
        let inner = Inner { clock: Some(FIRST_AUTHOR_TIME), ..Inner::default() };
        Self { inner: Mutex::new(inner) }
    }
}

/// The daemon's scoped reference of a surface.
pub fn surface_ref(conversation: &str, author: &str, surface_id: &str) -> String {
    let bytes = ramflux_protocol::canonical_json_bytes(&(conversation, author, surface_id))
        .unwrap_or_default();
    ramflux_protocol::hash_base64url("ramflux.surface.identity.v1", &bytes)
}

fn command_surface_id(event: &A2uiConversationEvent) -> String {
    use ramflux_protocol::A2uiConversationCommand as C;
    match event.command() {
        C::CreateSurface(c) => c.surface_id.clone(),
        C::UpdateComponents(c) => c.surface_id.clone(),
        C::UpdateDataModel(c) => c.surface_id.clone(),
        C::DeleteSurface(c) => c.surface_id.clone(),
        C::PatchSurface(c) => c.surface_id.clone(),
        C::StreamAppend(c) => c.surface_id.clone(),
    }
}

impl FakeSurfaces {
    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("fake surfaces")
    }

    /// Applies an author's event (original wire bytes) and returns the
    /// surface's reference.
    ///
    /// # Errors
    ///
    /// Returns the protocol's refusal; the surface is then unchanged (its
    /// renderer error records the refusal, as the daemon does).
    pub fn author_event(&self, author: &str, wire: &[u8]) -> Result<String, SurfaceReject> {
        let event =
            A2uiConversationEvent::from_wire_bytes(wire).map_err(|_| SurfaceReject::InvalidJson)?;
        let id = command_surface_id(&event);
        let reference = surface_ref(event.conversation_id(), author, &id);
        let mut inner = self.inner();
        let now = inner.clock;
        inner.clock = now.map(|t| t + AUTHOR_TICK);
        let held = inner.surfaces.entry(reference.clone()).or_insert_with(|| Held {
            conversation: event.conversation_id().to_owned(),
            author: author.to_owned(),
            surface_id: id,
            state: SurfaceState::default(),
            out_of_sync: false,
            renderer_error: Value::Null,
            posted_at: now,
            updated_at: now,
        });
        match held.state.apply(&event) {
            Ok(next) => {
                held.state = next;
                held.renderer_error = Value::Null;
                held.updated_at = now;
                Ok(reference)
            }
            Err(reject) => {
                if reject == SurfaceReject::ResultHashMismatch {
                    // The receiver's state differs from the author's.
                    held.out_of_sync = true;
                }
                held.renderer_error = json!({ "code": "renderer_error", "reject": reject });
                Err(reject)
            }
        }
    }

    /// Carriers from now on carry no author time (old carriers).
    pub fn undated(&self) {
        self.inner().clock = None;
    }

    /// The daemon drops a surface (its metadata then says `removed`).
    pub fn remove(&self, reference: &str) {
        let mut inner = self.inner();
        inner.surfaces.remove(reference);
        inner.removed.push(reference.to_owned());
    }

    /// The surface was asked for in this rate window already (by another
    /// device of the account, or before a restart).
    pub fn asked_recently(&self, reference: &str) {
        self.inner().window.push(reference.to_owned());
    }

    /// A new rate window: each surface may be asked for once more.
    pub fn next_rate_window(&self) {
        self.inner().window.clear();
    }

    /// Marks a surface out of sync (the daemon missed an author's event) or
    /// back in sync.
    pub fn set_out_of_sync(&self, reference: &str, out_of_sync: bool) {
        if let Some(held) = self.inner().surfaces.get_mut(reference) {
            held.out_of_sync = out_of_sync;
        }
    }

    /// The author's creator receipts for every action accepted so far
    /// arrive; the actions were sent by `sender` (this account's
    /// principal). Returns the surfaces whose actions got one.
    pub fn deliver_receipts(&self, sender: &str) -> Vec<String> {
        let mut inner = self.inner();
        let mut touched = Vec::new();
        for action in inner.actions.iter_mut().filter(|a| a.receipt.is_none()) {
            action.receipt = Some((action.id.clone(), sender.to_owned()));
            if !touched.contains(&action.surface_ref) {
                touched.push(action.surface_ref.clone());
            }
        }
        touched
    }

    /// The principal of this account: its retained task-bound actions are
    /// listed as sent by it before their receipt.
    pub fn acting_as(&self, principal: &str) {
        self.inner().acting = Some(principal.to_owned());
    }

    /// Actions accepted so far.
    pub fn acted(&self) -> Vec<Acted> {
        self.inner().acted.clone()
    }

    /// Update requests sent to authors so far (surface references).
    pub fn resyncs(&self) -> Vec<String> {
        self.inner().resyncs.clone()
    }

    /// The current version and hash of a surface.
    pub fn current(&self, reference: &str) -> Option<(u64, String)> {
        self.inner()
            .surfaces
            .get(reference)
            .map(|h| (h.state.version(), h.state.hash().unwrap_or_default().to_owned()))
    }

    /// The `surface.changed` body of a surface (metadata only).
    pub fn meta(&self, reference: &str) -> Value {
        let inner = self.inner();
        match inner.surfaces.get(reference) {
            Some(held) => inner.meta(reference, held),
            None if inner.removed.iter().any(|r| r == reference) => {
                json!({ "surface_id": reference, "surface_ref": reference, "removed": true })
            }
            None => Value::Null,
        }
    }

    /// Answers the surface methods; `None` for any other method.
    pub fn handle(&self, method: &str, body: &Value) -> Option<Reply> {
        let text = |field: &str| body[field].as_str().unwrap_or_default().to_owned();
        Some(match method {
            "surface.get" => {
                let reference = text("surface_ref");
                let inner = self.inner();
                Reply::Ok(inner.surfaces.get(&reference).map_or(Value::Null, |held| {
                    let mut view = inner.meta(&reference, held);
                    view["snapshot"] =
                        serde_json::to_value(held.state.content()).unwrap_or_default();
                    view["fallback_text"] =
                        json!(held.state.content().map(|c| c.fallback_text.clone()));
                    view["actions"] = json!(inner.action_rows(&reference));
                    view["action_receipt_id"] = view["last_action_id"].clone();
                    view
                }))
            }
            "surface.list" => {
                let conversation = text("conversation_id");
                let inner = self.inner();
                let rows: Vec<Value> = inner
                    .surfaces
                    .iter()
                    .filter(|(_, held)| held.conversation == conversation)
                    .map(|(reference, held)| inner.meta(reference, held))
                    .collect();
                Reply::Ok(json!({ "surfaces": rows, "next_cursor": null }))
            }
            "surface.action" => self.action(body),
            "surface.request_resync" => self.request_resync(&text("surface_ref")),
            _ => return None,
        })
    }

    fn action(&self, body: &Value) -> Reply {
        let wire = body["wire_base64"]
            .as_str()
            .and_then(|w| ramflux_protocol::decode_base64url(w).ok())
            .unwrap_or_default();
        let Ok(action) = A2uiSurfaceAction::from_wire_bytes(&wire) else {
            return Reply::Error("ValidationFailed", "invalid surface action".to_owned());
        };
        let author = body["author_principal_id"].as_str().unwrap_or_default().to_owned();
        let reference = surface_ref(action.conversation_id(), &author, action.surface_id());
        let mut inner = self.inner();
        let Some(held) = inner.surfaces.get(&reference) else {
            return Reply::Ok(rejected(json!({ "code": "unknown_surface" }), "not_found"));
        };
        if let Err(reject) = held.state.validate_action(&action) {
            // The daemon's code (its refusal mapping, then: the same
            // version with other content is a stale hash).
            let code = if action.version() == held.state.version()
                && held.state.hash() != Some(action.surface_hash())
            {
                "stale_hash"
            } else {
                match reject {
                    SurfaceReject::Version { .. } | SurfaceReject::Stale => "stale_version",
                    SurfaceReject::Conflict
                    | SurfaceReject::ResultHashMismatch
                    | SurfaceReject::ArgumentsHashMismatch => "stale_hash",
                    SurfaceReject::Closed | SurfaceReject::BlockClosed => "closed",
                    SurfaceReject::NotCreated => "not_found",
                    SurfaceReject::LimitExceeded { .. } => "too_large",
                    _ => "invalid_request",
                }
            };
            let error = json!({ "code": "renderer_error", "reject": reject });
            return Reply::Ok(rejected(error, code));
        }
        let id = format!("rcpt-{}", inner.actions.len() + 1);
        inner.actions.push(HeldAction {
            id,
            surface_ref: reference.clone(),
            message_id: body["message_id"].clone(),
            version: action.version(),
            wire_base64: body["wire_base64"].as_str().unwrap_or_default().to_owned(),
            receipt: None,
        });
        inner.acted.push(Acted {
            surface_ref: reference.clone(),
            version: action.version(),
            action_id: action.action_id().to_owned(),
            author,
        });
        Reply::Ok(json!({
            "accepted": true, "fallback_text": "", "renderer_error": null,
            "surface_ref": reference, "surface_id": action.surface_id(),
            "message_id": body["message_id"], "delivery_state": "submitted",
        }))
    }

    fn request_resync(&self, reference: &str) -> Reply {
        let mut inner = self.inner();
        let Some(held) = inner.surfaces.get(reference) else {
            return Reply::Error("not_found", "surface is unknown".to_owned());
        };
        if !held.out_of_sync {
            return Reply::Error("not_out_of_sync", "surface is not out of sync".to_owned());
        }
        if inner.window.iter().any(|r| r == reference) {
            return Reply::Ok(json!({ "rate_limited": true, "reason_code": "rate_limited",
                                     "retry_after_seconds": 60 }));
        }
        inner.window.push(reference.to_owned());
        inner.resyncs.push(reference.to_owned());
        Reply::Ok(json!({ "submitted_devices": 1, "rate_limited": false,
                          "retry_after_seconds": 60 }))
    }
}

fn rejected(error: Value, code: &str) -> Value {
    json!({ "accepted": false, "fallback_text": "[Surface rejected]", "renderer_error": error,
            "reason_code": code })
}

impl Inner {
    /// The metadata of a surface, with its latest received action.
    fn meta(&self, reference: &str, held: &Held) -> Value {
        let mut meta = meta(reference, held);
        meta["last_action_id"] = self
            .actions
            .iter()
            .rev()
            .find(|a| a.surface_ref == reference && a.receipt.is_some())
            .and_then(|a| a.receipt.as_ref())
            .map_or(Value::Null, |(id, _)| json!(id));
        meta
    }

    /// `surface.get` `actions[]` of a surface: the actions whose receipt
    /// arrived, and this account's task-bound ones still waiting for it.
    fn action_rows(&self, reference: &str) -> Vec<Value> {
        self.actions
            .iter()
            .filter(|a| a.surface_ref == reference)
            .filter_map(|a| {
                let binding = ramflux_protocol::decode_base64url(&a.wire_base64)
                    .ok()
                    .and_then(|w| A2uiSurfaceAction::from_wire_bytes(&w).ok())
                    .and_then(|action| action.task_binding().cloned());
                let (sender, verified) = match &a.receipt {
                    Some((_, sender)) => (sender.clone(), true),
                    None if binding.is_some() => (self.acting.clone()?, false),
                    None => return None,
                };
                Some(json!({
                    "id": a.id, "action_receipt_id": a.id, "receipt_verified": verified,
                    "message_id": a.message_id, "sender_principal_id": sender,
                    "version": a.version, "actionable": false, "task_binding": binding,
                    "wire_base64": a.wire_base64, "signature": "",
                }))
            })
            .collect()
    }
}

fn meta(reference: &str, held: &Held) -> Value {
    json!({
        "surface_ref": reference,
        "surface_id": held.surface_id,
        "conversation_id": held.conversation,
        "author_principal_id": held.author,
        "version": held.state.version(),
        "surface_hash": held.state.hash(),
        "closed": held.state.is_closed(),
        "out_of_sync": held.out_of_sync,
        "renderer_error": held.renderer_error,
        "streams": held.state.stream_states(),
        "created_at": held.posted_at,
        "posted_at": held.posted_at,
        "updated_at": held.updated_at,
    })
}
