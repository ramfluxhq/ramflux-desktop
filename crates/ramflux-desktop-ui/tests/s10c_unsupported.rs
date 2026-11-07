// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The "newer message type" placeholder of message schema v1 (S10c c06): it
//! carries only kind, layer, reason and stored length; a supported reparse
//! of the same message replaces it exactly once; a tombstone always wins.

use ramflux_desktop_core::{
    AccountId, DomainEvent, MessagePayload, MomentId, MomentKind, UnsupportedInfo,
};
use ramflux_desktop_sdk::Scope;
use ramflux_desktop_sdk::translate::Translator;
use ramflux_desktop_sdk::translate::v1::candidate::apply;
use ramflux_desktop_test::v1_fixtures::{
    delete_update, entity, identity, message_ref, message_update, text_message, unknown_entity,
    unsupported_update, with_journal,
};
use ramflux_protocol::message::{DeleteScope, EntityKind, MessageUpdate};

const CTX: &str = "c-unsup";
const TEXT: &str = "now readable";

fn translator() -> Translator {
    Translator::new(Scope::Account(AccountId::new("acct-main")), "/tmp/s10c-unsupported.sock")
}

fn moment_id() -> MomentId {
    MomentId::new(format!("msg:{CTX}:01010101"))
}

fn id() -> ramflux_protocol::message::EventIdentity {
    identity(CTX, &[1; 4], "dev-ana", 10, 1)
}

fn placeholder(journal: u64) -> MessageUpdate {
    unsupported_update(id(), journal, 1, 99, 412)
}

fn reparse(journal: u64) -> MessageUpdate {
    with_journal(message_update(id(), text_message(TEXT, Vec::new())), journal)
}

fn run(translator: &mut Translator, update: MessageUpdate) -> Vec<DomainEvent> {
    apply(translator, &[update]).events
}

fn kind_of(events: &[DomainEvent]) -> Option<&MomentKind> {
    events.iter().find_map(|event| match event {
        DomainEvent::MomentAdded(moment) => Some(&moment.kind),
        DomainEvent::MomentUpdated { kind: Some(kind), .. } => Some(kind),
        _ => None,
    })
}

fn message(events: &[DomainEvent]) -> MessagePayload {
    match kind_of(events) {
        Some(MomentKind::Message(payload)) => payload.clone(),
        other => panic!("not a message: {other:?}"),
    }
}

fn info(events: &[DomainEvent]) -> UnsupportedInfo {
    match kind_of(events) {
        Some(MomentKind::Fallback(p)) => p.unsupported.clone().expect("unsupported info"),
        other => panic!("not a placeholder: {other:?}"),
    }
}

#[test]
fn v1_unsupported_placeholder_reparse_once_and_erased_stays_erased() {
    // The placeholder, then the reparse replaces it once.
    let mut translator = translator();
    let first = run(&mut translator, placeholder(10));
    assert_eq!(info(&first).kind, "99");
    let replaced = run(&mut translator, reparse(11));
    assert_eq!(message(&replaced).text, TEXT);
    assert!(run(&mut translator, reparse(11)).is_empty(), "replaced once");

    // Erased stays erased: a delete, then a reparse of the same id.
    let mut translator = self::translator();
    run(&mut translator, placeholder(10));
    let delete = delete_update(
        identity(CTX, &[9; 4], "dev-ana", 20, 2),
        vec![message_ref(CTX, &[1; 4])],
        DeleteScope::Everyone,
    );
    let erased = message(&run(&mut translator, delete));
    assert!(erased.tombstone && erased.text.is_empty());
    assert!(run(&mut translator, reparse(11)).is_empty(), "tombstone stays");
    assert_eq!(translator.cached_plaintext(&moment_id()), None, "no reparsed text cached");
}

#[test]
fn placeholder_carries_kind_layer_reason_and_length_only() {
    let mut translator = translator();
    let events = run(&mut translator, placeholder(10));
    let UnsupportedInfo { kind, layer, reason, raw_length } = info(&events);
    assert_eq!(
        (kind.as_str(), layer.as_str(), reason.as_str(), raw_length),
        ("99", "1", "unknown_kind", 412)
    );
    let shown = format!("{events:?}");
    assert!(
        !shown.contains("RAW-HANDLE") && !shown.contains("RAW-DIGEST"),
        "no raw bytes or handle"
    );
    let raw_handle: &[u8] = b"RAW-HANDLE-SECRET";
    assert!(!shown.contains(&format!("{:?}", raw_handle.to_vec())), "no handle bytes in any model");
}

#[test]
fn same_journal_sequence_twice_replaces_once() {
    let mut translator = translator();
    run(&mut translator, placeholder(10));
    assert_eq!(run(&mut translator, reparse(12)).len(), 1, "first delivery replaces");
    assert!(run(&mut translator, reparse(12)).is_empty(), "second delivery changes nothing");
}

#[test]
fn lower_journal_sequence_is_ignored() {
    let mut translator = translator();
    run(&mut translator, placeholder(10));
    assert!(run(&mut translator, reparse(9)).is_empty(), "stale reparse ignored");
    assert_eq!(
        message(&run(&mut translator, reparse(11))).text,
        TEXT,
        "a later one still replaces"
    );
}

#[test]
fn tombstoned_id_stays_tombstone_after_a_reparse() {
    let mut translator = translator();
    run(&mut translator, reparse(10));
    let delete = delete_update(
        identity(CTX, &[9; 4], "dev-ana", 20, 2),
        vec![message_ref(CTX, &[1; 4])],
        DeleteScope::Everyone,
    );
    let erased = message(&run(&mut translator, delete));
    assert!(erased.tombstone && erased.text.is_empty(), "tombstone stays");
    assert!(run(&mut translator, reparse(11)).is_empty());
    assert_eq!(translator.cached_plaintext(&moment_id()), None);
    assert_eq!(translator.cached_preview(&moment_id()), None);
}

#[test]
fn unknown_entity_kind_placeholder_is_replaced_once_by_a_supported_reparse() {
    let mut translator = translator();
    let unknown = with_journal(
        message_update(id(), text_message("see here", vec![unknown_entity(0, 3, 7)])),
        10,
    );
    let events = run(&mut translator, unknown);
    assert_eq!(info(&events).reason, "unknown_entity_kind", "whole message placeholder");
    let supported = with_journal(
        message_update(id(), text_message("see here", vec![entity(0, 3, EntityKind::EntityBold)])),
        11,
    );
    assert_eq!(message(&run(&mut translator, supported.clone())).text, "see here");
    assert!(run(&mut translator, supported).is_empty(), "replaced once");
}
