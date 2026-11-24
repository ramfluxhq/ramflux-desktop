// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Message actions, read receipts and leaving a group, between the two
//! desktops, each through the real controls:
//!
//! 11. Alice replies to Bob's message (its menu, Reply, the composer quotes
//!     it), edits her reply (Edit: the composer takes its text) and deletes
//!     one of her messages for everyone (Delete for everyone, then Confirm
//!     on the strip); Bob's desktop shows the reply with its quote, the new
//!     text marked edited, and the deleted message as deleted.
//! 12. Alice sends a message while Bob's desktop shows the conversation;
//!     Bob's desktop marks it read and Alice's desktop shows it read.
//! 13. Bob leaves the group of step 7 (the conversation's menu, Leave
//!     group, Confirm); Alice's desktop no longer lists him as a member.

use ramflux_desktop_core::{MessagePayload, MomentId, MomentKind, ParticipantId, Receipt, Store};
use ramflux_desktop_ui::i18n::tr;

use super::{Demo, probe};
use crate::rehearsal::daemon::DemoDaemon;
use crate::rehearsal::host::Host;
use crate::rehearsal::steps::Notes;

fn payload<'s>(store: &'s Store, id: &MomentId) -> Option<&'s MessagePayload> {
    match &store.moment(id)?.kind {
        MomentKind::Message(payload) => Some(payload),
        _ => None,
    }
}

fn moment_of(
    store: &Store,
    context: &ramflux_desktop_core::ContextId,
    text: &str,
) -> Option<MomentId> {
    probe::message(store, context, text).map(|m| m.id.clone())
}

/// Opens the menu of `moment` on side `ix` and chooses `label`.
fn choose<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    ix: usize,
    moment: &MomentId,
    label: &str,
) -> Result<(), String> {
    let desk = demo.sides[ix].desk()?;
    desk.scroll_to_moment(demo.host, moment)?;
    desk.click(demo.host, Some(&format!("moment-{moment}")), "message-menu")?;
    desk.click_label(demo.host, &tr(label))
}

/// Step 11: reply, edit and delete for everyone, seen by the other side.
pub fn message_actions<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    notes: &mut Notes,
) -> Result<(), String> {
    let context = demo.conversation.clone();
    let [_, bob_text, alice_text] = demo.texts.as_slice() else {
        return Err("step 5 left no messages to act on".to_owned());
    };
    let (bob_text, alice_text) = (bob_text.clone(), alice_text.clone());
    let run = demo.config.run_id.clone();
    let limit = demo.config.limits.answer;
    for ix in 0..2 {
        demo.sides[ix].desk()?.show_context(demo.host, &context)?;
    }
    // Reply.
    let quoted_a = demo
        .read(0, |s| moment_of(s, &context, &bob_text))?
        .ok_or("Alice's desktop does not show Bob's message")?;
    let quoted_b = demo
        .read(1, |s| moment_of(s, &context, &bob_text))?
        .ok_or("Bob's desktop does not show his own message")?;
    choose(demo, 0, &quoted_a, "message_ui.reply")?;
    let desk = demo.sides[0].desk()?;
    if !desk.exists(demo.host, "composer-action") {
        return Err("Reply did not open the quote in the composer".to_owned());
    }
    let quote = desk.label(demo.host, "composer-action");
    demo.shots(0, "11_reply_compose", notes)?;
    let reply = format!("A reply to Bob ({run})");
    let desk = demo.sides[0].desk()?;
    desk.type_into(demo.host, "composer-input", &reply)?;
    desk.click(demo.host, None, "composer-send")?;
    notes.add(format!(
        "Alice chose Reply on Bob's message (composer: {quote:?}) and sent {reply:?}"
    ));
    let seen = demo.wait(1, limit, "the reply with its quote", |desk, host| {
        desk.store(host, |s| {
            moment_of(s, &context, &reply)
                .and_then(|id| payload(s, &id).cloned())
                .is_some_and(|p| p.reply_to.as_ref().is_some_and(|r| r.message_id == quoted_b))
        })
    })?;
    notes.add(format!("Bob's desktop shows the reply quoting his message after {seen:?}"));
    demo.shots(1, "11_reply_b", notes)?;
    // Edit.
    let reply_a =
        demo.read(0, |s| moment_of(s, &context, &reply))?.ok_or("no reply on Alice's side")?;
    let reply_b =
        demo.read(1, |s| moment_of(s, &context, &reply))?.ok_or("no reply on Bob's side")?;
    choose(demo, 0, &reply_a, "message_ui.edit")?;
    let edited = format!("An edited reply to Bob ({run})");
    let desk = demo.sides[0].desk()?;
    if !desk.exists(demo.host, "composer-action") {
        return Err("Edit did not open the message in the composer".to_owned());
    }
    desk.replace_composer_text(demo.host, &edited)?;
    desk.click(demo.host, None, "composer-send")?;
    let seen = demo.wait(1, limit, "the edited text", |desk, host| {
        desk.store(host, |s| {
            s.body(&reply_b) == Some(edited.as_str())
                && payload(s, &reply_b).is_some_and(|p| p.revision > 0)
        })
    })?;
    let marked =
        demo.sides[1].desk()?.exists_in(demo.host, &format!("moment-{reply_b}"), "message-edited");
    notes.add(format!(
        "Alice edited her reply to {edited:?}; Bob's desktop shows the new text after {seen:?} (marked edited: {marked})"
    ));
    demo.shots(1, "11_edited_b", notes)?;
    // Delete for everyone.
    let gone_a = demo
        .read(0, |s| moment_of(s, &context, &alice_text))?
        .ok_or("no message of Alice's to delete")?;
    let gone_b = demo
        .read(1, |s| moment_of(s, &context, &alice_text))?
        .ok_or("Bob's desktop does not show Alice's message")?;
    choose(demo, 0, &gone_a, "message_ui.delete_everyone")?;
    let desk = demo.sides[0].desk()?;
    if !desk.strip_open(demo.host) {
        return Err("Delete for everyone asked for no confirmation".to_owned());
    }
    demo.shots(0, "11_delete_confirm", notes)?;
    demo.sides[0].desk()?.tap_confirm(demo.host)?;
    let seen = demo.wait(1, limit, "the message deleted", |desk, host| {
        desk.store(host, |s| payload(s, &gone_b).is_some_and(|p| p.tombstone))
    })?;
    let own = demo.read(0, |s| payload(s, &gone_a).is_some_and(|p| p.tombstone))?;
    notes.add(format!(
        "Alice deleted {alice_text:?} for everyone (confirmed on the strip); Bob's desktop shows it deleted after {seen:?}; Alice's too: {own}"
    ));
    demo.shots(0, "11_deleted_a", notes)?;
    demo.shots(1, "11_deleted_b", notes)?;
    Ok(())
}

/// Step 12: a read receipt seen by the sender.
pub fn read_receipt<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    notes: &mut Notes,
) -> Result<(), String> {
    let context = demo.conversation.clone();
    let run = demo.config.run_id.clone();
    let alice = demo.sides[0].facts()?.principal.clone();
    let limits = demo.config.limits;
    demo.sides[1].desk()?.show_context(demo.host, &context)?;
    let text = format!("Read me, Bob ({run})");
    let desk = demo.sides[0].desk()?;
    desk.show_context(demo.host, &context)?;
    desk.type_into(demo.host, "composer-input", &text)?;
    desk.click(demo.host, None, "composer-send")?;
    demo.wait(1, limits.answer, "the message", |desk, host| {
        desk.store(host, |s| probe::message(s, &context, &text).is_some())
    })?;
    notes.add(format!("Alice sent {text:?}; Bob's desktop shows the conversation"));
    let read = demo.wait(0, limits.receipt, "the read receipt", |desk, host| {
        desk.store(host, |s| probe::receipt(s, &context, &text, &alice) == Some(Receipt::Read))
    });
    let receipt = demo.read(0, |s| probe::receipt(s, &context, &text, &alice))?;
    let read = read.map_err(|e| format!("{e}: Alice's message stayed {receipt:?}"))?;
    let label = demo.read(0, |s| moment_of(s, &context, &text))?.and_then(|id| {
        demo.sides[0].desk().ok()?.label_in(demo.host, &format!("moment-{id}"), "message-receipt")
    });
    notes.add(format!(
        "Alice's desktop shows the message read after {read:?} (receipt label {label:?})"
    ));
    demo.shots(0, "12_read_a", notes)?;
    Ok(())
}

/// Step 13: Bob leaves the group; Alice no longer lists him.
pub fn leave_group<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    notes: &mut Notes,
) -> Result<(), String> {
    let (group, _) = demo.group.clone().ok_or("step 7 made no group")?;
    let bob = ParticipantId::new(&demo.sides[1].facts()?.principal);
    let limit = demo.config.limits.answer;
    let desk = demo.sides[1].desk()?;
    desk.show_context(demo.host, &group)?;
    desk.click(demo.host, None, "conversation-more")?;
    desk.click_label(demo.host, &tr("group.leave.action"))?;
    let desk = demo.sides[1].desk()?;
    if !desk.strip_open(demo.host) {
        return Err("Leave group asked for no confirmation".to_owned());
    }
    demo.shots(1, "13_leave_confirm", notes)?;
    demo.sides[1].desk()?.tap_confirm(demo.host)?;
    let gone = demo.wait(0, limit, "Bob gone from the group", |desk, host| {
        desk.store(host, |s| s.context(&group).is_some_and(|c| !c.participants.contains(&bob)))
    });
    let failures = demo.read(1, probe::failures)?;
    let gone = gone.map_err(|e| format!("{e}; Bob's desktop{failures}"))?;
    let bob_side = demo.read(1, |s| s.context(&group).map(|c| c.participants.contains(&bob)))?;
    notes.add(format!(
        "Bob left with Leave group (confirmed); Alice's desktop no longer lists him after {gone:?}; Bob's desktop: group {}",
        match bob_side {
            None => "no longer listed".to_owned(),
            Some(member) => format!("still listed (member: {member})"),
        }
    ));
    demo.shots(0, "13_left_a", notes)?;
    demo.shots(1, "13_left_b", notes)?;
    Ok(())
}
