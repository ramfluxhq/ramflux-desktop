// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Live step: a group with Alice and Bob, and messages both ways in it.
//!
//! The desktop has no "new group" control (it governs groups the daemon
//! lists: members, roles, invitations), so the group is made with the
//! reference CLI on Alice's daemon: `group create` with her device as owner,
//! then `group member add` for Bob's device with his commitment and delivery
//! target. Everything after that is the desktops: each sends from its
//! composer, and each sees the other's message arrive through its event
//! stream with polling off. If the daemon lists no conversation for a new
//! group until a first message names one, that first message is sent with
//! `rf group send` and the step says so.

use ramflux_desktop_core::{ContextId, ContextKind, Store};

use super::setup::short;
use super::{Demo, probe};
use crate::rehearsal::daemon::DemoDaemon;
use crate::rehearsal::host::Host;
use crate::rehearsal::steps::Notes;

/// The group context of a store (each rehearsal account is in one group).
fn group_context(store: &Store) -> Option<ContextId> {
    store.contexts().find(|c| c.kind == ContextKind::Group).map(|c| c.id.clone())
}

/// The live group step.
pub fn group<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    notes: &mut Notes,
) -> Result<(), String> {
    let (alice, bob) = (demo.sides[0].facts()?.clone(), demo.sides[1].facts()?.clone());
    let run = demo.config.run_id.clone();
    let limits = demo.config.limits;
    let group = format!("rehearsal-group-{run}");
    let created = demo.sides[0].daemon.cli(&[
        "group",
        "create",
        "--account",
        &alice.account,
        "--group",
        &group,
        "--creator",
        &alice.device,
        "--creator-target-delivery",
        &alice.target,
    ])?;
    notes.add(format!(
        "rf group create {group} on Alice's daemon: roles {}",
        created.get("roles").cloned().unwrap_or_default()
    ));
    let added = demo.sides[0].daemon.cli(&[
        "group",
        "member",
        "add",
        "--account",
        &alice.account,
        "--group",
        &group,
        "--member-device",
        &bob.device,
        "--member-principal-commitment",
        &bob.commitment,
        "--target-delivery",
        &bob.target,
    ])?;
    if added["roles"][bob.device.as_str()] != "member" {
        return Err(format!("Bob is not a member after group member add: {added}"));
    }
    notes.add(format!("rf group member add: Bob's device {} is member", short(&bob.device)));

    let listed = demo.wait(0, limits.discover, "the group conversation", |desk, host| {
        desk.store(host, |s| group_context(s).is_some())
    });
    let opener = format!("Group hello from Alice ({run})");
    let mut opened_by_cli = false;
    if listed.is_err() {
        let conversation = format!("rehearsal-group-conv-{run}");
        let message = format!("rehearsal-group-msg-{run}");
        demo.sides[0].daemon.cli(&[
            "group",
            "send",
            "--account",
            &alice.account,
            "--group",
            &group,
            "--conversation",
            &conversation,
            "--message",
            &message,
            "--sender",
            &alice.device,
            "--body",
            &opener,
        ])?;
        opened_by_cli = true;
        notes.add(format!(
            "the daemon listed no conversation for the new group; first message with rf group \
             send ({conversation})"
        ));
        demo.wait(0, limits.answer, "the group conversation", |desk, host| {
            desk.store(host, |s| group_context(s).is_some())
        })?;
    } else {
        notes.add("Alice's desktop listed the group conversation after create");
    }
    let context = demo.read(0, group_context)?.ok_or("no group context")?;
    notes.add(format!("group conversation {}", context.as_str()));
    demo.sides[0].desk()?.show_context(demo.host, &context)?;
    if !opened_by_cli {
        send(demo, 0, &context, &opener)?;
    }
    see(demo, 0, &context, &opener)?;

    let found = demo.wait(1, limits.discover, "the group conversation", |desk, host| {
        desk.store(host, |s| s.context(&context).is_some())
    });
    if found.is_err() {
        demo.sides[1].daemon.cli(&[
            "group",
            "read",
            "--account",
            &bob.account,
            "--group",
            &group,
            "--conversation",
            context.as_str(),
        ])?;
        demo.fallbacks.push("rf group read for Bob's group conversation".to_owned());
        notes.add(
            "Bob's desktop did not get the group conversation from the event stream in time; \
             the reference CLI received it (rf group read)",
        );
        demo.wait(1, limits.answer, "the group conversation", |desk, host| {
            desk.store(host, |s| s.context(&context).is_some())
        })?;
    } else {
        notes.add("Bob's desktop got the group conversation from the event stream");
    }
    demo.sides[1].desk()?.show_context(demo.host, &context)?;
    see(demo, 1, &context, &opener)?;
    let reply = format!("Group reply from Bob ({run})");
    send(demo, 1, &context, &reply)?;
    see(demo, 0, &context, &reply)?;
    let again = format!("Group note from Alice ({run})");
    send(demo, 0, &context, &again)?;
    see(demo, 1, &context, &again)?;
    notes.add("both group messages from the composers seen on the other desktop");
    demo.group = Some((context, vec![opener, reply, again]));
    demo.shots(0, "07_group", notes)?;
    demo.shots(1, "07_group", notes)?;
    Ok(())
}

fn send<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    ix: usize,
    context: &ContextId,
    text: &str,
) -> Result<(), String> {
    let desk = demo.sides[ix].desk()?;
    desk.show_context(demo.host, context)?;
    if let Some(reason) = desk.label(demo.host, "composer-unavailable") {
        return Err(format!("{} cannot send in the group: {reason}", demo.sides[ix].name));
    }
    desk.type_into(demo.host, "composer-input", text)?;
    desk.click(demo.host, None, "composer-send")?;
    see(demo, ix, context, text)
}

fn see<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    ix: usize,
    context: &ContextId,
    text: &str,
) -> Result<(), String> {
    let limit = demo.config.limits.answer;
    let context = context.clone();
    demo.wait(ix, limit, "the group message", |desk, host| {
        desk.store(host, |s| probe::message(s, &context, text).is_some())
    })
    .map(drop)
}
