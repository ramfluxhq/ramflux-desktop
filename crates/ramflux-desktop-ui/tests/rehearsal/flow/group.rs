// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The group step: Alice makes a group with Bob on her desktop, and both
//! write in it.
//!
//! Alice opens the context stack's "+" and chooses New group…, names the
//! group, ticks Bob among her contacts and presses Create group; the form
//! closes once her daemon lists the group conversation, which is then
//! shown. Everything after that is the desktops too: each sends from its
//! composer and sees the other's message arrive. Bob's desktop must learn
//! the group conversation from his daemon; nothing is done with the
//! reference CLI, so a daemon that does not deliver it fails the step with
//! what was waited for.

use ramflux_desktop_core::{ContextId, ParticipantId};
use ramflux_desktop_ui::i18n::tr;

use super::setup::short;
use super::{Demo, probe};
use crate::rehearsal::daemon::DemoDaemon;
use crate::rehearsal::host::Host;
use crate::rehearsal::steps::Notes;

/// The group step.
pub fn group<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    notes: &mut Notes,
) -> Result<(), String> {
    let bob = demo.sides[1].facts()?.clone();
    let run = demo.config.run_id.clone();
    let limits = demo.config.limits;
    let name = format!("Rehearsal group {run}");

    let desk = demo.sides[0].desk()?;
    desk.click(demo.host, None, "stack-new")?;
    desk.click_label(demo.host, &tr("stack.new_group"))?;
    let Some(Some(context)) = desk.group_form(demo.host) else {
        return Err("New group… opened no form with a fresh group id".to_owned());
    };
    desk.type_into(demo.host, "new-group-name", &name)?;
    let member = format!("new-group-member-{}", bob.principal);
    desk.click(demo.host, None, &member)
        .map_err(|e| format!("{e}: Bob is not offered as a member ({member})"))?;
    demo.shots(0, "07_new_group", notes)?;
    let desk = demo.sides[0].desk()?;
    desk.click(demo.host, None, "new-group-create")?;
    let created = demo.wait(0, limits.answer, "the group conversation shown", |desk, host| {
        desk.current_context(host).as_ref() == Some(&context)
    });
    if let Err(error) = created {
        let status = demo.sides[0].desk()?.label(demo.host, "new-group-status");
        return Err(status.map_or(error.clone(), |status| format!("{error}: {status}")));
    }
    let shown = demo.read(0, |s| s.context(&context).cloned())?.ok_or("no group context")?;
    if shown.title != name {
        return Err(format!("the group is shown as {:?}, not {name:?}", shown.title));
    }
    if !shown.participants.contains(&ParticipantId::new(&bob.principal)) {
        return Err(format!("Bob is not a member of the new group: {:?}", shown.participants));
    }
    notes.add(format!(
        "Alice made the group with the New group form (name {name:?}, Bob {} ticked); \
         conversation {}",
        short(&bob.principal),
        short(context.as_str())
    ));

    let opener = format!("Group hello from Alice ({run})");
    send(demo, 0, &context, &opener)?;
    demo.wait(1, limits.discover, "the group conversation", |desk, host| {
        desk.store(host, |s| s.context(&context).is_some())
    })?;
    let bob_title = demo.read(1, |s| s.context(&context).map(|c| c.title.clone()))?;
    notes.add(format!(
        "Bob's desktop got the group conversation from his daemon (shown as {:?}; the name is \
         Alice's own)",
        bob_title.unwrap_or_default()
    ));
    demo.sides[1].desk()?.show_context(demo.host, &context)?;
    see(demo, 1, &context, &opener)?;
    let reply = format!("Group reply from Bob ({run})");
    send(demo, 1, &context, &reply)?;
    see(demo, 0, &context, &reply)?;
    let again = format!("Group note from Alice ({run})");
    send(demo, 0, &context, &again)?;
    see(demo, 1, &context, &again)?;
    notes.add("messages both ways from the composers, each seen on the other desktop");
    let kept = demo.read(0, |s| s.context(&context).map(|c| c.title.clone()))?;
    if kept.as_deref() != Some(name.as_str()) {
        return Err(format!("Alice's group name did not survive the group reads: {kept:?}"));
    }
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
