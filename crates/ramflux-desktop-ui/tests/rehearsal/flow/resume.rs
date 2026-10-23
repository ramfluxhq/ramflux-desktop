// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Step 7: Alice locks and unlocks, then her daemon restarts; the
//! conversation's history is there each time.

use ramflux_desktop_core::{LockState, SourceStatus};
use ramflux_desktop_ui::route::Page;
use ramflux_desktop_ui::settings::SettingsSection;

use super::{Demo, probe};
use crate::rehearsal::daemon::DemoDaemon;
use crate::rehearsal::host::Host;
use crate::rehearsal::steps::Notes;

/// Step 7 on Alice's side.
pub fn lock_and_restart<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    notes: &mut Notes,
) -> Result<(), String> {
    let limit = demo.config.limits.answer;
    let desk = demo.sides[0].desk()?;
    desk.open_settings(demo.host, SettingsSection::Account);
    desk.click(demo.host, None, "settings-lock")?;
    demo.wait(0, limit, "the entry page after lock", |desk, host| {
        matches!(desk.page(host), Page::Entry(_))
            && desk.store(host, |s| {
                probe::account(s).is_some_and(|a| a.lock_state == LockState::Locked)
            })
    })?;
    demo.sides[0].desk()?.close_settings(demo.host);
    notes.add("Alice locked: back on the entry page");
    demo.shots(0, "07_locked", notes)?;
    unlock(demo)?;
    history(demo)?;
    notes.add("unlocked: history shown");

    demo.sides[0].daemon.restart()?;
    // A restarted daemon opens its accounts locked; the desktop shows that
    // once it has reconnected. (Before it notices, the old state is shown.)
    let discover = demo.config.limits.discover;
    let locked = demo.wait(0, discover.max(limit / 3), "the restarted daemon", |desk, host| {
        desk.store(host, |s| {
            s.source_status() == SourceStatus::Connected
                && probe::account(s).is_some_and(|a| a.lock_state == LockState::Locked)
        })
    });
    if locked.is_ok() {
        notes.add("daemon restarted: the account is locked again");
        demo.shots(0, "07_restarted_locked", notes)?;
        unlock(demo)?;
    } else {
        demo.wait(0, limit, "the restarted daemon", |desk, host| {
            desk.page(host) == Page::Workspace
                && desk.store(host, |s| s.source_status() == SourceStatus::Connected)
        })?;
        notes.add("daemon restarted: the account stayed open");
    }
    history(demo)?;
    notes.add("history kept after the restart");
    demo.shots(0, "07_restarted", notes)
}

/// Unlocks Alice from the entry page with her passphrase.
fn unlock<H: Host, D: DemoDaemon>(demo: &mut Demo<'_, H, D>) -> Result<(), String> {
    let secret = demo.sides[0].passphrase.clone();
    let desk = demo.sides[0].desk()?;
    desk.type_into(demo.host, "passphrase", &secret)?;
    desk.click(demo.host, None, "unlock")?;
    let limit = demo.config.limits.answer;
    demo.wait(0, limit, "the workspace after unlock", |desk, host| {
        desk.page(host) == Page::Workspace && desk.store(host, probe::unlocked)
    })
    .map(drop)
}

/// Shows the conversation and waits for every message of step 5.
fn history<H: Host, D: DemoDaemon>(demo: &mut Demo<'_, H, D>) -> Result<(), String> {
    let context = demo.conversation.clone();
    let limit = demo.config.limits.answer;
    let texts = demo.texts.clone();
    let desk = demo.sides[0].desk()?;
    desk.until(demo.host, limit, "the conversation", |desk, host| {
        desk.store(host, |s| s.context(&context).is_some())
    })?;
    desk.show_context(demo.host, &context)?;
    demo.wait(0, limit, "the conversation's history", |desk, host| {
        desk.store(host, |s| texts.iter().all(|t| probe::message(s, &context, t).is_some()))
    })
    .map(drop)
}
