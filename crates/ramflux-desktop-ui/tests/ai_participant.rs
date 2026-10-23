// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI integration tests of the AI participant in the conversation: a
//! streamed reply, a task proposal started from its card, a step that needs
//! the user (approve with the hold, deny), cancel by the requester, asking
//! from the composer and the command palette, a choice answered in the
//! agent's surface, a surface that changed under review or is out of date,
//! a closed surface, and surfaces over the renderer limits. Each drives the
//! production window with real input and asserts what reached the source.

mod common;

use common::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, Entity, TestAppContext};
use ramflux_desktop_core::{
    IntentKind, Lifecycle, MomentId, ParticipantId, StepState, TaskId, TaskState, methods,
};
use ramflux_desktop_ui::Shell;
use ramflux_desktop_ui::workspace::ComposerMode;

fn task(cx: &mut TestAppContext, shell: &Entity<Shell>, id: &str) -> ramflux_desktop_core::Task {
    with_store(cx, shell, |s| s.task(&TaskId::new(id)).cloned()).expect("task")
}

fn lifecycle(cx: &mut TestAppContext, shell: &Entity<Shell>, id: &str) -> Option<Lifecycle> {
    with_store(cx, shell, |s| s.moment(&MomentId::new(id)).map(|m| m.lifecycle))
}

fn count(cx: &mut TestAppContext, shell: &Entity<Shell>, method: &str) -> usize {
    effects(cx, shell).iter().filter(|m| *m == method).count()
}

/// Top edge of `id` inside `scope`, in window coordinates.
fn top(
    cx: &mut TestAppContext,
    handle: gpui_kit::WindowHandle<gpui_kit::base::Root>,
    scope: &str,
    id: &str,
) -> f32 {
    let (scope, id) = (scope.to_owned(), id.to_owned());
    in_window(cx, handle, move |window, _| {
        f32::from(window.within(scope).find(id).bounds().origin.y)
    })
}

/// Opens demo 3 and asks Orbit in the group (a message, not a task).
fn asked(cx: &mut TestAppContext) -> (gpui_kit::WindowHandle<gpui_kit::base::Root>, Entity<Shell>) {
    let (handle, shell) = open(cx, "s10_ai_participant");
    advance(cx, 200);
    show_context(cx, handle, &shell, "c-launch");
    type_into(cx, handle, "composer-input", "@Orbit can you check the launch checklist?");
    click(cx, handle, "composer-send");
    assert_eq!(last_body(cx, &shell, methods::SEND_MESSAGE)["context_id"], "c-launch");
    (handle, shell)
}

/// Demo 3 up to the step that needs the user's approval.
fn at_approval(
    cx: &mut TestAppContext,
) -> (gpui_kit::WindowHandle<gpui_kit::base::Root>, Entity<Shell>) {
    let (handle, shell) = asked(cx);
    advance(cx, 1_100);
    click_in(cx, handle, "moment-m-task", "task-start");
    advance(cx, 1_400);
    (handle, shell)
}

#[gpui_kit::test]
fn a_reply_streams_in_place_without_moving_what_is_above_it(cx: &mut TestAppContext) {
    let (handle, shell) = asked(cx);
    advance(cx, 350);
    // Created with an empty text: one waiting line holds its place.
    assert!(exists_in(cx, handle, "moment-m-reply", "block-waiting"));
    assert!(exists_in(cx, handle, "moment-m-reply", "surface-streaming"));
    let block_top = top(cx, handle, "moment-m-reply", "block-b-reply");
    advance(cx, 200);
    assert!(!exists_in(cx, handle, "moment-m-reply", "block-waiting"), "text replaced the line");
    assert!(exists_in(cx, handle, "moment-m-reply", "surface-streaming"));
    assert_eq!(
        top(cx, handle, "moment-m-reply", "block-b-reply"),
        block_top,
        "appends grow downward"
    );
    advance(cx, 200);
    assert_eq!(top(cx, handle, "moment-m-reply", "block-b-reply"), block_top);
    advance(cx, 200);
    assert!(!exists_in(cx, handle, "moment-m-reply", "surface-streaming"), "final ends the mark");
    assert_eq!(
        top(cx, handle, "moment-m-reply", "block-b-reply"),
        block_top,
        "the mark sits outside the flow: ending the stream moves nothing"
    );
    let version =
        with_store(cx, &shell, |s| s.surface(&MomentId::new("m-reply")).map(|r| r.version));
    assert_eq!(version, Some(4));
    assert!(
        effects(cx, &shell).iter().all(|m| m != methods::SURFACE_ACTION),
        "reading sends nothing"
    );
}

#[gpui_kit::test]
fn a_proposal_is_started_from_its_card_and_needs_me_once(cx: &mut TestAppContext) {
    let (handle, shell) = asked(cx);
    advance(cx, 1_100);
    assert_eq!(lifecycle(cx, &shell, "m-task"), Some(Lifecycle::NeedsMe));
    assert!(exists_in(cx, handle, "moment-m-task", "task-start"));
    assert!(exists_in(cx, handle, "moment-m-task", "step-notes"), "a proposal shows its plan");
    click_in(cx, handle, "moment-m-task", "task-start");
    let body = last_body(cx, &shell, methods::TASK_START);
    assert_eq!(
        (body["proposal_id"].as_str(), body["assignee"].as_str()),
        (Some("t-launch"), Some("p-orbit"))
    );
    assert_eq!(body["proposed_by"], "Human");
    advance(cx, 1_400);
    assert_eq!(task(cx, &shell, "t-launch").state, TaskState::NeedsMe);
    assert!(exists_in(cx, handle, "moment-m-task", "step-decision"), "the approval is in the step");
    assert!(!exists_in(cx, handle, "moment-m-task", "task-start"));
    let attention = with_store(cx, &shell, |s| ramflux_desktop_core::selectors::attention(s).len());
    assert_eq!(attention, 1, "one decision, one place");
}

#[gpui_kit::test]
fn approving_the_step_needs_the_hold_then_the_result_is_patched_in(cx: &mut TestAppContext) {
    let (handle, shell) = at_approval(cx);
    click_in(cx, handle, "moment-m-task", "step-approve");
    assert!(strip_open(cx, &shell), "high risk: the hold-to-confirm strip");
    assert_eq!(count(cx, &shell, methods::GRANT_APPROVE), 0, "nothing sent before the hold");
    hold_confirm(cx, handle);
    let body = last_body(cx, &shell, methods::GRANT_APPROVE);
    assert_eq!(body["grant_id"], "g-deploy");
    assert_eq!(body["confirmed_by"]["gesture"], "Hold");
    assert_eq!(body["proposed_by"], "Human");
    advance(cx, 900);
    assert!(
        exists_in(cx, handle, "moment-m-status", "block-b-done"),
        "the result patched in place"
    );
    assert!(!exists_in(cx, handle, "moment-m-status", "surface-conflict"));
    advance(cx, 700);
    assert_eq!(task(cx, &shell, "t-launch").state, TaskState::Done);
    assert_eq!(lifecycle(cx, &shell, "m-task"), Some(Lifecycle::Done));
    assert!(exists_in(cx, handle, "moment-m-task", "task-result"));
    click_in(cx, handle, "moment-m-task", "task-show-result");
    let ws = workspace(cx, &shell);
    let highlighted = cx.update(|cx| ws.read(cx).conversation().read(cx).highlighted().cloned());
    assert_eq!(highlighted, Some(MomentId::new("m-status")), "the result surface comes into view");
    assert_eq!(count(cx, &shell, methods::GRANT_APPROVE), 1, "sent once");
}

#[gpui_kit::test]
fn denying_the_step_fails_the_task(cx: &mut TestAppContext) {
    let (handle, shell) = at_approval(cx);
    click_in(cx, handle, "moment-m-task", "step-deny");
    assert!(!strip_open(cx, &shell), "denying needs no hold");
    assert_eq!(last_body(cx, &shell, methods::GRANT_DENY)["grant_id"], "g-deploy");
    advance(cx, 400);
    let t = task(cx, &shell, "t-launch");
    assert_eq!(t.state, TaskState::Failed);
    assert_eq!(t.steps[2].state, StepState::Skipped);
    assert_eq!(lifecycle(cx, &shell, "m-task"), Some(Lifecycle::Failed));
    assert!(
        !exists_in(cx, handle, "moment-m-task", "task-cancel"),
        "a finished task cannot be cancelled"
    );
}

#[gpui_kit::test]
fn the_requester_declines_a_proposal_and_cancels_a_running_task(cx: &mut TestAppContext) {
    let (handle, shell) = asked(cx);
    advance(cx, 1_100);
    click_in(cx, handle, "moment-m-task", "task-cancel");
    assert_eq!(last_body(cx, &shell, methods::TASK_CANCEL)["task_id"], "t-launch");
    advance(cx, 200);
    assert_eq!(task(cx, &shell, "t-launch").state, TaskState::Cancelled);
    assert_eq!(lifecycle(cx, &shell, "m-task"), Some(Lifecycle::Dismissed));

    let (handle, shell) = at_approval(cx);
    assert!(exists_in(cx, handle, "moment-m-task", "task-cancel"));
    click_in(cx, handle, "moment-m-task", "task-cancel");
    assert_eq!(count(cx, &shell, methods::TASK_CANCEL), 1);
    advance(cx, 200);
    let t = task(cx, &shell, "t-launch");
    assert_eq!(t.state, TaskState::Cancelled);
    assert_eq!(t.steps[1].state, StepState::Skipped, "the step that waited is skipped");
    assert!(!exists_in(cx, handle, "moment-m-task", "step-decision"), "nothing left to decide");
}

#[gpui_kit::test]
fn agents_never_start_a_proposal_for_the_user(cx: &mut TestAppContext) {
    let (handle, shell) = asked(cx);
    advance(cx, 1_100);
    let root = cx.update(|cx| shell.read(cx).root().clone());
    let result = cx
        .update_window(handle.into(), |_, window, cx| {
            root.update(cx, |root, cx| {
                root.submit_agent(
                    ParticipantId::new("p-orbit"),
                    IntentKind::AcceptTaskProposal { task_id: TaskId::new("t-launch") },
                    window,
                    cx,
                )
            })
        })
        .expect("window");
    assert!(result.starts_with("refused"), "{result}");
    assert_eq!(count(cx, &shell, methods::TASK_START), 0);
    assert_eq!(task(cx, &shell, "t-launch").state, TaskState::Proposed);
}

#[gpui_kit::test]
fn the_composer_asks_an_ai_participant_by_mention_or_from_the_palette(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s10_ai_participant");
    advance(cx, 200);
    show_context(cx, handle, &shell, "c-launch");
    click(cx, handle, "composer-mode");
    let ws = workspace(cx, &shell);
    let composer = cx.update(|cx| ws.read(cx).composer().clone());
    assert_eq!(cx.update(|cx| composer.read(cx).mode()), ComposerMode::Task);
    type_into(cx, handle, "composer-input", "@orbit draft the release notes");
    click(cx, handle, "composer-send");
    let body = last_body(cx, &shell, methods::TASK_START);
    assert_eq!(body["conversation_id"], "c-launch");
    assert_eq!(body["assignee"], "p-orbit");
    assert_eq!(
        body["text"], "draft the release notes",
        "the mention addresses, it is not the request"
    );
    // The palette's "Ask Orbit to…" switches the composer to a request for Orbit.
    click(cx, handle, "composer-mode");
    assert_eq!(cx.update(|cx| composer.read(cx).mode()), ComposerMode::Message);
    cx.update_window(handle.into(), |_, window, cx| {
        ws.update(cx, |w, cx| w.ask_agent(ParticipantId::new("p-orbit"), window, cx));
    })
    .expect("window");
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| composer.read(cx).mode()), ComposerMode::Task);
    let named = cx.update(|cx| composer.read(cx).addressee(cx).map(|a| a.name));
    assert_eq!(named.as_deref(), Some("Orbit"));
}

#[gpui_kit::test]
fn a_conversation_without_an_ai_participant_has_nobody_to_ask(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    show_context(cx, handle, &shell, "c-lena");
    click(cx, handle, "composer-mode");
    let ws = workspace(cx, &shell);
    let mode = cx.update(|cx| ws.read(cx).composer().read(cx).mode());
    assert_eq!(mode, ComposerMode::Message);
}

#[gpui_kit::test]
fn a_choice_is_answered_in_the_surface_the_task_points_to(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s11_surface_versions");
    advance(cx, 300);
    show_context(cx, handle, &shell, "c-plan");
    assert!(
        exists_in(cx, handle, "moment-m-task", "step-pointer"),
        "the step points to the surface"
    );
    assert!(
        !exists_in(cx, handle, "moment-m-task", "step-decision"),
        "it never repeats the controls"
    );
    click_in(cx, handle, "moment-m-task", "step-show-view");
    let ws = workspace(cx, &shell);
    let highlighted = cx.update(|cx| ws.read(cx).conversation().read(cx).highlighted().cloned());
    assert_eq!(highlighted, Some(MomentId::new("m-pick")));
    advance(cx, 3_300);
    click_in(cx, handle, "moment-m-pick", "action-thu");
    let body = last_body(cx, &shell, methods::SURFACE_ACTION);
    assert_eq!((body["action_id"].as_str(), body["version"].as_u64()), (Some("thu"), Some(3)));
    let shown = with_store(cx, &shell, |s| s.surface_hash(&MomentId::new("m-pick")).cloned());
    assert_eq!(body["surface_hash"], serde_json::json!(shown));
    advance(cx, 1_500);
    assert!(exists_in(cx, handle, "moment-m-pick", "surface-closed"), "a frozen record");
    assert!(exists_in(cx, handle, "moment-m-pick", "surface-closed-text"));
    assert!(!exists_in(cx, handle, "moment-m-pick", "surface-actions"));
    // "Show" opened the row, so the final view is open; the disclosure
    // folds it back to the one-line record.
    assert!(exists_in(cx, handle, "moment-m-pick", "block-b-answer"));
    click_in(cx, handle, "moment-m-pick", "surface-disclosure");
    assert!(
        !exists_in(cx, handle, "moment-m-pick", "block-b-answer"),
        "folded to its fallback text"
    );
    assert!(exists_in(cx, handle, "moment-m-pick", "surface-closed-text"));
    assert_eq!(task(cx, &shell, "t-day").state, TaskState::Done);
    assert_eq!(count(cx, &shell, methods::SURFACE_ACTION), 1);
}

#[gpui_kit::test]
fn a_surface_changed_under_review_is_never_acted_on_silently(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s11_surface_versions");
    advance(cx, 300);
    show_context(cx, handle, &shell, "c-plan");
    click_in(cx, handle, "moment-m-pick", "action-fri");
    assert!(strip_open(cx, &shell), "a high-risk choice waits for the hold");
    advance(cx, 1_300);
    assert!(!strip_open(cx, &shell), "the change cancelled the pending hold");
    assert!(exists_in(cx, handle, "moment-m-pick", "surface-review-again"));
    assert_eq!(count(cx, &shell, methods::SURFACE_ACTION), 0, "no silent resend");
}

#[gpui_kit::test]
fn an_out_of_date_surface_refuses_actions_until_resent(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s11_surface_versions");
    advance(cx, 2_600);
    show_context(cx, handle, &shell, "c-plan");
    assert!(exists_in(cx, handle, "moment-m-pick", "surface-conflict"));
    click_in(cx, handle, "moment-m-pick", "action-thu");
    assert_eq!(count(cx, &shell, methods::SURFACE_ACTION), 0, "disabled while out of date");
    advance(cx, 1_000);
    assert!(!exists_in(cx, handle, "moment-m-pick", "surface-conflict"), "resolved by the resend");
    assert!(exists_in(cx, handle, "moment-m-pick", "block-b-note"));
}

#[gpui_kit::test]
fn surfaces_over_a_limit_show_only_their_fallback(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s12_surface_limits");
    advance(cx, 700);
    show_context(cx, handle, &shell, "c-limits");
    assert!(exists_in(cx, handle, "moment-m-many", "surface-over-limit"));
    assert!(!exists_in(cx, handle, "moment-m-many", "surface-actions"), "nothing to act on");
    assert!(exists_in(cx, handle, "moment-m-log", "block-b-log"), "within the limits it streams");
    advance(cx, 200);
    assert!(exists_in(cx, handle, "moment-m-log", "surface-over-limit"));
    assert!(!exists_in(cx, handle, "moment-m-log", "block-b-log"), "never a partial rendering");
    let errors = cx.update(|cx| {
        shell.read(cx).store().read(cx).session().map(|s| s.renderer_errors().total())
    });
    assert!(errors.is_some_and(|n| n >= 2), "both are renderer errors: {errors:?}");
}
