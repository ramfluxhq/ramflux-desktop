// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI integration tests, one or more per moment renderer. Each drives the
//! production window with real input and asserts the resulting intent and
//! effect (or, for view-local behavior, that nothing reached the source).

mod common;

use common::*;
use gpui_kit::TestAppContext;
use ramflux_desktop_core::{
    ConflictResolution, Lifecycle, MomentId, MomentKind, ObjectId, TransferState, methods,
};
use ramflux_desktop_ui::workspace::ComposerMode;

fn lifecycle(
    cx: &mut TestAppContext,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::Shell>,
    id: &str,
) -> Option<Lifecycle> {
    with_store(cx, shell, |s| s.moment(&MomentId::new(id)).map(|m| m.lifecycle))
}

#[gpui_kit::test]
fn message_rows_show_tombstones_attachments_and_send_into_the_context(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    let shown = rows(cx, &shell);
    assert!(shown.contains(&"m-team-00".to_owned()) && shown.contains(&"m-team-0".to_owned()));
    assert!(exists_in(cx, handle, "moment-m-team-0", "message-attachment"), "the attachment row");
    assert!(exists_in(cx, handle, "moment-m-team-0", "message-receipt"), "an outbound receipt");
    type_into(cx, handle, "composer-input", "Looks good");
    in_window(cx, handle, |window, cx| gpui_kit::test::TestWindowExt::press(window, "enter", cx));
    assert_eq!(last_body(cx, &shell, methods::SEND_MESSAGE)["context_id"], "c-team");
}

#[gpui_kit::test]
fn object_transfers_pull_preview_and_save(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    click_in(cx, handle, "moment-m-team-specs", "object-pull");
    assert_eq!(last_body(cx, &shell, methods::OBJECT_PULL)["object_id"], "o-specs");
    advance(cx, 800);
    let state =
        with_store(cx, &shell, |s| s.object(&ObjectId::new("o-specs")).map(|o| o.transfer.clone()));
    assert_eq!(state, Some(TransferState::Complete));
    let before = effects(cx, &shell).len();
    click_in(cx, handle, "moment-m-team-specs", "object-preview");
    assert!(exists_in(cx, handle, "moment-m-team-specs", "object-preview-text"));
    assert_eq!(effects(cx, &shell).len(), before, "preview is view-local");
    click_in(cx, handle, "moment-m-team-specs", "object-save");
    assert!(cx.did_prompt_for_new_path(), "saving asks where with the native dialog");
    assert_eq!(effects(cx, &shell).len(), before, "nothing is saved before a path is chosen");
    cx.simulate_new_path_selection(|dir| Some(dir.join("lighting-specs.pdf")));
    cx.run_until_parked();
    let body = last_body(cx, &shell, methods::OBJECT_SAVE);
    let path = body["path"].as_str().unwrap_or_default().to_owned();
    assert!(path.ends_with("/lighting-specs.pdf") && !path.starts_with('~'), "{path}");
}

#[gpui_kit::test]
fn an_identity_change_waits_for_a_review_and_one_accept(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s04_safety");
    advance(cx, 1_000);
    show_context(cx, handle, &shell, "c-chen");
    assert!(exists(cx, handle, "identity-banner-p-chen"), "the context header carries the banner");
    assert!(exists(cx, handle, "moment-identity:changed:p-chen:A2"), "and a service line");
    advance(cx, 5_000);
    assert!(exists(cx, handle, "identity-banner-p-chen"), "nothing dismisses or accepts it");
    assert!(!exists(cx, handle, "identity-sheet-text"));
    click(cx, handle, "banner-review-p-chen");
    assert!(exists(cx, handle, "identity-sheet-text"), "Review opens the sheet");
    assert!(
        !bodies(cx, &shell).iter().any(|(m, _)| m == methods::IDENTITY_ACCEPT_CHANGE),
        "opening accepts nothing"
    );
    click(cx, handle, "identity-sheet-accept");
    let body = last_body(cx, &shell, methods::IDENTITY_ACCEPT_CHANGE);
    assert_eq!(
        (body["participant_id"].as_str(), body["new_anchor"].as_str()),
        (Some("p-chen"), Some("A2"))
    );
    advance(cx, 500);
    assert!(!exists(cx, handle, "identity-banner-p-chen"), "accepted: the banner goes");
    assert!(!exists(cx, handle, "identity-sheet-text"), "and the sheet closes");
    assert!(exists(cx, handle, "moment-identity:accepted:p-chen:A2"));
}

#[gpui_kit::test]
fn contact_requests_are_answered_inline(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s04_safety");
    advance(cx, 1_000);
    show_context(cx, handle, &shell, "c-requests");
    click_in(cx, handle, "moment-m-noor", "contact-accept");
    assert_eq!(last_body(cx, &shell, methods::CONTACT_ACCEPT)["moment_id"], "m-noor");
    advance(cx, 200);
    assert_eq!(lifecycle(cx, &shell, "m-noor"), Some(Lifecycle::Done));
}

#[gpui_kit::test]
fn task_progress_runs_through_needs_me_to_done(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s06_in_progress");
    advance(cx, 200);
    let workspace = workspace(cx, &shell);
    click(cx, handle, "composer-mode");
    let mode = cx.update(|cx| workspace.read(cx).composer().read(cx).mode());
    assert_eq!(mode, ComposerMode::Task);
    type_into(cx, handle, "composer-input", "Book a train to Lyon on Friday morning");
    click(cx, handle, "composer-send");
    let start = last_body(cx, &shell, methods::TASK_START);
    assert_eq!(start["text"], "Book a train to Lyon on Friday morning");
    assert_eq!(start["assignee"], "p-orbit", "the conversation's AI participant is asked");
    advance(cx, 3_000);
    assert_eq!(lifecycle(cx, &shell, "m-task"), Some(Lifecycle::NeedsMe));
    // One moment, one place: the decision lives only in the task step; the
    // standalone intervention is a reference that brings the step into view.
    assert!(!exists_in(cx, handle, "moment-m-hold", "approve"), "no second decision");
    assert!(exists_in(cx, handle, "moment-m-hold", "intervention-open-task"));
    click_in(cx, handle, "moment-m-hold", "intervention-open-task");
    let (expanded, highlighted) = cx.update(|cx| {
        let field = workspace.read(cx).conversation().read(cx);
        (field.is_expanded(&MomentId::new("m-task")), field.highlighted().cloned())
    });
    assert!(expanded, "the task opens with its steps");
    assert_eq!(highlighted, Some(MomentId::new("m-task")));
    click_in(cx, handle, "moment-m-task", "step-approve");
    assert_eq!(
        last_body(cx, &shell, methods::GRANT_APPROVE)["grant_id"],
        "g-hold",
        "the step's own grant"
    );
    advance(cx, 2_000);
    assert_eq!(lifecycle(cx, &shell, "m-task"), Some(Lifecycle::Done));
    assert!(exists_in(cx, handle, "moment-m-task", "task-result"), "the result shows in the card");
    click_in(cx, handle, "moment-m-task", "task-steps");
    let expanded = cx.update(|cx| {
        workspace.read(cx).conversation().read(cx).is_expanded(&MomentId::new("m-task"))
    });
    assert!(!expanded, "collapsing the steps is view-local");
}

#[gpui_kit::test]
fn conflicts_compare_side_by_side_and_resolve(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s07_conflict_privacy_handoff");
    advance(cx, 1_000);
    assert!(
        exists_in(cx, handle, "moment-m-conflict", "conflict-compare"),
        "the diff compare view"
    );
    click_in(cx, handle, "moment-m-conflict", "conflict-keep-both");
    let body = last_body(cx, &shell, methods::CONFLICT_RESOLVE);
    assert_eq!(body["resolution"], serde_json::json!(ConflictResolution::KeepBoth));
    assert_eq!(body["object_id"], "o-budget");
    advance(cx, 200);
    assert_eq!(lifecycle(cx, &shell, "m-conflict"), Some(Lifecycle::Done));
}

#[gpui_kit::test]
fn privacy_reveal_needs_confirmation_and_writes_an_audit_line(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s07_conflict_privacy_handoff");
    advance(cx, 1_000);
    assert!(exists_in(cx, handle, "moment-m-iban", "privacy-mask"));
    click_in(cx, handle, "moment-m-iban", "privacy-reveal-button");
    assert!(strip_open(cx, &shell), "revealing opens the confirm strip");
    assert!(effects(cx, &shell).is_empty());
    tap_confirm(cx, handle);
    let body = last_body(cx, &shell, methods::PRIVACY_REVEAL);
    assert_eq!(body["moment_id"], "m-iban");
    assert_eq!(body["confirmed_by"]["gesture"], "Tap");
    advance(cx, 300);
    assert!(exists_in(cx, handle, "moment-m-iban", "privacy-content"));
    assert!(exists_in(cx, handle, "moment-m-iban", "privacy-audit"), "the audit note line");
    let audited = with_store(cx, &shell, |s| {
        s.audit().iter().any(|a| a.action == methods::PRIVACY_REVEAL && a.human_confirmed)
    });
    assert!(audited);
}

#[gpui_kit::test]
fn handoffs_continue_on_this_device(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s07_conflict_privacy_handoff");
    advance(cx, 1_000);
    click_in(cx, handle, "moment-m-handoff", "handoff-accept");
    let body = last_body(cx, &shell, methods::HANDOFF_ACCEPT);
    assert_eq!(body["from_device"], "d-desk");
    advance(cx, 200);
    let target = with_store(cx, &shell, |s| {
        match s.moment(&MomentId::new("m-handoff")).map(|m| m.kind.clone()) {
            Some(MomentKind::Handoff(p)) => p.to_device.map(|d| d.to_string()),
            _ => None,
        }
    });
    assert_eq!(target.as_deref(), Some("d-laptop"));
}
