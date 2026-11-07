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

use super::{context_of, media, moment_id, moment_of, unsupported};
use crate::translate::Translator;

/// Why a message cannot be shown.
enum Refusal {
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
    let device = identity.author.as_ref().map_or("", |a| a.device_id.as_str());
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
            ..Default::default()
        }),
        Err(Refusal::SpanRange) => unsupported("span_out_of_range", "span_out_of_range"),
        Err(Refusal::EntityKind) => unsupported("unknown_entity", "unknown_entity_kind"),
    };
    let origin = translator.origin(device);
    let moment = moment_of(identity, id, origin.as_str(), kind);
    if matches!(&moment.kind, MomentKind::Message(p) if !p.media.is_empty()) {
        translator.media_moments.insert(moment.id.clone(), moment.clone());
    }
    translator.report_moment(moment)
}

/// The spans of `text`, or why the whole message is refused.
fn spans(text: &str, entities: &[Entity]) -> Result<Vec<TextSpan>, Refusal> {
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
fn target_context(here: &ContextId, reference: &MessageRef) -> ContextId {
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
