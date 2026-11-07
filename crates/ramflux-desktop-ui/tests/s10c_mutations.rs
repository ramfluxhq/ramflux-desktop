// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Mutations of message schema v1 (S10c c05): edit with a revision badge,
//! sticky delete tombstones that purge every text cache, the reaction
//! aggregate and peer receipts moving an own message's tick.

mod common;
mod sdk_fake;

use common::*;
use ramflux_desktop_core::{
    AccountId, ContextId, DeleteScope as CoreScope, DomainEvent, MessagePayload, MomentId,
    MomentKind, Receipt,
};
use ramflux_desktop_sdk::Scope;
use ramflux_desktop_sdk::translate::Translator;
use ramflux_desktop_sdk::translate::v1::candidate::apply;
use ramflux_desktop_test::v1_fixtures::{
    delete_update, edit_update, identity, message_ref, message_update, reaction_update,
    receipt_update, text_message,
};
use ramflux_desktop_ui::route::Page;
use ramflux_protocol::message::{DeleteScope, MessageUpdate, ReceiptState};

const CTX: &str = "c-mut";
const OLD: &str = "the old words";

fn translator() -> Translator {
    let mut translator =
        Translator::new(Scope::Account(AccountId::new("acct-main")), "/tmp/s10c-mut.sock");
    translator.set_own_device("dev-me");
    translator
}

fn id_of(n: u8) -> Vec<u8> {
    vec![n; 4]
}

fn moment_id(n: u8) -> MomentId {
    MomentId::new(format!("msg:{CTX}:{}", format!("{n:02x}").repeat(4)))
}

fn content(n: u8, author: &str, text: &str) -> MessageUpdate {
    message_update(
        identity(CTX, &id_of(n), author, u64::from(n), u64::from(n)),
        text_message(text, Vec::new()),
    )
}

/// An id for a mutation event: distinct from every message id.
fn event(n: u8, author: &str) -> ramflux_protocol::message::EventIdentity {
    identity(CTX, &id_of(n), author, 100 + u64::from(n), 100 + u64::from(n))
}

fn run(translator: &mut Translator, update: MessageUpdate) -> Vec<DomainEvent> {
    apply(translator, &[update]).events
}

/// The last message payload among `events`.
fn payload(events: &[DomainEvent]) -> MessagePayload {
    events
        .iter()
        .rev()
        .find_map(|event| match event {
            DomainEvent::MomentAdded(moment) => Some(moment.kind.clone()),
            DomainEvent::MomentUpdated { kind: Some(kind), .. } => Some(kind.clone()),
            _ => None,
        })
        .and_then(|kind| match kind {
            MomentKind::Message(payload) => Some(payload),
            _ => None,
        })
        .expect("a message payload was emitted")
}

fn seeded(author: &str) -> Translator {
    let mut translator = translator();
    run(&mut translator, content(1, author, OLD));
    assert_eq!(translator.cached_plaintext(&moment_id(1)), Some(OLD), "seeded plaintext");
    assert_eq!(translator.cached_preview(&moment_id(1)), Some(OLD), "seeded preview");
    assert_eq!(
        translator.cached_notification_text(&moment_id(1)),
        Some(OLD),
        "seeded notification"
    );
    translator
}

fn assert_purged(translator: &Translator, n: u8) {
    assert_eq!(
        translator.cached_plaintext(&moment_id(n)),
        None,
        "cached plaintext for the id is None after Delete"
    );
    assert_eq!(translator.cached_preview(&moment_id(n)), None, "preview purged");
    assert_eq!(
        translator.cached_notification_text(&moment_id(n)),
        None,
        "notification text purged"
    );
}

fn delete(translator: &mut Translator, n: u8, scope: DeleteScope) -> Vec<DomainEvent> {
    run(translator, delete_update(event(50, "dev-ana"), vec![message_ref(CTX, &id_of(n))], scope))
}

#[test]
fn v1_mutations_tombstone_sticky_and_cache_purged() {
    let mut translator = seeded("dev-ana");
    let events = delete(&mut translator, 1, DeleteScope::Everyone);
    let shown = payload(&events);
    assert!(shown.tombstone && shown.text.is_empty(), "tombstone with no text");
    assert_eq!(shown.delete_scope, Some(CoreScope::Everyone));
    assert_purged(&translator, 1);

    // Sticky: later content, edit and reaction change nothing.
    assert!(run(&mut translator, content(1, "dev-ana", "again")).is_empty());
    let edit = edit_update(
        event(51, "dev-ana"),
        message_ref(CTX, &id_of(1)),
        5,
        text_message("edited after delete", Vec::new()),
    );
    assert!(run(&mut translator, edit).is_empty());
    let react =
        reaction_update(event(52, "dev-bo"), message_ref(CTX, &id_of(1)), "\u{1F44D}", false);
    assert!(run(&mut translator, react).is_empty());
    assert_purged(&translator, 1);
}

#[test]
fn delete_everyone_purges_every_cache_and_empties_the_payload() {
    let mut translator = seeded("dev-ana");
    let shown = payload(&delete(&mut translator, 1, DeleteScope::Everyone));
    assert!(shown.tombstone && shown.text.is_empty());
    assert_purged(&translator, 1);
}

#[test]
fn delete_local_tombstones_own_rows() {
    let mut translator = seeded("dev-me");
    let shown = payload(&delete(&mut translator, 1, DeleteScope::Local));
    assert!(shown.tombstone && shown.text.is_empty());
    assert_eq!(shown.delete_scope, Some(CoreScope::Local));
    assert_purged(&translator, 1);
}

#[test]
fn delete_local_leaves_peer_rows_untouched() {
    let mut translator = seeded("dev-ana");
    assert!(delete(&mut translator, 1, DeleteScope::Local).is_empty(), "no event");
    assert_eq!(translator.cached_plaintext(&moment_id(1)), Some(OLD), "plaintext kept");
    assert_eq!(translator.cached_preview(&moment_id(1)), Some(OLD));
}

#[test]
fn delete_before_content_tombstones_and_later_content_stays_out() {
    let mut translator = translator();
    let shown = payload(&delete(&mut translator, 1, DeleteScope::Everyone));
    assert!(shown.tombstone && shown.text.is_empty());
    assert!(run(&mut translator, content(1, "dev-ana", OLD)).is_empty(), "content ignored");
    let react =
        reaction_update(event(52, "dev-bo"), message_ref(CTX, &id_of(1)), "\u{1F44D}", false);
    assert!(run(&mut translator, react).is_empty(), "reaction ignored");
    assert_purged(&translator, 1);
}

fn edit_to(translator: &mut Translator, revision: u64, text: &str) -> Vec<DomainEvent> {
    let edit = edit_update(
        event(60 + u8::try_from(revision).unwrap_or(0), "dev-ana"),
        message_ref(CTX, &id_of(1)),
        revision,
        text_message(text, Vec::new()),
    );
    run(translator, edit)
}

#[test]
fn edit_with_a_higher_revision_replaces_the_content() {
    let mut translator = seeded("dev-ana");
    let shown = payload(&edit_to(&mut translator, 2, "new words"));
    assert_eq!((shown.text.as_str(), shown.revision), ("new words", 2));
    assert_eq!(translator.cached_plaintext(&moment_id(1)), Some("new words"));
}

#[test]
fn edit_with_an_equal_revision_is_ignored() {
    let mut translator = seeded("dev-ana");
    edit_to(&mut translator, 2, "new words");
    assert!(edit_to(&mut translator, 2, "other words").is_empty());
    assert_eq!(translator.cached_plaintext(&moment_id(1)), Some("new words"));
}

#[test]
fn edit_with_a_lower_revision_is_ignored() {
    let mut translator = seeded("dev-ana");
    edit_to(&mut translator, 3, "new words");
    assert!(edit_to(&mut translator, 2, "stale words").is_empty());
    assert_eq!(translator.cached_plaintext(&moment_id(1)), Some("new words"));
}

/// The chips after one reaction, or `None` when nothing changed.
fn react(
    translator: &mut Translator,
    token: &str,
    remove: bool,
    author: &str,
) -> Option<Vec<(String, u32, bool)>> {
    let update = reaction_update(event(70, author), message_ref(CTX, &id_of(1)), token, remove);
    let events = run(translator, update);
    (!events.is_empty()).then(|| {
        payload(&events).reactions.into_iter().map(|c| (c.token, c.count, c.mine)).collect()
    })
}

fn thumbs() -> String {
    "\u{1F44D}".to_owned()
}

#[test]
fn reaction_of_one_grapheme_counts_once() {
    let mut translator = seeded("dev-ana");
    assert_eq!(
        react(&mut translator, &thumbs(), false, "dev-bo"),
        Some(vec![(thumbs(), 1, false)])
    );
}

#[test]
fn reaction_of_two_graphemes_is_ignored() {
    let mut translator = seeded("dev-ana");
    assert_eq!(react(&mut translator, "\u{1F44D}\u{1F44D}", false, "dev-bo"), None);
}

#[test]
fn empty_reaction_is_ignored() {
    let mut translator = seeded("dev-ana");
    assert_eq!(react(&mut translator, "", false, "dev-bo"), None);
}

#[test]
fn reaction_of_65_bytes_is_ignored() {
    let mut translator = seeded("dev-ana");
    // One grapheme: a base letter with combining marks, 65 bytes long.
    let token = format!("a{}", "\u{301}".repeat(32));
    assert_eq!(token.len(), 65);
    assert_eq!(react(&mut translator, &token, false, "dev-bo"), None);
}

#[test]
fn reaction_of_64_bytes_counts() {
    let mut translator = seeded("dev-ana");
    // One grapheme, exactly 64 bytes: e-acute (2 bytes) and 31 combining marks.
    let token = format!("\u{e9}{}", "\u{301}".repeat(31));
    assert_eq!(token.len(), 64);
    assert_eq!(react(&mut translator, &token, false, "dev-bo"), Some(vec![(token, 1, false)]));
}

#[test]
fn reaction_of_one_byte_counts() {
    let mut translator = seeded("dev-ana");
    assert_eq!(
        react(&mut translator, "a", false, "dev-bo"),
        Some(vec![("a".to_owned(), 1, false)]),
        "a 1-byte grapheme is a valid token"
    );
}

#[test]
fn control_character_reaction_is_ignored() {
    let mut translator = seeded("dev-ana");
    assert_eq!(react(&mut translator, "\u{7}", false, "dev-bo"), None);
}

#[test]
fn whitespace_only_reaction_is_ignored() {
    let mut translator = seeded("dev-ana");
    assert_eq!(react(&mut translator, " ", false, "dev-bo"), None);
}

#[test]
fn reaction_added_twice_counts_once() {
    let mut translator = seeded("dev-ana");
    react(&mut translator, &thumbs(), false, "dev-bo");
    assert_eq!(react(&mut translator, &thumbs(), false, "dev-bo"), None, "no change");
    assert_eq!(
        react(&mut translator, &thumbs(), false, "dev-cy"),
        Some(vec![(thumbs(), 2, false)])
    );
}

#[test]
fn removing_an_absent_reaction_changes_nothing() {
    let mut translator = seeded("dev-ana");
    assert_eq!(react(&mut translator, &thumbs(), true, "dev-bo"), None);
}

#[test]
fn reaction_added_then_removed_hides_the_chip() {
    let mut translator = seeded("dev-ana");
    react(&mut translator, &thumbs(), false, "dev-bo");
    assert_eq!(react(&mut translator, &thumbs(), true, "dev-bo"), Some(Vec::new()));
}

#[test]
fn own_reaction_is_marked_mine() {
    let mut translator = seeded("dev-ana");
    assert_eq!(react(&mut translator, &thumbs(), false, "dev-me"), Some(vec![(thumbs(), 1, true)]));
}

fn receipt_for_own_message(
    translator: &mut Translator,
    state: ReceiptState,
    receiver: &str,
) -> Vec<DomainEvent> {
    let update =
        receipt_update(event(80, receiver), vec![message_ref(CTX, &id_of(1))], state, receiver);
    run(translator, update)
}

#[test]
fn peer_receipts_move_an_own_message_up_and_never_down() {
    let mut translator = seeded("dev-me");
    assert_eq!(
        payload(&receipt_for_own_message(
            &mut translator,
            ReceiptState::ReceiptDelivered,
            "dev-ana"
        ))
        .receipt,
        Receipt::Delivered
    );
    assert_eq!(
        payload(&receipt_for_own_message(&mut translator, ReceiptState::ReceiptRead, "dev-ana"))
            .receipt,
        Receipt::Read
    );
    assert!(
        receipt_for_own_message(&mut translator, ReceiptState::ReceiptDelivered, "dev-ana")
            .is_empty(),
        "a later delivered does not lower read"
    );
}

#[test]
fn receipt_from_an_own_device_does_not_move_the_tick() {
    let mut translator = seeded("dev-me");
    assert!(
        receipt_for_own_message(&mut translator, ReceiptState::ReceiptRead, "dev-me").is_empty()
    );
}

#[test]
fn receipt_for_a_peer_message_is_ignored() {
    let mut translator = seeded("dev-ana");
    assert!(
        receipt_for_own_message(&mut translator, ReceiptState::ReceiptRead, "dev-bo").is_empty()
    );
}

const DM: &str = "dm-ana";

#[gpui_kit::test]
fn edit_badge_and_reaction_chips_render(cx: &mut gpui_kit::TestAppContext) {
    let _locale = locale_guard();
    let (fake, _state, path) = sdk_fake::start("s10c-mutations");
    let (handle, shell) = open_with(cx, sdk_fake::options(&path));
    cx.update(|cx| ramflux_desktop_ui::i18n::select_locale("en", cx));
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the conversation", |cx| {
        with_store(cx, &shell, |s| s.context(&ContextId::new(DM)).is_some())
    });
    show_context(cx, handle, &shell, DM);
    let id = |n: u8, hlc: u64| identity(DM, &[n; 4], "dev-ana", hlc, hlc);
    let first = message_update(id(1, 10), text_message("before", Vec::new()));
    assert!(sdk_fake::push_generated(&path, first));
    wait(cx, "the message", |cx| {
        with_store(cx, &shell, |s| s.moment(&MomentId::new("msg:dm-ana:01010101")).is_some())
    });
    let target = message_ref(DM, &[1; 4]);
    let edit = edit_update(id(2, 20), target.clone(), 1, text_message("after", Vec::new()));
    let like = reaction_update(id(3, 30), target.clone(), "\u{1F44D}", false);
    let like_again =
        reaction_update(identity(DM, &[4; 4], "dev-bo", 40, 40), target, "\u{1F44D}", false);
    for update in [edit, like, like_again] {
        assert!(sdk_fake::push_generated(&path, update));
    }
    wait(cx, "the badge", |cx| has_label(cx, handle, "Edited"));
    wait(cx, "the chip", |cx| has_label(cx, handle, "\u{1F44D} 2"));
    with_store(cx, &shell, |s| {
        let MomentKind::Message(p) =
            &s.moment(&MomentId::new("msg:dm-ana:01010101")).expect("m").kind
        else {
            panic!("message")
        };
        assert_eq!((p.revision, p.reactions.len(), p.reactions[0].count), (1, 1, 2));
        assert_eq!(
            s.bodies().get(&MomentId::new("msg:dm-ana:01010101")).map(|t| t.to_string()),
            Some("after".to_owned())
        );
    });
    // Invalid tokens leave no chip.
    let target = message_ref(DM, &[1; 4]);
    for (n, token) in [(5_u8, "ab"), (6, "\u{7}"), (7, " ")] {
        let bad = reaction_update(id(n, 50 + u64::from(n)), target.clone(), token, false);
        assert!(sdk_fake::push_generated(&path, bad));
    }
    advance(cx, 300);
    assert!(!has_label(cx, handle, "ab 1") && !has_label(cx, handle, " 1"));
    assert!(
        !exists_in(cx, handle, "moment-msg:dm-ana:01010101", "message-reaction-1"),
        "one chip only"
    );
    // Taking both reactions back hides the chip.
    for (n, who) in [(8_u8, "dev-ana"), (9, "dev-bo")] {
        let undo = reaction_update(
            identity(DM, &[n; 4], who, 60 + u64::from(n), 60 + u64::from(n)),
            target.clone(),
            "\u{1F44D}",
            true,
        );
        assert!(sdk_fake::push_generated(&path, undo));
    }
    wait(cx, "the chip gone", |cx| {
        !has_label(cx, handle, "\u{1F44D} 2") && !has_label(cx, handle, "\u{1F44D} 1")
    });
    assert!(!exists_in(cx, handle, "moment-msg:dm-ana:01010101", "message-reaction-0"));
    assert!(!exists_in(cx, handle, "moment-msg:dm-ana:01010101", "message-reactions"));
    fake.shutdown();
}

#[test]
fn replayed_content_does_not_roll_back_an_edit() {
    let mut translator = seeded("dev-ana");
    edit_to(&mut translator, 3, "new words");
    assert!(
        run(&mut translator, content(1, "dev-ana", OLD)).is_empty(),
        "the identical content again changes nothing"
    );
    assert_eq!(translator.cached_plaintext(&moment_id(1)), Some("new words"));
    assert!(edit_to(&mut translator, 3, "other").is_empty(), "revision 3 is still applied");
}

#[test]
fn replayed_content_does_not_lower_a_receipt() {
    let mut translator = seeded("dev-me");
    receipt_for_own_message(&mut translator, ReceiptState::ReceiptRead, "dev-ana");
    assert!(
        run(&mut translator, content(1, "dev-me", OLD)).is_empty(),
        "still read, nothing emitted"
    );
}

/// Floods the translator with other messages until message 1 is evicted.
fn evict(translator: &mut Translator) {
    for n in 0..4_200_u32 {
        let id = n.to_be_bytes();
        let update = message_update(
            identity(CTX, &id, "dev-ana", 1_000 + u64::from(n), 1_000 + u64::from(n)),
            text_message("history", Vec::new()),
        );
        run(translator, update);
    }
}

#[test]
fn content_older_than_an_applied_edit_is_ignored_after_eviction() {
    let mut translator = seeded("dev-ana");
    edit_to(&mut translator, 3, "new words");
    evict(&mut translator);
    assert!(run(&mut translator, content(1, "dev-ana", OLD)).is_empty(), "the edit fence holds");
}

#[test]
fn a_read_receipt_survives_the_eviction_of_its_message() {
    let mut translator = seeded("dev-me");
    receipt_for_own_message(&mut translator, ReceiptState::ReceiptRead, "dev-ana");
    evict(&mut translator);
    // The rebuilt message carries the read receipt, so it equals what was
    // reported: nothing is emitted (without the fence it would regress to sent).
    assert!(run(&mut translator, content(1, "dev-me", OLD)).is_empty(), "the receipt fence holds");
}

#[test]
fn local_delete_before_peer_content_leaves_the_peer_row() {
    let mut translator = translator();
    assert!(
        delete(&mut translator, 1, DeleteScope::Local).is_empty(),
        "no fence for an unseen target"
    );
    let shown = payload(&run(&mut translator, content(1, "dev-ana", OLD)));
    assert!(!shown.tombstone);
    assert_eq!(shown.text, OLD, "the peer row is untouched");
}

#[test]
fn local_delete_before_own_content_tombstones_it() {
    let mut translator = translator();
    assert!(delete(&mut translator, 1, DeleteScope::Local).is_empty());
    let shown = payload(&run(&mut translator, content(1, "dev-me", OLD)));
    assert!(shown.tombstone && shown.text.is_empty());
    assert_eq!(shown.delete_scope, Some(CoreScope::Local));
    assert_purged(&translator, 1);
}

#[test]
fn sustained_history_keeps_retained_text_bounded() {
    let mut translator = translator();
    evict(&mut translator);
    assert!(translator.retained_messages() <= 4_096, "bounded like the body cache");
    assert_eq!(translator.cached_plaintext(&moment_id(0)), None, "the oldest text is dropped");
    translator.own_account_locked();
    assert_eq!(translator.retained_messages(), 0, "a lock drops the text");
}

#[test]
fn a_lock_keeps_the_light_fences() {
    let mut translator = seeded("dev-ana");
    delete(&mut translator, 1, DeleteScope::Everyone);
    translator.own_account_locked();
    assert!(
        run(&mut translator, content(1, "dev-ana", OLD)).is_empty(),
        "a deleted message stays deleted"
    );
}

fn lock(translator: &mut Translator) {
    translator.own_account_locked();
}

#[test]
fn an_edited_message_returns_after_a_lock() {
    let mut translator = seeded("dev-ana");
    edit_to(&mut translator, 3, "new words");
    lock(&mut translator);
    assert!(
        run(&mut translator, content(1, "dev-ana", OLD)).is_empty(),
        "the base is kept silently"
    );
    let shown = payload(&edit_to(&mut translator, 3, "new words"));
    assert_eq!(
        (shown.text.as_str(), shown.revision),
        ("new words", 3),
        "the edit is published again"
    );
    assert_eq!(translator.cached_plaintext(&moment_id(1)), Some("new words"));
}

#[test]
fn an_edited_message_returns_after_eviction() {
    let mut translator = seeded("dev-ana");
    edit_to(&mut translator, 3, "new words");
    evict(&mut translator);
    assert!(
        run(&mut translator, content(1, "dev-ana", OLD)).is_empty(),
        "the base is kept silently"
    );
    let shown = payload(&edit_to(&mut translator, 3, "new words"));
    assert_eq!((shown.text.as_str(), shown.revision), ("new words", 3));
}

#[test]
fn an_older_edit_is_ignored_when_the_base_comes_back() {
    let mut translator = seeded("dev-ana");
    edit_to(&mut translator, 3, "new words");
    lock(&mut translator);
    run(&mut translator, content(1, "dev-ana", OLD));
    assert!(edit_to(&mut translator, 2, "stale words").is_empty(), "older than the applied edit");
    assert_eq!(payload(&edit_to(&mut translator, 3, "new words")).revision, 3);
}

/// Floods the translator with `count` other messages starting at `from`.
fn flood(translator: &mut Translator, from: u32, count: u32) {
    for n in from..from + count {
        let update = message_update(
            identity(CTX, &n.to_be_bytes(), "dev-ana", 1_000 + u64::from(n), 1_000 + u64::from(n)),
            text_message("history", Vec::new()),
        );
        run(translator, update);
    }
}

#[test]
fn a_republished_message_keeps_its_author_for_a_local_delete() {
    let mut translator = seeded("dev-me");
    flood(&mut translator, 10, 3_000);
    // A peer receipt republishes the message, refreshing its retention.
    receipt_for_own_message(&mut translator, ReceiptState::ReceiptDelivered, "dev-ana");
    flood(&mut translator, 5_000, 3_000);
    let shown = payload(&delete(&mut translator, 1, DeleteScope::Local));
    assert!(shown.tombstone, "the own row is still known as own after the republication");
}

#[test]
fn a_republished_message_keeps_its_author_for_a_peer_receipt() {
    let mut translator = seeded("dev-me");
    flood(&mut translator, 10, 3_000);
    receipt_for_own_message(&mut translator, ReceiptState::ReceiptDelivered, "dev-ana");
    flood(&mut translator, 5_000, 3_000);
    let shown =
        payload(&receipt_for_own_message(&mut translator, ReceiptState::ReceiptRead, "dev-ana"));
    assert_eq!(shown.receipt, Receipt::Read);
}

#[test]
fn a_refusal_after_a_delete_does_not_bring_the_text_back() {
    use ramflux_desktop_sdk::translate::v1::candidate::media_failed;
    use ramflux_desktop_test::v1_fixtures::media_ref;
    let mut translator = translator();
    let mut message = text_message(OLD, Vec::new());
    message.media = vec![media_ref("o-1", "cat.png", "image/png", 10, None)];
    run(&mut translator, message_update(identity(CTX, &id_of(1), "dev-ana", 1, 1), message));
    let erased = payload(&delete(&mut translator, 1, DeleteScope::Everyone));
    assert!(erased.tombstone && erased.media.is_empty());
    let after = media_failed(&mut translator, "o-1", &ramflux_desktop_sdk::bus::BusError::Timeout);
    assert!(
        after.events.is_empty(),
        "nothing is re-emitted over the tombstone: {:?}",
        after.events
    );
    assert_eq!(translator.cached_plaintext(&moment_id(1)), None);
}

/// A message edited to revision 3, then an account lock and the content
/// replayed once: its base is retained but pending, nothing is reported.
fn pending_base(author: &str) -> Translator {
    let mut translator = seeded(author);
    edit_to(&mut translator, 3, "new words");
    lock(&mut translator);
    assert!(run(&mut translator, content(1, author, OLD)).is_empty(), "the base is silent");
    translator
}

fn assert_edit_returns(translator: &mut Translator) -> MessagePayload {
    let shown = payload(&edit_to(translator, 3, "new words"));
    assert_eq!((shown.text.as_str(), shown.revision), ("new words", 3), "the edited state");
    shown
}

#[test]
fn a_pending_base_is_not_republished_by_a_second_content_replay() {
    let mut translator = pending_base("dev-ana");
    assert!(
        run(&mut translator, content(1, "dev-ana", OLD)).is_empty(),
        "the same content again must not show the text the edit removed"
    );
    assert_edit_returns(&mut translator);
}

#[test]
fn a_pending_base_is_not_published_by_a_requested_body_reload() {
    let mut translator = pending_base("dev-ana");
    translator.request_reload(&moment_id(1));
    assert!(run(&mut translator, content(1, "dev-ana", OLD)).is_empty(), "no pre-edit text");
    assert_edit_returns(&mut translator);
}

#[test]
fn a_pending_base_is_not_republished_by_a_media_refusal() {
    use ramflux_desktop_sdk::translate::v1::candidate::media_failed;
    use ramflux_desktop_test::v1_fixtures::media_ref;
    let mut translator = seeded("dev-ana");
    let with_media = |text: &str| {
        let mut message = text_message(text, Vec::new());
        message.media = vec![media_ref("o-9", "cat.png", "image/png", 10, None)];
        message
    };
    run(
        &mut translator,
        message_update(identity(CTX, &id_of(2), "dev-ana", 2, 2), with_media(OLD)),
    );
    let edit =
        edit_update(event(61, "dev-ana"), message_ref(CTX, &id_of(2)), 3, with_media("new words"));
    run(&mut translator, edit.clone());
    lock(&mut translator);
    run(
        &mut translator,
        message_update(identity(CTX, &id_of(2), "dev-ana", 2, 2), with_media(OLD)),
    );
    let after = media_failed(&mut translator, "o-9", &ramflux_desktop_sdk::bus::BusError::Timeout);
    assert!(
        after.events.is_empty(),
        "a refusal must not republish the pre-edit text: {:?}",
        after.events
    );
    let shown = payload(&run(&mut translator, edit));
    assert_eq!((shown.text.as_str(), shown.revision), ("new words", 3));
    assert_eq!(
        shown.media[0].state,
        ramflux_desktop_core::MediaState::Unavailable,
        "the refusal is applied to the edited state"
    );
}

#[test]
fn a_pending_base_is_not_republished_by_a_reaction() {
    let mut translator = pending_base("dev-ana");
    assert_eq!(react(&mut translator, &thumbs(), false, "dev-bo"), None, "silent while pending");
    let shown = assert_edit_returns(&mut translator);
    assert_eq!(shown.reactions.len(), 1, "the reaction is part of the edited state");
}

#[test]
fn a_pending_base_is_not_republished_by_a_receipt() {
    let mut translator = pending_base("dev-me");
    assert!(
        receipt_for_own_message(&mut translator, ReceiptState::ReceiptRead, "dev-ana").is_empty(),
        "silent while pending"
    );
    let shown = assert_edit_returns(&mut translator);
    assert_eq!(shown.receipt, Receipt::Read, "the receipt is part of the edited state");
}

#[test]
fn a_delete_still_wins_over_a_pending_base() {
    let mut translator = pending_base("dev-ana");
    let shown = payload(&delete(&mut translator, 1, DeleteScope::Everyone));
    assert!(shown.tombstone && shown.text.is_empty(), "no pre-edit text in the tombstone");
    assert!(edit_to(&mut translator, 3, "new words").is_empty(), "the tombstone is sticky");
}
