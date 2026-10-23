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
fn agent_surface_streams_in_place_then_offers_its_action(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 750);
    show_context(cx, handle, &shell, "c-orbit");
    assert!(exists_in(cx, handle, "moment-m-orbit-a", "surface-streaming"), "streaming is shown");
    let text = |cx: &mut TestAppContext| {
        with_store(cx, &shell, |s| {
            match s.moment(&MomentId::new("m-orbit-a")).map(|m| m.kind.clone()) {
                Some(MomentKind::AgentSurface(p)) => {
                    p.blocks[0].data["text"].as_str().unwrap_or_default().to_owned()
                }
                _ => String::new(),
            }
        })
    };
    let partial = text(cx);
    assert_eq!(partial, "Three decisions were made: ");
    advance(cx, 200);
    assert!(text(cx).len() > partial.len(), "appended text renders incrementally");
    assert!(exists_in(cx, handle, "moment-m-orbit-a", "block-b-answer"));
    advance(cx, 500);
    assert!(!exists_in(cx, handle, "moment-m-orbit-a", "surface-streaming"), "streaming ended");
    click_in(cx, handle, "moment-m-orbit-a", "action-copy");
    let body = last_body(cx, &shell, methods::SURFACE_ACTION);
    assert_eq!(body["moment_id"], "m-orbit-a");
    assert_eq!(body["action_id"], "copy");
}

#[gpui_kit::test]
fn the_block_catalog_collapses_in_place_and_disabled_actions_send_nothing(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s02_daily");
    advance(cx, 3_000);
    show_context(cx, handle, &shell, "c-orbit");
    let scope = "moment-m-orbit-digest";
    assert!(exists_in(cx, handle, scope, "block-b-hours"), "metric");
    assert!(!exists_in(cx, handle, scope, "block-b-file"), "collapsed after four blocks");
    click_in(cx, handle, scope, "surface-disclosure");
    assert!(effects(cx, &shell).is_empty(), "expanding is view-local");
    for block in [
        "b-done",
        "b-budget",
        "b-progress",
        "b-log",
        "b-warn",
        "b-err",
        "b-ok",
        "b-file",
        "b-actions",
    ] {
        assert!(exists_in(cx, handle, scope, &format!("block-{block}")), "{block} renders");
    }
    click_in(cx, handle, scope, "action-archive");
    assert!(effects(cx, &shell).is_empty(), "a disabled action sends nothing");
    click_in(cx, handle, scope, "action-open-report");
    assert_eq!(last_body(cx, &shell, methods::SURFACE_ACTION)["action_id"], "open-report");
    advance(cx, 300);
    assert!(exists_in(cx, handle, scope, "block-b-opened"), "the response patches the surface");
}

#[gpui_kit::test]
fn unknown_blocks_render_the_fallback_and_failed_transfers_retry(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s08_failures");
    advance(cx, 4_000);
    show_context(cx, handle, &shell, "c-orbit");
    assert!(
        exists_in(cx, handle, "moment-m-chart", "block-fallback"),
        "unknown kind -> fallback block"
    );
    assert!(rows(cx, &shell).contains(&"m-fallback".to_owned()), "fallback moment row");
    show_context(cx, handle, &shell, "c-lena");
    assert!(rows(cx, &shell).contains(&"m-lost".to_owned()), "system rows");
    click_in(cx, handle, "moment-m-upload", "object-retry");
    assert_eq!(last_body(cx, &shell, methods::OBJECT_RETRY)["object_id"], "o-photos");
    advance(cx, 1_500);
    assert_eq!(lifecycle(cx, &shell, "m-upload-retry"), Some(Lifecycle::Done));
}

#[gpui_kit::test]
fn surface_forms_add_their_values_to_the_action(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s03_intervention");
    advance(cx, 1_000);
    let scope = "moment-m-window";
    click_in(cx, handle, scope, "option-sun-0400");
    assert!(effects(cx, &shell).is_empty(), "choosing an option is view-local");
    type_into(cx, handle, "field-note", "Restart after backups");
    click_in(cx, handle, scope, "action-schedule");
    let body = last_body(cx, &shell, methods::SURFACE_ACTION);
    assert_eq!(body["values"]["window"], "sun-0400", "the moment level added the selection");
    assert_eq!(body["values"]["note"], "Restart after backups");
    advance(cx, 300);
    assert!(exists_in(cx, handle, scope, "block-b-scheduled"));
}

#[gpui_kit::test]
fn approval_blocks_decide_their_grant(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s03_intervention");
    advance(cx, 1_000);
    in_window(cx, handle, |window, cx| {
        gpui_kit::test::TestWindowExt::within(window, "block-b-notice").click("approve", cx)
    });
    let body = last_body(cx, &shell, methods::GRANT_APPROVE);
    assert_eq!(body["grant_id"], "g-notice", "the block's own grant, medium risk: no hold");
    assert!(!strip_open(cx, &shell));
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
    let body = last_body(cx, &shell, methods::OBJECT_SAVE);
    assert_eq!(body["path"], "~/Downloads/lighting-specs.pdf");
}

#[gpui_kit::test]
fn safety_changes_stay_sticky_until_verified(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s04_safety");
    advance(cx, 1_000);
    show_context(cx, handle, &shell, "c-mira");
    assert!(exists(cx, handle, "safety-banner-p-mira"), "the context header carries the banner");
    advance(cx, 5_000);
    assert!(exists(cx, handle, "safety-banner-p-mira"), "still there");
    click_in(cx, handle, "moment-m-key", "verify-safety");
    assert_eq!(last_body(cx, &shell, methods::SAFETY_VERIFY)["participant_id"], "p-mira");
    advance(cx, 200);
    assert!(!exists(cx, handle, "safety-banner-p-mira"), "verified: the banner goes");
    assert_eq!(lifecycle(cx, &shell, "m-key"), Some(Lifecycle::Done));
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
    assert_eq!(
        last_body(cx, &shell, methods::TASK_START)["request"],
        "Book a train to Lyon on Friday morning"
    );
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
fn privacy_reveal_needs_a_hold_and_writes_an_audit_line(cx: &mut TestAppContext) {
    let (handle, shell) = open(cx, "s07_conflict_privacy_handoff");
    advance(cx, 1_000);
    assert!(exists_in(cx, handle, "moment-m-iban", "privacy-mask"));
    click_in(cx, handle, "moment-m-iban", "privacy-reveal-button");
    assert!(strip_open(cx, &shell), "revealing opens the hold strip");
    assert!(effects(cx, &shell).is_empty());
    hold_confirm(cx, handle);
    let body = last_body(cx, &shell, methods::PRIVACY_REVEAL);
    assert_eq!(body["moment_id"], "m-iban");
    assert_eq!(body["confirmed_by"]["gesture"], "Hold");
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
