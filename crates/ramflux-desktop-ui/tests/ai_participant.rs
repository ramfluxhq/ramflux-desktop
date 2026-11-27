// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI integration tests of the AI participant in the conversation: a
//! streamed reply, a task proposal started from its card, a step that needs
//! the user (approve with the confirmation, deny), cancel by the requester, asking
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
fn denying_the_step_fails_the_task(cx: &mut TestAppContext) {
    let (handle, shell) = at_approval(cx);
    click_in(cx, handle, "moment-m-task", "step-deny");
    assert!(!strip_open(cx, &shell), "denying needs no confirmation");
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

/// The task row's header names the task's state with the card's words:
/// the state mark is drawn and the row's accessible name carries it.
fn names_state(
    cx: &mut TestAppContext,
    handle: gpui_kit::WindowHandle<gpui_kit::base::Root>,
    state: TaskState,
) {
    let word = ramflux_desktop_ui::i18n::task_state(state);
    assert!(exists_in(cx, handle, "moment-m-task", "row-state"), "{state:?} has a state mark");
    let label = in_window(cx, handle, |window, _| {
        window.find("moment-m-task").label().map(str::to_owned).unwrap_or_default()
    });
    assert!(label.contains(word.as_ref()), "{state:?}: {label:?} names {word:?}");
}

#[gpui_kit::test]
fn a_task_row_names_its_state_in_every_state(cx: &mut TestAppContext) {
    let (handle, shell) = asked(cx);
    advance(cx, 1_100);
    names_state(cx, handle, TaskState::Proposed);
    click_in(cx, handle, "moment-m-task", "task-cancel");
    advance(cx, 200);
    assert_eq!(task(cx, &shell, "t-launch").state, TaskState::Cancelled);
    names_state(cx, handle, TaskState::Cancelled);

    let (handle, shell) = at_approval(cx);
    assert_eq!(task(cx, &shell, "t-launch").state, TaskState::NeedsMe);
    names_state(cx, handle, TaskState::NeedsMe);
    click_in(cx, handle, "moment-m-task", "step-approve");
    tap_confirm(cx, handle);
    advance(cx, 1_600);
    assert_eq!(task(cx, &shell, "t-launch").state, TaskState::Done);
    names_state(cx, handle, TaskState::Done);

    let (handle, shell) = at_approval(cx);
    click_in(cx, handle, "moment-m-task", "step-deny");
    advance(cx, 400);
    assert_eq!(task(cx, &shell, "t-launch").state, TaskState::Failed);
    names_state(cx, handle, TaskState::Failed);
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
    // Design 02: with nobody to ask there is no task request to switch to.
    assert!(!exists(cx, handle, "composer-mode"), "no receiver line without an agent");
    let ws = workspace(cx, &shell);
    let mode = cx.update(|cx| ws.read(cx).composer().read(cx).mode());
    assert_eq!(mode, ComposerMode::Message);
}
