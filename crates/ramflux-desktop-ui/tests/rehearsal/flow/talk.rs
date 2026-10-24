// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Steps 4 to 6: contacts both ways, a conversation started from the
//! contacts dock with messages both ways, and an attachment sent with the
//! composer's attach button.

use ramflux_desktop_core::{
    ContactLink, ContextId, Direction, ParticipantId, SafetyState, TransferState,
};

use super::setup::short;
use super::{Demo, probe};
use crate::rehearsal::daemon::DemoDaemon;
use crate::rehearsal::host::Host;
use crate::rehearsal::steps::Notes;

/// Step 4: each side adds the other by principal id in the contacts dock
/// (both directions, as the daemon links contacts one way per add), then
/// verifies the other's safety number where the daemon offers it.
pub fn contacts<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    notes: &mut Notes,
) -> Result<(), String> {
    for (me, peer) in [(0, 1), (1, 0)] {
        let id = demo.sides[peer].facts()?.principal.clone();
        let desk = demo.sides[me].desk()?;
        desk.click(demo.host, None, "rail-contacts")?;
        desk.type_into(demo.host, "contact-add-id", &id)?;
        desk.click(demo.host, None, "contact-add")?;
        let row = format!("contacts-{id}");
        let participant = ParticipantId::new(&id);
        let limit = demo.config.limits.answer;
        let added = demo.wait(me, limit, "the contact", |desk, host| {
            desk.exists(host, &row)
                && desk.store(host, |s| {
                    s.participant(&participant)
                        .is_some_and(|p| p.link_state() == ContactLink::Accepted)
                })
        });
        if let Err(error) = added {
            let note = demo.sides[me].desk()?.label(demo.host, "contact-add-note");
            return Err(note.map_or(error.clone(), |note| format!("{error}: {note}")));
        }
        notes.add(format!(
            "{} added {} ({})",
            demo.sides[me].name,
            demo.sides[peer].name,
            short(&id)
        ));
        verify(demo, me, &participant, &row, notes)?;
        demo.shots(me, "04_contacts", notes)?;
        demo.sides[me].desk()?.click(demo.host, None, "rail-contacts")?;
    }
    Ok(())
}

/// Presses Verify on the contact row when the daemon offers it and notes
/// the safety state either way.
fn verify<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    me: usize,
    participant: &ParticipantId,
    row: &str,
    notes: &mut Notes,
) -> Result<(), String> {
    let name = demo.sides[me].name;
    let limit = demo.config.limits.verify;
    let start = std::time::Instant::now();
    // Verify needs the contact's identity, which the daemon lists once it
    // has the contact's devices; press again until it is verified or the
    // limit passes.
    while start.elapsed() < limit {
        let desk = demo.sides[me].desk()?;
        let _ = desk.click(demo.host, Some(row), "verify");
        let pressed = std::time::Instant::now();
        while pressed.elapsed() < std::time::Duration::from_secs(1) {
            demo.host.tick();
        }
        let verified = demo.read(me, |s| {
            s.participant(participant)
                .filter(|p| p.safety == SafetyState::Verified)
                .map(|p| p.safety_number.clone())
        })?;
        if let Some(number) = verified {
            notes.add(format!(
                "{name}: safety number verified ({})",
                number.as_deref().map_or_else(|| "no number shown".to_owned(), short)
            ));
            return Ok(());
        }
    }
    let state = demo.read(me, |s| s.participant(participant).map(|p| p.safety))?;
    notes.add(format!(
        "{name}: safety number not verifiable yet ({state:?}; the daemon lists it once it has \
         the contact's devices)"
    ));
    Ok(())
}

/// Step 5: Alice starts the conversation from Bob's row in her contacts
/// dock (a draft, then the first message from the composer), then Bob
/// replies and Alice writes again from their desktops; both see all three,
/// and each sent message is delivered. Bob's desktop learns a conversation
/// it did not start from his daemon's event stream (`conversation.changed`);
/// should it not arrive in time, the reference CLI receives it and a note
/// says the stream did not deliver it.
pub fn messages<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    notes: &mut Notes,
) -> Result<(), String> {
    let (alice, bob) = (demo.sides[0].facts()?.clone(), demo.sides[1].facts()?.clone());
    let run = demo.config.run_id.clone();
    let limits = demo.config.limits;
    let opener = format!("Hello Bob, this is Alice ({run})");
    start(demo, &bob.principal, &opener, notes)?;
    let context = demo.conversation.clone();
    notes.add(format!(
        "opener started from the contacts dock (Message on Bob, first message from the \
         composer); conversation {}",
        short(context.as_str())
    ));
    see(demo, 0, &opener)?;
    let found = demo.wait(1, limits.discover, "the conversation", |desk, host| {
        desk.store(host, |s| s.context(&context).is_some())
    });
    if found.is_err() {
        demo.sides[1].daemon.dm_read(&bob.account, context.as_str())?;
        notes.add(
            "Bob's desktop did not get the new conversation from the event stream in time; \
             the reference CLI received it (rf dm read)",
        );
        demo.wait(1, limits.answer, "the conversation", |desk, host| {
            desk.store(host, |s| s.context(&context).is_some())
        })?;
    } else {
        notes.add("Bob's desktop got the new conversation from the event stream");
    }
    demo.sides[1].desk()?.show_context(demo.host, &context)?;
    see(demo, 1, &opener)?;
    let reply = format!("Hi Alice, Bob here ({run})");
    send(demo, 1, &reply)?;
    see(demo, 0, &reply)?;
    let again = format!("Good to see you, Bob ({run})");
    send(demo, 0, &again)?;
    see(demo, 1, &again)?;
    for (ix, text, me) in [(0, &opener, &alice), (1, &reply, &bob), (0, &again, &alice)] {
        let waited = demo.wait(ix, limits.receipt, "the delivery receipt", |desk, host| {
            desk.store(host, |s| probe::delivered(probe::receipt(s, &context, text, &me.principal)))
        });
        let receipt = demo.read(ix, |s| probe::receipt(s, &context, text, &me.principal))?;
        let who = demo.sides[ix].name;
        if waited.is_err() {
            return Err(format!("{who}'s message {text:?} stayed {receipt:?}"));
        }
        notes.add(format!("{who}: {text:?} {receipt:?}"));
    }
    demo.texts = vec![opener, reply, again];
    demo.shots(0, "05_messages", notes)?;
    demo.shots(1, "05_messages", notes)?;
    Ok(())
}

/// Alice presses Message on `peer`'s contact row, writes `text` as the
/// first message of the draft and sends it; the conversation she lands in
/// becomes the demo's.
fn start<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    peer: &str,
    text: &str,
    notes: &mut Notes,
) -> Result<(), String> {
    let row = format!("contacts-{peer}");
    let limit = demo.config.limits.answer;
    let desk = demo.sides[0].desk()?;
    if !desk.exists(demo.host, &row) {
        desk.click(demo.host, None, "rail-contacts")?;
    }
    desk.click(demo.host, Some(&row), "message")?;
    if !desk.draft_open(demo.host) {
        return Err("Message on the contact opened no new-conversation draft".to_owned());
    }
    demo.shots(0, "05_new_conversation", notes)?;
    let desk = demo.sides[0].desk()?;
    if let Some(reason) = desk.label(demo.host, "composer-unavailable") {
        return Err(format!("the draft cannot start: {reason}"));
    }
    desk.type_into(demo.host, "composer-input", text)?;
    desk.click(demo.host, None, "composer-send")?;
    let started = demo.wait(0, limit, "the started conversation", |desk, host| {
        !desk.draft_open(host) && desk.current_context(host).is_some()
    });
    if let Err(error) = started {
        let desk = demo.sides[0].desk()?;
        let status = desk.label(demo.host, "new-conversation-status");
        return Err(status.map_or(error.clone(), |status| format!("{error}: {status}")));
    }
    let desk = demo.sides[0].desk()?;
    demo.conversation = desk.current_context(demo.host).unwrap_or_else(|| ContextId::new(""));
    Ok(())
}

/// Sends `text` from side `ix`'s composer in the shown conversation.
fn send<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    ix: usize,
    text: &str,
) -> Result<(), String> {
    let desk = demo.sides[ix].desk()?;
    if let Some(reason) = desk.label(demo.host, "composer-unavailable") {
        return Err(format!("{} cannot send here: {reason}", demo.sides[ix].name));
    }
    desk.type_into(demo.host, "composer-input", text)?;
    desk.click(demo.host, None, "composer-send")?;
    see(demo, ix, text)
}

/// Waits until side `ix` shows the message `text` in the conversation.
fn see<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    ix: usize,
    text: &str,
) -> Result<(), String> {
    let context = demo.conversation.clone();
    let limit = demo.config.limits.answer;
    demo.wait(ix, limit, "the message", |desk, host| {
        desk.store(host, |s| probe::message(s, &context, text).is_some())
    })
    .map(drop)
}

/// Step 6: Alice attaches a file with the composer's attach button: the
/// file is checked and staged, then sent. While it travels her message
/// shows it as transferring; then her objects list it as sent and Bob sees
/// the message. Bob's copy is reported as it is: the daemon does not fetch a
/// received attachment yet, so nothing is shown as available.
pub fn attachment<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    notes: &mut Notes,
) -> Result<(), String> {
    let context = demo.conversation.clone();
    let limit = demo.config.limits.answer;
    let name = format!("rehearsal-{}.txt", demo.config.run_id);
    let path = demo.work.join(&name);
    let bytes: Vec<u8> =
        (0..8192_u32).map(|i| b"ramflux demo attachment\n"[i as usize % 24]).collect();
    std::fs::write(&path, &bytes).map_err(|e| format!("cannot write the attachment: {e}"))?;
    let before = demo.read(1, |s| s.context_moment_count(&context))?;
    let desk = demo.sides[0].desk()?;
    desk.show_context(demo.host, &context)?;
    let chosen = desk.attach_file(demo.host, &path)?;
    if !desk.exists(demo.host, "composer-attachment") {
        return Err("the chosen file is not staged in the composer".to_owned());
    }
    if let Some(detail) = desk.label(demo.host, "composer-attachment") {
        notes.add(format!("staged: {detail}"));
    }
    demo.shots(0, "06_attach_staged", notes)?;
    let desk = demo.sides[0].desk()?;
    desk.click(demo.host, None, "composer-send")?;
    notes.add(format!("attached with the composer's attach button ({chosen})"));
    let wanted = name.clone();
    demo.wait(0, limit, "the sent attachment", |desk, host| {
        desk.store(host, |s| {
            s.objects().any(|o| {
                o.direction == Direction::Outbound
                    && o.name.as_deref() == Some(wanted.as_str())
                    && !matches!(o.transfer, TransferState::Transferring { .. })
            })
        })
    })?;
    let sent = demo.read(0, |s| {
        s.objects()
            .find(|o| {
                o.direction == Direction::Outbound && o.name.as_deref() == Some(name.as_str())
            })
            .map(|o| (o.transfer.clone(), o.size))
    })?;
    if let Some((TransferState::Failed { reason }, _)) = &sent {
        return Err(format!("Alice's attachment failed: {reason}"));
    }
    notes.add(format!("Alice: {name} listed as sent ({sent:?})"));
    demo.wait(1, limit, "the attachment's message", |desk, host| {
        desk.store(host, |s| s.context_moment_count(&context) > before)
    })?;
    notes.add("Bob: the message arrived");
    let received = demo.read(1, |s| {
        s.objects()
            .filter(|o| o.direction == Direction::Inbound)
            .map(|o| o.transfer.clone())
            .collect::<Vec<_>>()
    })?;
    if received.contains(&TransferState::Complete) {
        notes.add("Bob: an incoming object is listed as available; compare its bytes by hand");
    } else {
        notes.add(format!(
            "Bob: attachment not fetched ({} incoming objects listed; the daemon cannot fetch a \
             received attachment yet)",
            received.len()
        ));
    }
    demo.shots(0, "06_attachment", notes)?;
    demo.shots(1, "06_attachment", notes)?;
    Ok(())
}
