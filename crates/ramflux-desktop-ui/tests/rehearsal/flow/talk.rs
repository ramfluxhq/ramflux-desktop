// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Steps 4 to 6: contacts both ways, a conversation started from the
//! contacts dock with messages both ways, and an attachment sent with the
//! composer's attach button.

use ramflux_desktop_core::{ContactLink, ContextId, ParticipantId, SafetyState};

use super::setup::short;
use super::{Demo, files, probe};
use crate::rehearsal::daemon::DemoDaemon;
use crate::rehearsal::host::Host;
use crate::rehearsal::steps::Notes;

/// Step 4: contacts both ways through the contacts dock, one way each:
/// Bob shows his contact card (Show my card) and Alice adds Bob by pasting
/// the card his desktop shows; Bob adds Alice by her principal id, which the
/// form says is trusted on first use. Each contact must be listed as
/// accepted and reachable (the daemon resolved it when it was added); then
/// each side verifies the other's safety number where the daemon offers it.
pub fn contacts<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    notes: &mut Notes,
) -> Result<(), String> {
    let card = shown_card(demo, 1, notes)?;
    for (me, peer, by_card) in [(0, 1, true), (1, 0, false)] {
        let id = demo.sides[peer].facts()?.principal.clone();
        let text = if by_card { card.clone() } else { id.clone() };
        let desk = demo.sides[me].desk()?;
        if !desk.exists(demo.host, "contact-add-id") {
            desk.click(demo.host, None, "rail-contacts")?;
        }
        // The card Alice imported asks Bob: the daemon holds her request, and
        // Bob answers it (adding her by id is refused while it waits).
        let request = format!("contacts-request-{id}");
        let asked = !by_card
            && demo
                .wait(me, demo.config.limits.discover, "the contact request", |desk, host| {
                    desk.exists(host, &request)
                })
                .is_ok();
        let desk = demo.sides[me].desk()?;
        let trust = if asked {
            desk.click(demo.host, Some(&request), "request-accept")?;
            "accepted the request".to_owned()
        } else {
            desk.type_into(demo.host, "contact-add-id", &text)?;
            let trust = desk.label(demo.host, "contact-add-kind").unwrap_or_default();
            desk.click(demo.host, None, "contact-add")?;
            trust
        };
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
        let how =
            if by_card { "from the contact card Bob's desktop shows" } else { "by principal id" };
        notes.add(format!(
            "{} added {} {how} ({}); the form said: {trust}",
            demo.sides[me].name,
            demo.sides[peer].name,
            short(&id)
        ));
        let reach = demo.wait(me, limit, "the contact reachable", |desk, host| {
            desk.store(host, |s| {
                s.participant(&participant).is_some_and(|p| p.reachable == Some(true))
            })
        });
        let shown = demo.read(me, |s| {
            s.participant(&participant).map(|p| (p.reachable, p.safety, p.kind, p.kind_claim))
        })?;
        if reach.is_err() {
            return Err(format!(
                "{} cannot reach {} after adding them: {shown:?} (the daemon did not resolve the \
                 contact)",
                demo.sides[me].name, demo.sides[peer].name
            ));
        }
        notes.add(format!(
            "{}: {} is reachable at once ({shown:?})",
            demo.sides[me].name, demo.sides[peer].name
        ));
        demo.shots(me, if by_card { "04_add_by_card" } else { "04_add_by_id" }, notes)?;
        verify(demo, me, &participant, &row, notes)?;
        demo.shots(me, "04_contacts", notes)?;
        demo.sides[me].desk()?.click(demo.host, None, "rail-contacts")?;
    }
    Ok(())
}

/// Shows side `ix`'s own contact card in its contacts dock and returns the
/// card as the desktop shows it (its link when the card text spans lines,
/// which one field cannot hold).
pub(super) fn shown_card<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    ix: usize,
    notes: &mut Notes,
) -> Result<String, String> {
    let desk = demo.sides[ix].desk()?;
    if !desk.exists(demo.host, "contact-card-show") {
        desk.click(demo.host, None, "rail-contacts")?;
    }
    desk.click(demo.host, None, "contact-card-show")?;
    let limit = demo.config.limits.answer;
    let shown = demo.wait(ix, limit, "the contact card", |desk, host| {
        desk.label(host, "contact-card-text").is_some()
    });
    if let Err(error) = shown {
        let status = demo.sides[ix].desk()?.label(demo.host, "contact-card-status");
        return Err(status.map_or(error.clone(), |status| format!("{error}: {status}")));
    }
    let desk = demo.sides[ix].desk()?;
    let text = desk.label(demo.host, "contact-card-text").unwrap_or_default();
    let qr = desk.exists(demo.host, "contact-card-qr");
    let card = if text.contains('\n') {
        demo.read(ix, |s| s.own_contact_card().and_then(|c| c.link.clone()))?.unwrap_or(text)
    } else {
        text
    };
    notes.add(format!(
        "{} showed the contact card ({} bytes{})",
        demo.sides[ix].name,
        card.len(),
        if qr { ", with its QR code" } else { "" }
    ));
    demo.shots(ix, "04_my_card", notes)?;
    demo.sides[ix].desk()?.click(demo.host, None, "contact-card-hide")?;
    Ok(card)
}

/// Presses Verify on the contact row when the daemon offers it, then "The
/// numbers match" in the comparison it opens, and notes the safety state
/// either way.
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
        // Verify opens the comparison; the numbers match is the confirmation.
        let _ = desk.click(demo.host, Some(row), "verify");
        demo.host.tick();
        let _ = desk.click(demo.host, None, "verify-confirm");
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
    if let Err(error) = start(demo, &bob.principal, &opener, notes) {
        if let Ok(links) =
            demo.sides[0].daemon.cli(&["contact", "list", &format!("--account={}", alice.account)])
        {
            notes.add(format!("Alice's daemon: rf contact list {links}"));
        }
        return Err(error);
    }
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
        demo.fallbacks.push("rf dm read for Bob's first DM".to_owned());
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
    // Accepting the request made the direct conversation already: Message
    // opens it; otherwise it opens a new-conversation draft.
    if !desk.draft_open(demo.host) {
        let Some(existing) = desk.current_context(demo.host) else {
            return Err("Message on the contact opened neither a conversation nor a draft".into());
        };
        notes.add(format!("Message opened the conversation {}", existing.as_str()));
        demo.conversation = existing;
        return send(demo, 0, text);
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

/// Step 6: files both ways: Alice sends Bob a file and Bob sends Alice an
/// image, each with the composer's attach button (prepared media: the file
/// is streamed into the daemon, then sent); each receiver presses Download
/// on the received file and the downloaded bytes must be the ones sent.
/// The fake daemon has no prepared-media API: there the step says so.
pub fn attachment<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    notes: &mut Notes,
) -> Result<(), String> {
    if demo.config.limits.fetch.is_none() {
        notes.add("not run: the fake daemon has no prepared-media API (attachment.prepare.*)");
        return Ok(());
    }
    let context = demo.conversation.clone();
    let run = demo.config.run_id.clone();
    let (file, file_bytes) =
        files::text_file(&demo.work, &format!("rehearsal-{run}.txt"), 700 * 1024)?;
    let (image, image_bytes) = files::image_file(&demo.work, &format!("rehearsal-{run}.png"))?;
    for (from, to, path, bytes) in [(0, 1, &file, &file_bytes), (1, 0, &image, &image_bytes)] {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        if let Err(error) = files::send_file(demo, from, &context, path, notes) {
            if !error.contains("backend.media.no_home") {
                return Err(error);
            }
            // The honest "not sent": no home node is pinned for the
            // contact (added by answering a request). The sender adds the
            // contact's card, as the row offers, and retries.
            notes.add(format!(
                "{}: {name} not sent, shown with its reason ({error})",
                demo.sides[from].name
            ));
            demo.shots(from, "06_not_sent_no_home", notes)?;
            let card = shown_card(demo, to, notes)?;
            let desk = demo.sides[from].desk()?;
            if !desk.exists(demo.host, "contact-add-id") {
                desk.click(demo.host, None, "rail-contacts")?;
            }
            desk.type_into(demo.host, "contact-add-id", &card)?;
            desk.click(demo.host, None, "contact-add")?;
            let mut note = None;
            let pause = std::time::Instant::now();
            while pause.elapsed() < std::time::Duration::from_secs(5) {
                demo.host.tick();
                note = demo.sides[from].desk()?.label(demo.host, "contact-add-note");
            }
            notes.add(format!(
                "{} added {}'s contact card to pin the home node; the form said: {note:?}",
                demo.sides[from].name, demo.sides[to].name
            ));
            files::retry_file(demo, from, &context, &name, notes)?;
        }
        demo.shots(from, &format!("06_sent_{}", if from == 0 { "file" } else { "image" }), notes)?;
        let got = files::receive_file(demo, to, &context, &name, notes)?;
        if &got != bytes {
            return Err(format!(
                "{name}: {} downloaded {} bytes that differ from the {} sent",
                demo.sides[to].name,
                got.len(),
                bytes.len()
            ));
        }
        notes.add(format!("{}: {name} identical to what was sent", demo.sides[to].name));
        demo.shots(
            to,
            &format!("06_received_{}", if from == 0 { "file" } else { "image" }),
            notes,
        )?;
    }
    Ok(())
}
