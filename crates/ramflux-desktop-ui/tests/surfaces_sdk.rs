// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The production window with agent surfaces from a daemon (a fake one that
//! holds them on the protocol's state machine, as the pinned daemon
//! 8eebac1 answers): a streamed reply shows its writing mark until the
//! daemon reports the stream final, and its last-updated time; a surface
//! out of sync with its author says so, refuses actions, and asks the author
//! for its current state (`surface.request_resync`) with the daemon's typed
//! answer shown in the surface; an action the daemon refuses because the
//! surface changed asks the user to review it again; a surface the daemon
//! removes leaves the conversation.

mod common;
mod sdk_fake;

use common::*;
use gpui_kit::TestAppContext;
use ramflux_desktop_core::{SurfaceId, SurfaceState, SyncMode, UpdateRequest};
use ramflux_desktop_ui::route::Page;
use sdk_fake::author::{Bot, GROUP, scenario_surfaces};
use sdk_fake::wait;

fn version(
    cx: &mut TestAppContext,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::Shell>,
    reference: &str,
) -> u64 {
    with_store(cx, shell, |s| {
        s.surface_moment(&SurfaceId::new(reference))
            .and_then(|m| s.surface(m))
            .map_or(0, |r| r.version)
    })
}

fn moment_id(reference: &str) -> String {
    format!("moment-surface:{GROUP}:{reference}")
}

#[gpui_kit::test]
fn surfaces_from_the_daemon_stream_go_out_of_sync_and_ask_for_review(cx: &mut TestAppContext) {
    let (fake, state, path) = sdk_fake::start("ui-surfaces");
    fake.enable_stream();
    let (handle, shell) = open_sized(cx, sdk_fake::options(&path), 1200.);
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the stream", |cx| {
        with_store(cx, &shell, |s| s.sync_status().is_some_and(|x| x.mode == SyncMode::Stream))
    });
    show_context(cx, handle, &shell, GROUP);
    let mut bot = Bot::new();

    // A streamed reply: the writing mark shows while it grows.
    let s10 = scenario_surfaces("s10_ai_participant");
    let (reply, _) = bot.send(&fake, &state, &s10[0], true);
    wait(cx, "the reply", |cx| version(cx, &shell, &reply) == 1);
    let (_, accepted) = bot.send(&fake, &state, &s10[1], true);
    assert!(accepted);
    wait(cx, "the first append", |cx| version(cx, &shell, &reply) == 2);
    in_window(cx, handle, |_, _| {});
    assert!(exists_in(cx, handle, &moment_id(&reply), "surface-streaming"));
    // The last append is final: the mark ends with it, not later.
    for event in &s10[2..4] {
        assert!(bot.send(&fake, &state, event, true).1);
    }
    wait(cx, "the final append", |cx| version(cx, &shell, &reply) == 4);
    in_window(cx, handle, |_, _| {});
    assert!(!exists_in(cx, handle, &moment_id(&reply), "surface-streaming"));
    // Posted, then updated 90 s later by the author's clock (another minute).
    assert!(exists_in(cx, handle, &moment_id(&reply), "surface-updated"));

    // A choice; the daemon then knows it missed the author's change.
    let s11 = scenario_surfaces("s11_surface_versions");
    let (pick, _) = bot.send(&fake, &state, &s11[0], true);
    wait(cx, "the choice", |cx| version(cx, &shell, &pick) == 1);
    let surfaces = state.lock().expect("state").surfaces.clone();
    surfaces.set_out_of_sync(&pick, true);
    Bot::announce(&fake, &state, &pick);
    let out_of_sync = |cx: &mut TestAppContext| {
        with_store(cx, &shell, |s| {
            s.surface_moment(&SurfaceId::new(&pick))
                .and_then(|m| s.surface(m))
                .map(|r| (r.state.clone(), r.update.clone()))
        })
    };
    wait(cx, "out of sync", |cx| {
        out_of_sync(cx).is_some_and(|(state, _)| state == SurfaceState::OutOfSync)
    });
    in_window(cx, handle, |_, _| {});
    let scope = moment_id(&pick);
    assert!(exists_in(cx, handle, &scope, "surface-out-of-sync"));
    // Actions are paused with the reason, never sent.
    click_in(cx, handle, &scope, "action-thu");
    advance(cx, 200);
    assert!(surfaces.acted().is_empty(), "no action leaves an out-of-sync surface");

    // Request update, asked for already in the daemon's window: its typed
    // rate_limited answer shows in the surface.
    surfaces.asked_recently(&pick);
    click_in(cx, handle, &scope, "surface-request-update");
    wait(cx, "the refusal", |cx| {
        out_of_sync(cx).is_some_and(|(_, update)| {
            update == Some(UpdateRequest::Refused("backend.surface.rate_limited".into()))
        })
    });
    in_window(cx, handle, |_, _| {});
    assert!(exists_in(cx, handle, &scope, "surface-update-answer"));
    assert!(surfaces.resyncs().is_empty());
    // The next window: sent to the author, whose state ends it.
    surfaces.next_rate_window();
    click_in(cx, handle, &scope, "surface-request-update");
    wait(cx, "sent", |cx| out_of_sync(cx).is_some_and(|(_, u)| u == Some(UpdateRequest::Sent)));
    assert_eq!(surfaces.resyncs(), std::slice::from_ref(&pick));
    surfaces.set_out_of_sync(&pick, false);
    bot.send(&fake, &state, &s11[1], true);
    wait(cx, "in sync at v2", |cx| {
        version(cx, &shell, &pick) == 2
            && out_of_sync(cx).is_some_and(|(state, _)| state == SurfaceState::Open)
    });
    in_window(cx, handle, |_, _| {});
    assert!(!exists_in(cx, handle, &scope, "surface-out-of-sync"));

    // The author changes it again before this window reads it; the user
    // acts on v2: the daemon refuses the stale action, the surface asks for
    // a new review.
    let (_, accepted) = bot.send(&fake, &state, &s11[3], false);
    assert!(accepted, "v3 against v2");
    click_in(cx, handle, &scope, "action-thu");
    wait(cx, "the review notice", |cx| {
        let workspace = workspace(cx, &shell);
        let moment = ramflux_desktop_core::MomentId::new(format!("surface:{GROUP}:{pick}"));
        cx.update(|cx| workspace.read(cx).conversation().read(cx).is_review_again(&moment))
    });
    assert!(surfaces.acted().is_empty(), "the stale action was not sent");
    in_window(cx, handle, |_, _| {});
    assert!(exists_in(cx, handle, &scope, "surface-review-again"));

    // The daemon drops the reply: it leaves the conversation.
    surfaces.remove(&reply);
    Bot::announce(&fake, &state, &reply);
    wait(cx, "removed", |cx| {
        with_store(cx, &shell, |s| s.surface_moment(&SurfaceId::new(&reply)).is_none())
    });
    in_window(cx, handle, |_, _| {});
    assert!(
        with_store(cx, &shell, |s| s.surface_moment(&SurfaceId::new(&pick)).is_some()),
        "the other surface stays"
    );
    assert!(!exists_in(cx, handle, &scope, "surface-streaming"), "a choice is not written");
    fake.shutdown();
}
