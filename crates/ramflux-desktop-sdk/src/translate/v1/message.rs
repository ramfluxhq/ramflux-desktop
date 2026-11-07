// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A v1 message: text, scalar-offset spans, reply and forward references.
//!
//! Offsets count Unicode scalar values, never UTF-8 bytes or UTF-16 units.
//! One bad entity makes the whole message an unsupported placeholder: no
//! partial text, no link, no action (design §3, lines 2944-2954).

use ramflux_desktop_core::{
    ContextId, DomainEvent, ForwardRef, MessagePayload, MomentKind, ParticipantId, Receipt,
    ReplyRef, SpanKind, TextSpan,
};
use ramflux_protocol::message::{Entity, EntityKind, EventIdentity, Forward, Message, MessageRef};

use super::{context_of, media, moment_id, moment_of, mutation, unsupported};
use crate::translate::Translator;

/// Why a message cannot be shown.
pub(super) enum Refusal {
    /// An entity range leaves the text or overflows.
    SpanRange,
    /// An entity kind this build does not know, or a kind missing its value.
    EntityKind,
}

pub(super) fn project(
    translator: &mut Translator,
    identity: &EventIdentity,
    message: &Message,
) -> Option<DomainEvent> {
    let context = context_of(identity);
    let id = moment_id(&context, &identity.id);
    // A tombstone is sticky: content arriving after a delete changes nothing.
    if translator.v1.tombstoned.contains(&id) {
        return None;
    }
    let device = identity.author.as_ref().map_or("", |a| a.device_id.as_str());
    // A local delete that arrived first applies if this turns out to be the
    // account's own message, and is dropped otherwise.
    if translator.v1.pending_local.remove(&id) && translator.is_own_device(device) {
        return mutation::tombstone_pending(translator, identity, &id);
    }
    // The same content again (a replay, or a body reload) re-reports the
    // message as it is now: the edit, receipt and reactions applied since
    // are not rolled back. Once the message left the retained set, content
    // older than an applied edit is kept silently as the base of the replayed edit.
    if let Some(current) = translator.v1.message(&id).cloned() {
        return translator.report_moment(current);
    }
    let kind = match spans(&message.text, &message.entities) {
        Ok(spans) => MomentKind::Message(MessagePayload {
            text: message.text.clone(),
            receipt: if translator.is_own_device(device) {
                Receipt::Sent
            } else {
                Receipt::default()
            },
            reply_to: message.reply_to.as_ref().map(|r| reply(&context, r)),
            forward: message.forward.as_ref().map(|f| forward(&context, f)),
            spans,
            media: media::views(translator, &message.media),
            reactions: mutation::reaction_counts(translator, &id),
            ..Default::default()
        }),
        Err(Refusal::SpanRange) => unsupported("span_out_of_range", "span_out_of_range"),
        Err(Refusal::EntityKind) => unsupported("unknown_entity", "unknown_entity_kind"),
    };
    let origin = translator.origin(device);
    let mut moment = moment_of(identity, id, origin.as_str(), kind);
    if let MomentKind::Message(payload) = &mut moment.kind {
        // Keep the highest receipt applied before the message was retained.
        let floor = translator.v1.fences.get(&moment.id).map_or(0, |fence| fence.receipt);
        if let Some(receipt) = mutation::receipt_of(floor)
            && mutation::rank(receipt) > mutation::rank(payload.receipt)
        {
            payload.receipt = receipt;
        }
    }
    if let MomentKind::Message(payload) = &moment.kind {
        translator.v1.cache_text(&moment.id, &payload.text);
        translator.v1.put(moment.clone(), Some(device.to_owned()));
        // Content older than an applied edit (after a lock, or once the
        // message left the retained set) is only the base of the edit that is
        // replayed after it: keep it without reporting it, and forget what
        // was reported so the edited state is published again.
        if translator.v1.fences.get(&moment.id).is_some_and(|fence| fence.revision > 0) {
            translator.moments.remove(&moment.id);
            return None;
        }
    }
    translator.report_moment(moment)
}

/// The spans of `text`, or why the whole message is refused.
pub(super) fn spans(text: &str, entities: &[Entity]) -> Result<Vec<TextSpan>, Refusal> {
    let scalars = u64::try_from(text.chars().count()).unwrap_or(u64::MAX);
    entities
        .iter()
        .map(|entity| {
            let end = entity.offset.checked_add(entity.length).ok_or(Refusal::SpanRange)?;
            if u64::from(end) > scalars {
                return Err(Refusal::SpanRange);
            }
            Ok(TextSpan { offset: entity.offset, length: entity.length, kind: span_kind(entity)? })
        })
        .collect()
}

fn span_kind(entity: &Entity) -> Result<SpanKind, Refusal> {
    let value = entity.value.as_deref().filter(|v| !v.is_empty());
    match EntityKind::try_from(entity.kind).map_err(|_| Refusal::EntityKind)? {
        EntityKind::EntityBold => Ok(SpanKind::Bold),
        EntityKind::EntityItalic => Ok(SpanKind::Italic),
        EntityKind::EntityCode => Ok(SpanKind::Code),
        EntityKind::EntitySpoiler => Ok(SpanKind::Spoiler),
        EntityKind::EntityLink => {
            value.map(|url| SpanKind::Link { url: url.to_owned() }).ok_or(Refusal::EntityKind)
        }
        EntityKind::EntityMention => value
            .map(|who| SpanKind::Mention { participant: ParticipantId::new(who) })
            .ok_or(Refusal::EntityKind),
        EntityKind::EntityUnspecified => Err(Refusal::EntityKind),
    }
}

/// The context a reference points into: its own, else the message's.
pub(super) fn target_context(here: &ContextId, reference: &MessageRef) -> ContextId {
    if reference.conversation_id.is_empty() {
        here.clone()
    } else {
        ContextId::new(&reference.conversation_id)
    }
}

fn reply(here: &ContextId, reference: &MessageRef) -> ReplyRef {
    ReplyRef { message_id: moment_id(&target_context(here, reference), &reference.id) }
}

fn forward(here: &ContextId, forward: &Forward) -> ForwardRef {
    let original = forward.original.as_ref();
    ForwardRef {
        from_context: original.map(|r| target_context(here, r)),
        from_message: original.map(|r| moment_id(&target_context(here, r), &r.id)),
        from_name: forward.original_display_name.clone(),
    }
}
