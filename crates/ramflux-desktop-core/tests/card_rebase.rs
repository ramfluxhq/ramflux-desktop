// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The card store's safety rules: which host events may follow the held
//! tree after this desktop's own replies, the pool's resume state, the link
//! a suspend or resume clears, a lost connection, an unknown submit outcome,
//! the link host check and the redacted `Debug` output of card data.

use ramflux_desktop_core::{
    CardBatch, CardEvent, CardLink, CardStatus, DomainEvent, EventEnvelope, SourceStatus, Store,
    SurfaceId, Timestamp, https_host,
};
use serde_json::{Value, json};

const SURFACE: &str = "s-form";
const FIELD: &str = "field";
const BUTTON: &str = "button";
const LABEL: &str = "label";

fn node(key: &str, kind: &str, parent: Option<&str>, children: &[&str], attrs: Value) -> Value {
    json!({
        "key": key,
        "component_id": key,
        "source_instance": [],
        "scope": "root:[]",
        "parent_key": parent,
        "slot": {"slot": "child"},
        "children": children,
        "kind": kind,
        "attrs": attrs,
    })
}

fn upsert(parent: Option<&str>, index: usize, node: Value) -> Value {
    json!({"op": "upsert", "parent": parent, "index": index, "node": node})
}

fn field(value: &str) -> Value {
    node(FIELD, "TextField", Some("col"), &[], json!({"input_pointer": "/name", "value": value}))
}

fn label(text: &str) -> Value {
    node(LABEL, "Text", Some(BUTTON), &[], json!({"text": text}))
}

struct Versions {
    previous: u64,
    version: u64,
    signed: u64,
    edit_seq: u64,
}

fn batch(session: u64, v: &Versions, mutations: Vec<Value>, host_slots: Value) -> CardBatch {
    serde_json::from_value(json!({
        "schema": "ramflux.card.mutations.v1",
        "session": session,
        "previous_version": v.previous,
        "version": v.version,
        "signed_version": v.signed,
        "signed_hash": "h",
        "edit_seq": v.edit_seq,
        "mutations": mutations,
        "host_slots": host_slots,
    }))
    .expect("test: batch")
}

fn open_batch(session: u64) -> CardBatch {
    batch(
        session,
        &Versions { previous: 0, version: 1, signed: 1, edit_seq: 0 },
        vec![
            upsert(None, 0, node("root", "Card", None, &["col"], json!({}))),
            upsert(
                Some("root"),
                0,
                node("col", "Column", Some("root"), &[FIELD, BUTTON], json!({})),
            ),
            upsert(Some("col"), 0, field("Ada")),
            upsert(
                Some("col"),
                1,
                node(BUTTON, "Button", Some("col"), &[LABEL], json!({"action_ref": "save"})),
            ),
            upsert(Some(BUTTON), 0, label("Save")),
        ],
        json!([]),
    )
}

struct World {
    store: Store,
    seq: u64,
}

impl World {
    fn opened() -> Self {
        let mut world = Self { store: Store::new(), seq: 0 };
        world.apply(DomainEvent::Card(CardEvent::Opened {
            surface_id: SurfaceId::new(SURFACE),
            session: 7,
            batch: Box::new(open_batch(7)),
        }));
        world
    }

    fn apply(&mut self, event: DomainEvent) {
        self.seq += 1;
        self.store.apply(&EventEnvelope::new(self.seq, Timestamp(0), event)).expect("test: apply");
    }

    fn reply(&mut self, batch: CardBatch, own_reply: bool) {
        self.apply(DomainEvent::Card(CardEvent::Batch {
            surface_id: SurfaceId::new(SURFACE),
            batch: Box::new(batch),
            draft_conflict: false,
            own_reply,
        }));
    }

    fn card(&self) -> &ramflux_desktop_core::CardRecord {
        self.store.card(&SurfaceId::new(SURFACE)).expect("test: card")
    }

    fn seen(&self, key: &str) -> ramflux_desktop_core::CardSeen {
        self.card().tree.as_ref().and_then(|t| t.seen(key)).expect("test: seen")
    }
}

#[test]
fn a_tap_follows_an_own_draft_reply_only_while_its_node_is_unchanged() {
    let mut world = World::opened();
    let button = world.seen(BUTTON);
    let draft = world.seen(FIELD);
    // The reply to this desktop's own draft changes the field only.
    world.reply(
        batch(
            7,
            &Versions { previous: 1, version: 2, signed: 1, edit_seq: 1 },
            vec![upsert(Some("col"), 0, field("Grace"))],
            json!([]),
        ),
        true,
    );
    assert!(world.card().rebasable(BUTTON, &button, false), "the button is what the user saw");
    assert!(world.card().rebasable(FIELD, &draft, true), "a draft follows its own chain");

    // The next own reply relabels the button the user pressed.
    world.reply(
        batch(
            7,
            &Versions { previous: 2, version: 3, signed: 1, edit_seq: 2 },
            vec![upsert(Some(BUTTON), 0, label("Delete everything"))],
            json!([]),
        ),
        true,
    );
    assert!(
        !world.card().rebasable(BUTTON, &button, false),
        "a tap is never rebased onto a relabelled button"
    );
    assert!(world.card().rebasable(FIELD, &draft, true), "the draft's node kept its target");

    // A tap without a recorded content is never rebased.
    let mut blind = world.seen(BUTTON);
    blind.content = None;
    blind.stamp = button.stamp;
    assert!(!world.card().rebasable(BUTTON, &blind, false));
}

#[test]
fn an_outside_change_ends_every_rebase() {
    let mut world = World::opened();
    let button = world.seen(BUTTON);
    let draft = world.seen(FIELD);
    world.reply(
        batch(
            7,
            &Versions { previous: 1, version: 2, signed: 2, edit_seq: 0 },
            vec![upsert(Some("col"), 0, field("Ada"))],
            json!([]),
        ),
        false,
    );
    assert!(!world.card().rebasable(BUTTON, &button, false));
    assert!(!world.card().rebasable(FIELD, &draft, true));
}

#[test]
fn a_resume_marks_the_card_resumed_and_a_suspend_drops_the_link_sheet() {
    let mut world = World::opened();
    let surface = SurfaceId::new(SURFACE);
    world.apply(DomainEvent::Card(CardEvent::OpenUrl {
        surface_id: surface.clone(),
        link: CardLink {
            slot_id: "[link_open,button]".into(),
            request_id: "r1".into(),
            url: "https://example.org/".into(),
            token: json!({}),
        },
    }));
    assert!(world.card().link.is_some());
    world.apply(DomainEvent::Card(CardEvent::Suspended { surface_id: surface.clone() }));
    assert!(world.card().link.is_none(), "a link sheet belongs to the suspended session");
    assert!(!world.card().resumed(world.store.card_pool(), &surface));
    world.apply(DomainEvent::Card(CardEvent::Resumed {
        surface_id: surface.clone(),
        session: 9,
        batch: Box::new(open_batch(9)),
    }));
    assert!(world.card().resumed(world.store.card_pool(), &surface), "a resume resumes the card");
    assert_eq!(world.store.card_pool().entry(SURFACE).map(|e| e.session), Some(9));
}

#[test]
fn a_lost_connection_forgets_every_card() {
    let mut world = World::opened();
    world.apply(DomainEvent::SourceStatusChanged(SourceStatus::Connected));
    assert!(world.store.card(&SurfaceId::new(SURFACE)).is_some());
    world.apply(DomainEvent::SourceStatusChanged(SourceStatus::Disconnected));
    assert!(world.store.card(&SurfaceId::new(SURFACE)).is_none());
    assert!(world.store.card_pool().is_empty());
    assert!(world.store.take_card_due().is_empty());
}

#[test]
fn an_unknown_submit_outcome_keeps_the_button_consumed() {
    let mut world = World::opened();
    let surface = SurfaceId::new(SURFACE);
    world.apply(DomainEvent::Card(CardEvent::PendingSubmit { surface_id: surface.clone() }));
    world.apply(DomainEvent::Card(CardEvent::SubmitFailed { surface_id: surface.clone() }));
    assert_eq!(world.card().status, CardStatus::PendingSubmit);
    assert!(world.card().submit_failed);
    assert!(world.card().refusal.is_none(), "an unknown outcome is no refusal");
    world.store.card_submit_sent(&surface);
    assert!(!world.card().submit_failed);
    // A refusal the desktop decides itself never releases a consumed button.
    world.store.card_refuse(&surface, ramflux_desktop_core::CARD_CHANGED_REASON);
    assert_eq!(world.card().status, CardStatus::PendingSubmit);
}

#[test]
fn a_link_with_anything_but_printable_ascii_is_refused() {
    assert_eq!(https_host("https://example.org/a?b=c#d"), Some("example.org"));
    assert_eq!(https_host("https://example.org:8443/"), Some("example.org"));
    for url in [
        "http://example.org/",
        "https://exa\u{202e}mple.org/",
        "https://example.org/\u{200b}",
        "https://example.org/\u{2066}x",
        "https://exämple.org/",
        "https://example.org/ a",
        "https://example.org/\u{7}",
        "https://user@example.org/",
        "https://",
    ] {
        assert_eq!(https_host(url), None, "{url:?} is refused");
    }
}

#[test]
fn card_data_debug_output_names_no_text_or_token() {
    let slots = json!([{
        "id": "[approval,button]",
        "key": BUTTON,
        "token": {"session": 7, "signedVersion": 1, "editSeq": 0, "treeVersion": 1, "key": BUTTON},
        "kind": "approval",
        "request_id": "secret-request",
        "request_ref": "secret-ref",
    }]);
    let batch = batch(
        7,
        &Versions { previous: 0, version: 1, signed: 1, edit_seq: 0 },
        open_batch(7).mutations.iter().map(|m| serde_json::to_value(m).expect("m")).collect(),
        slots,
    );
    let mut world = World::opened();
    world.reply(
        {
            let mut next = batch.clone();
            next.previous_version = 1;
            next.version = 2;
            next
        },
        true,
    );
    let tree = world.card().tree.clone().expect("tree");
    let token = tree.token_for(BUTTON).expect("token");
    let printed = format!(
        "{batch:?} {tree:?} {:?} {token:?} {:?} {:?}",
        tree.host_slots(),
        batch.mutations,
        world.seen(BUTTON)
    );
    for secret in ["Save", "Ada", "secret-request", "secret-ref", "save", "/name", BUTTON] {
        assert!(!printed.contains(secret), "{secret:?} leaked: {printed}");
    }
}
