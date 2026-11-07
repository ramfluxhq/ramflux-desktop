// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Mutations of a v1 message: edit, delete, reaction and receipt.
//!
//! - An edit replaces the content only when its revision is higher than the
//!   one shown; an equal or lower revision is ignored.
//! - A delete is a sticky tombstone (also when it arrives before the
//!   content): the text, preview and notification caches are purged, the
//!   emitted payload carries no text, and later content, edits and
//!   reactions leave it alone. A local delete applies to the account's own
//!   rows only.
//! - A reaction token is one grapheme of 1..=64 bytes without control
//!   characters, not only whitespace; a reactor counts once per token.
//! - A peer receipt only moves an own outbound message up (delivered, then
//!   read), never down.

use ramflux_desktop_core::{
    DeleteScope, DomainEvent, MessagePayload, Moment, MomentId, MomentKind, ReactionCount, Receipt,
};
use ramflux_protocol::message::{
    Delete, DeleteScope as WireScope, Edit, EventIdentity, MessageRef, QualifiedDevice, Reaction,
    Receipt as WireReceipt, ReceiptState,
};
use unicode_segmentation::UnicodeSegmentation as _;

use super::message::{spans, target_context};
use super::{context_of, media, moment_id};
use crate::translate::Translator;

/// The longest reaction token, in bytes.
const TOKEN_MAX_BYTES: usize = 64;

fn target_id(identity: &EventIdentity, target: &MessageRef) -> MomentId {
    moment_id(&target_context(&context_of(identity), target), &target.id)
}

fn payload_of(moment: &Moment) -> Option<&MessagePayload> {
    match &moment.kind {
        MomentKind::Message(payload) => Some(payload),
        _ => None,
    }
}

/// Stores `moment` as the message's latest state and reports the change.
fn publish(translator: &mut Translator, moment: Moment) -> Option<DomainEvent> {
    translator.v1.set_moment(moment.clone());
    translator.report_moment(moment)
}

/// The reaction chips of a message from its reactors.
pub(super) fn reaction_counts(translator: &Translator, id: &MomentId) -> Vec<ReactionCount> {
    let Some(reactors) = translator.v1.reactions.get(id) else { return Vec::new() };
    reactors
        .iter()
        .filter(|(_, who)| !who.is_empty())
        .map(|(token, who)| ReactionCount {
            token: token.clone(),
            count: u32::try_from(who.len()).unwrap_or(u32::MAX),
            mine: who.iter().any(|anchor| translator.v1.own_anchors.contains(anchor)),
        })
        .collect()
}

pub(super) fn edit(
    translator: &mut Translator,
    identity: &EventIdentity,
    edit: &Edit,
) -> Option<DomainEvent> {
    let id = edit.target.as_ref().map(|t| target_id(identity, t))?;
    if translator.v1.tombstoned.contains(&id) {
        return None;
    }
    let mut moment = translator.v1.message(&id)?.clone();
    let replacement = edit.replacement.as_ref()?;
    let MomentKind::Message(payload) = &mut moment.kind else { return None };
    // Only newer than what is shown, and never older than an applied edit.
    if edit.revision <= payload.revision
        || edit.revision < translator.v1.fences.get(&id).map_or(0, |fence| fence.revision)
    {
        return None;
    }
    // A replacement the build cannot show leaves the shown revision alone.
    let new_spans = spans(&replacement.text, &replacement.entities).ok()?;
    payload.text = replacement.text.clone();
    payload.spans = new_spans;
    payload.media = media::views(translator, &replacement.media);
    payload.revision = edit.revision;
    translator.v1.fence(&id).revision = edit.revision;
    translator.v1.cache_text(&id, &replacement.text);
    publish(translator, moment)
}

fn scope_of(delete: &Delete) -> Option<DeleteScope> {
    match WireScope::try_from(delete.scope) {
        Ok(WireScope::Everyone) => Some(DeleteScope::Everyone),
        Ok(WireScope::Local) => Some(DeleteScope::Local),
        Ok(WireScope::Unspecified) | Err(_) => None,
    }
}

pub(super) fn delete(
    translator: &mut Translator,
    identity: &EventIdentity,
    delete: &Delete,
) -> Vec<DomainEvent> {
    let Some(scope) = scope_of(delete) else { return Vec::new() };
    delete
        .targets
        .iter()
        .filter_map(|target| tombstone(translator, identity, target, scope))
        .collect()
}

fn tombstone(
    translator: &mut Translator,
    identity: &EventIdentity,
    target: &MessageRef,
    scope: DeleteScope,
) -> Option<DomainEvent> {
    let id = target_id(identity, target);
    if translator.v1.tombstoned.contains(&id) {
        return None;
    }
    let known = translator.v1.message(&id).cloned();
    if scope == DeleteScope::Local {
        // A local delete touches the account's own rows only. Whose row an
        // unseen message is will be known when it arrives: keep the intent.
        if known.is_none() {
            translator.v1.pending_local.insert(id);
            return None;
        }
        let author = translator.v1.author(&id);
        if !author.is_some_and(|device| translator.is_own_device(device)) {
            return None;
        }
    }
    erase(translator, identity, &id, known, scope, None)
}

/// A local delete that arrived before its target turned out to hit an own
/// row, now that the content (from `identity`) is here.
pub(super) fn tombstone_pending(
    translator: &mut Translator,
    identity: &EventIdentity,
    id: &MomentId,
) -> Option<DomainEvent> {
    let author = identity.author.as_ref().map(|a| a.device_id.clone());
    erase(translator, identity, id, None, DeleteScope::Local, author)
}

/// Turns the message into a sticky tombstone and purges every text cache.
fn erase(
    translator: &mut Translator,
    identity: &EventIdentity,
    id: &MomentId,
    known: Option<Moment>,
    scope: DeleteScope,
    author: Option<String>,
) -> Option<DomainEvent> {
    translator.v1.tombstoned.insert(id.clone());
    translator.v1.purge_text(id);
    translator.v1.reactions.remove(id);
    let receipt = known.as_ref().and_then(|m| payload_of(m)).map(|p| p.receipt);
    let mut moment = known.unwrap_or_else(|| {
        super::moment_of(
            identity,
            id.clone(),
            translator
                .origin(identity.author.as_ref().map_or("", |a| a.device_id.as_str()))
                .as_str(),
            MomentKind::Message(MessagePayload::default()),
        )
    });
    moment.kind = MomentKind::Message(MessagePayload {
        tombstone: true,
        delete_scope: Some(scope),
        receipt: receipt.unwrap_or_default(),
        ..Default::default()
    });
    if translator.v1.message(id).is_none() {
        translator.v1.put(moment.clone(), author);
    }
    publish(translator, moment)
}

/// A reaction token: one grapheme, 1..=64 bytes, no control characters, not
/// only whitespace.
fn valid_token(token: &str) -> bool {
    token.len() <= TOKEN_MAX_BYTES
        && token.graphemes(true).count() == 1
        && !token.chars().any(char::is_control)
        && !token.chars().all(char::is_whitespace)
}

fn reactor_of(author: Option<&QualifiedDevice>) -> String {
    author.map_or_else(String::new, |a| a.account_anchor.clone())
}

pub(super) fn reaction(
    translator: &mut Translator,
    identity: &EventIdentity,
    reaction: &Reaction,
) -> Option<DomainEvent> {
    let id = reaction.target.as_ref().map(|t| target_id(identity, t))?;
    if translator.v1.tombstoned.contains(&id) || !valid_token(&reaction.token) {
        return None;
    }
    let reactor = reactor_of(identity.author.as_ref());
    if identity.author.as_ref().is_some_and(|a| translator.is_own_device(&a.device_id)) {
        translator.v1.own_anchors.insert(reactor.clone());
    }
    let reactors = translator.v1.reactions.get_or_default_mut(&id);
    let changed = if reaction.remove {
        let removed = reactors.get_mut(&reaction.token).is_some_and(|who| who.remove(&reactor));
        if reactors.get(&reaction.token).is_some_and(std::collections::BTreeSet::is_empty) {
            reactors.remove(&reaction.token);
        }
        removed
    } else {
        reactors.entry(reaction.token.clone()).or_default().insert(reactor)
    };
    if !changed {
        return None;
    }
    let counts = reaction_counts(translator, &id);
    let mut moment = translator.v1.message(&id)?.clone();
    let MomentKind::Message(payload) = &mut moment.kind else { return None };
    payload.reactions = counts;
    publish(translator, moment)
}

pub(super) fn receipt(
    translator: &mut Translator,
    identity: &EventIdentity,
    receipt: &WireReceipt,
) -> Vec<DomainEvent> {
    let state = match ReceiptState::try_from(receipt.state) {
        Ok(ReceiptState::ReceiptDelivered) => Receipt::Delivered,
        Ok(ReceiptState::ReceiptRead) => Receipt::Read,
        Ok(ReceiptState::ReceiptUnspecified) | Err(_) => return Vec::new(),
    };
    // A receipt from this account's own device is the read-state path (c12r).
    let from = receipt.receiver.as_ref().map_or("", |r| r.device_id.as_str());
    if translator.is_own_device(from) {
        return Vec::new();
    }
    receipt
        .targets
        .iter()
        .filter_map(|target| {
            let id = target_id(identity, target);
            let author = translator.v1.author(&id)?.to_owned();
            if !translator.is_own_device(&author) {
                return None;
            }
            let mut moment = translator.v1.message(&id)?.clone();
            let MomentKind::Message(payload) = &mut moment.kind else { return None };
            if payload.tombstone || rank(payload.receipt) >= rank(state) {
                return None;
            }
            payload.receipt = state;
            translator.v1.fence(&id).receipt = rank(state);
            publish(translator, moment)
        })
        .collect()
}

/// The receipt a rank stands for.
pub(super) fn receipt_of(rank: u8) -> Option<Receipt> {
    match rank {
        3 => Some(Receipt::Read),
        2 => Some(Receipt::Delivered),
        _ => None,
    }
}

/// Receipts only move up.
pub(super) fn rank(receipt: Receipt) -> u8 {
    match receipt {
        Receipt::Read => 3,
        Receipt::Delivered => 2,
        Receipt::Sent => 1,
        _ => 0,
    }
}

#[cfg(feature = "test-support")]
impl Translator {
    /// Declares `device` this account's own device (a test seam).
    pub fn set_own_device(&mut self, device: &str) {
        self.self_device = Some(device.to_owned());
    }

    /// How many v1 messages are retained with their text (a test probe).
    pub fn retained_messages(&self) -> usize {
        self.v1.retained()
    }

    /// The plaintext cache of a v1 message (a test probe).
    pub fn cached_plaintext(&self, id: &MomentId) -> Option<&str> {
        self.v1.plaintext(id)
    }

    /// The preview cache of a v1 message (a test probe).
    pub fn cached_preview(&self, id: &MomentId) -> Option<&str> {
        self.v1.preview(id)
    }

    /// The notification text cache of a v1 message (a test probe).
    pub fn cached_notification_text(&self, id: &MomentId) -> Option<&str> {
        self.v1.notification(id)
    }
}
