// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Typed message schema v1 updates through the projection test seam (S10c c03):
//! Message and Service share one stream ordered by the daemon's order key,
//! spans count Unicode scalars, and a bad entity refuses the whole message.

use ramflux_desktop_core::{
    AccountId, Context, ContextId, ContextKind, DomainEvent, EventEnvelope, FallbackPayload,
    GovernanceAction, MessagePayload, Moment, MomentId, MomentKind, SpanKind, Store, Timestamp,
};
use ramflux_desktop_sdk::Scope;
use ramflux_desktop_sdk::translate::Translator;
use ramflux_desktop_sdk::translate::v1::candidate::apply;
use ramflux_desktop_test::v1_fixtures::{
    entity, forward_of, identity, message_ref, message_update, service_update, text_message,
    unknown_entity, unknown_service_update,
};
use ramflux_protocol::message::{Entity, EntityKind, MessageUpdate, ServiceAction};

const CONTEXT: &str = "c-group";

fn translator() -> Translator {
    Translator::new(Scope::Account(AccountId::new("acct-main")), "/tmp/s10c-projection.sock")
}

fn context() -> Context {
    Context {
        id: ContextId::new(CONTEXT),
        kind: ContextKind::Group,
        title: "Team".to_owned(),
        participants: Vec::new(),
        pinned: false,
        unread: 0,
        muted: false,
        disappearing_secs: None,
        roles: std::collections::BTreeMap::new(),
        epoch: None,
    }
}

fn id_of(n: u8) -> Vec<u8> {
    vec![n; 4]
}

fn message_at(n: u8, hlc: u64, text: &str, entities: Vec<Entity>) -> MessageUpdate {
    message_update(
        identity(CONTEXT, &id_of(n), "dev-ana", hlc, u64::from(n)),
        text_message(text, entities),
    )
}

/// The moments one update projects to.
fn moments(update: &MessageUpdate) -> Vec<Moment> {
    apply(&mut translator(), std::slice::from_ref(update))
        .events
        .into_iter()
        .filter_map(|event| match event {
            DomainEvent::MomentAdded(moment) => Some(*moment),
            _ => None,
        })
        .collect()
}

fn one(update: &MessageUpdate) -> Moment {
    let mut all = moments(update);
    assert_eq!(all.len(), 1, "one moment");
    all.remove(0)
}

fn payload(moment: &Moment) -> &MessagePayload {
    match &moment.kind {
        MomentKind::Message(p) => p,
        other => panic!("not a message: {other:?}"),
    }
}

fn refused(moment: &Moment) -> &FallbackPayload {
    match &moment.kind {
        MomentKind::Fallback(p) => p,
        other => panic!("not a placeholder: {other:?}"),
    }
}

/// The text a span covers, counting scalars.
fn covered(text: &str, offset: u32, length: u32) -> String {
    text.chars().skip(offset as usize).take(length as usize).collect()
}

#[test]
fn v1_message_service_projection_and_order() {
    // Arrival order is not stream order: the clock decides, display time never.
    let late = message_update(
        identity(CONTEXT, &id_of(3), "dev-ana", 300, 3),
        text_message("third", Vec::new()),
    );
    let rename = service_update(
        identity(CONTEXT, &id_of(2), "dev-bo", 200, 1),
        ServiceAction::ServiceNameChanged,
        &[],
        Some("Renamed"),
    );
    let mut reply = text_message("first", vec![entity(0, 5, EntityKind::EntityBold)]);
    reply.reply_to = Some(message_ref(CONTEXT, &id_of(9)));
    reply.forward = Some(forward_of(message_ref("c-other", &id_of(8)), "Kai"));
    let first = message_update(identity(CONTEXT, &id_of(1), "dev-ana", 100, 1), reply);

    let mut tr = translator();
    let events = apply(&mut tr, &[late, rename, first]).events;
    let mut store = Store::new();
    store
        .apply(&EventEnvelope::new(1, Timestamp(1), DomainEvent::ContextUpserted(context())))
        .expect("context");
    for (seq, event) in events.into_iter().enumerate() {
        store.apply(&EventEnvelope::new(2 + seq as u64, Timestamp(1), event)).expect("moment");
    }
    let stream: Vec<(String, &'static str)> = store
        .context_moments(&ContextId::new(CONTEXT))
        .map(|m| {
            let kind = match &m.kind {
                MomentKind::Message(_) => "message",
                MomentKind::GroupRename(_) => "rename",
                _ => "other",
            };
            (m.id.to_string(), kind)
        })
        .collect();
    assert_eq!(
        stream.iter().map(|(_, kind)| *kind).collect::<Vec<_>>(),
        vec!["message", "rename", "message"],
        "one stream, ordered by hlc: {stream:?}"
    );
    assert!(stream[0].0.ends_with("01010101") && stream[2].0.ends_with("03030303"));

    // The first message keeps its spans, reply and forward references.
    let shown = store.moment(&MomentId::new(stream[0].0.clone())).expect("first");
    let MomentKind::Message(p) = &shown.kind else { panic!("message") };
    assert_eq!(p.spans.len(), 1);
    assert_eq!(p.spans[0].kind, SpanKind::Bold);
    assert_eq!(
        p.reply_to.as_ref().map(|r| r.message_id.to_string()),
        Some(format!("msg:{CONTEXT}:09090909"))
    );
    let forward = p.forward.as_ref().expect("forward");
    assert_eq!(forward.from_context, Some(ContextId::new("c-other")));
    assert_eq!(forward.from_message, Some(MomentId::new("msg:c-other:08080808")));
    assert_eq!(forward.from_name.as_deref(), Some("Kai"));
    assert_eq!(shown.order.as_ref().map(|o| (o.hlc, o.device_sequence)), Some((100, 1)));
}

#[test]
fn service_actions_project_to_their_rows() {
    let added = moments(&service_update(
        identity(CONTEXT, &id_of(4), "dev-ana", 10, 4),
        ServiceAction::ServiceMemberAdded,
        &["dev-bo", "dev-cy"],
        None,
    ));
    assert_eq!(added.len(), 2, "one row per subject");
    assert!(added.iter().all(|m| matches!(
        &m.kind,
        MomentKind::Governance(g) if g.action == GovernanceAction::Joined
    )));
    assert_ne!(added[0].id, added[1].id);
    let unknown = one(&unknown_service_update(identity(CONTEXT, &id_of(5), "dev-ana", 11, 5), 77));
    let info = refused(&unknown).unsupported.as_ref().expect("info");
    assert_eq!(info.reason, "unknown_service_action");
    assert!(
        moments(&service_update(
            identity(CONTEXT, &id_of(6), "dev-ana", 12, 6),
            ServiceAction::ServicePinChanged,
            &[],
            None,
        ))
        .is_empty(),
        "a pin is not a stream row"
    );
}

#[test]
fn span_end_at_scalar_length_renders() {
    let moment =
        one(&message_at(1, 1, "A\u{1F600}\u{e9}", vec![entity(0, 3, EntityKind::EntityBold)]));
    assert_eq!(payload(&moment).spans.len(), 1);
}

#[test]
fn span_end_past_scalar_length_refuses_the_whole_message() {
    let moment =
        one(&message_at(1, 1, "A\u{1F600}\u{e9}", vec![entity(0, 4, EntityKind::EntityBold)]));
    let placeholder = refused(&moment);
    assert_eq!(placeholder.text, "", "no partial text");
    assert_eq!(
        placeholder.unsupported.as_ref().map(|i| i.reason.as_str()),
        Some("span_out_of_range")
    );
}

#[test]
fn span_offset_plus_length_overflow_refuses_the_whole_message() {
    let moment = one(&message_at(1, 1, "hello", vec![entity(u32::MAX, 1, EntityKind::EntityBold)]));
    assert_eq!(
        refused(&moment).unsupported.as_ref().map(|i| i.reason.as_str()),
        Some("span_out_of_range")
    );
}

#[test]
fn unrecognized_entity_kind_refuses_the_whole_message_with_no_link_or_action() {
    for kind in [7, 99, -1, 0] {
        let moment = one(&message_at(1, 1, "see here", vec![unknown_entity(0, 3, kind)]));
        let placeholder = refused(&moment);
        assert_eq!(placeholder.text, "", "no partial text for kind {kind}");
        assert_eq!(
            placeholder.unsupported.as_ref().map(|i| i.reason.as_str()),
            Some("unknown_entity_kind")
        );
    }
    // A link without its target carries no usable value either.
    let moment = one(&message_at(1, 1, "see here", vec![entity(0, 3, EntityKind::EntityLink)]));
    assert!(matches!(moment.kind, MomentKind::Fallback(_)));
}

#[test]
fn four_thousand_ninety_six_entities_render() {
    let many = vec![entity(0, 1, EntityKind::EntityItalic); 4_096];
    let moment = one(&message_at(1, 1, "x", many));
    assert_eq!(payload(&moment).spans.len(), 4_096);
}

#[test]
fn spans_count_scalars_not_bytes_or_utf16_units() {
    let text = "A\u{1F600}\u{e9}";
    // Scalar offsets 0, 1, 2 start A, the emoji, e-acute (UTF-16: 0, 1, 3).
    let moment = one(&message_at(
        1,
        1,
        text,
        vec![entity(1, 1, EntityKind::EntityBold), entity(2, 1, EntityKind::EntityItalic)],
    ));
    let spans = &payload(&moment).spans;
    assert_eq!(
        covered(text, spans[0].offset, spans[0].length),
        "\u{1F600}",
        "bold covers the emoji only"
    );
    assert_eq!(covered(text, spans[1].offset, spans[1].length), "\u{e9}");
    // UTF-16 offset 3 is not a scalar offset: it leaves the text.
    let utf16 = one(&message_at(1, 1, text, vec![entity(3, 1, EntityKind::EntityBold)]));
    assert!(matches!(utf16.kind, MomentKind::Fallback(_)));
}
