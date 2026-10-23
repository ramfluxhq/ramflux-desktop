// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Renderer conformance in the production window: a patch that lands while
//! the hold strip is open cancels the hold and asks for a new review; a
//! surface over a limit shows only its summary; an out-of-date surface
//! disables its actions; a refused approval shows its fail-closed state in
//! the moment and the approvals dock and is never sent again.

mod common;

use common::*;
use gpui_kit::test::{TestEventExt as _, TestWindowExt as _};
use gpui_kit::{MouseButton, TestAppContext};
use ramflux_desktop_core::{GrantFailure, GrantId, MomentId, methods};
use ramflux_desktop_test::load_conformance;
use ramflux_desktop_ui::app::InspectorEntry;

/// Opens the window and plays a conformance fixture up to `ms`.
fn play(cx: &mut TestAppContext, name: &str, ms: u64) -> Opened {
    let (handle, shell) = open(cx, "s02_daily");
    let scenario = load_conformance(name).expect("fixture");
    let store = store_handle(cx, &shell);
    cx.update(|cx| store.update(cx, |store, cx| store.load_scenario(&scenario, cx)));
    advance(cx, ms);
    show_context(cx, handle, &shell, "c-conf");
    (handle, shell)
}

fn surface_actions(
    cx: &mut TestAppContext,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::Shell>,
) -> usize {
    effects(cx, shell).iter().filter(|l| *l == methods::SURFACE_ACTION).count()
}

fn intent_results(
    cx: &mut TestAppContext,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::Shell>,
) -> Vec<String> {
    let store = store_handle(cx, shell);
    cx.update(|cx| {
        store
            .read(cx)
            .inspector()
            .entries()
            .filter_map(|e| match e {
                InspectorEntry::Intent { result, .. } => Some(result.clone()),
                _ => None,
            })
            .collect()
    })
}

fn review_again(
    cx: &mut TestAppContext,
    shell: &gpui_kit::Entity<ramflux_desktop_ui::Shell>,
) -> bool {
    let workspace = workspace(cx, shell);
    cx.update(|cx| {
        workspace.read(cx).conversation().read(cx).is_review_again(&MomentId::new("m-form"))
    })
}

#[gpui_kit::test]
fn a_patch_during_the_hold_cancels_it_and_asks_for_a_new_review(cx: &mut TestAppContext) {
    // The agent patches the deploy action at 3000 ms.
    let (handle, shell) = play(cx, "l2_interactive", 2_500);
    click_in(cx, handle, "moment-m-form", "action-deploy");
    assert!(strip_open(cx, &shell), "a high-risk action opens the hold strip");

    // Start holding; the patch lands 500 ms into the 1200 ms hold.
    let center = in_window(cx, handle, |window, cx| {
        let center = window.find("hold-confirm").bounds().center();
        window.pointer_down(center, MouseButton::Left, cx);
        center
    });
    advance(cx, 700);
    assert!(!strip_open(cx, &shell), "the hold is cancelled by the patch");
    advance(cx, 1_000);
    in_window(cx, handle, |window, cx| window.pointer_up(center, MouseButton::Left, cx));
    assert!(!strip_open(cx, &shell), "the hold is cancelled by the patch");
    assert_eq!(surface_actions(cx, &shell), 0, "nothing was performed");
    assert!(
        intent_results(cx, &shell).iter().any(|r| r == "confirmation cancelled: surface changed")
    );
    assert!(review_again(cx, &shell));
    assert!(exists_in(cx, handle, "moment-m-form", "surface-review-again"), "the notice is shown");

    // Acting again is the new review: the notice clears and the hold binds
    // the new version.
    click_in(cx, handle, "moment-m-form", "action-deploy");
    assert!(!review_again(cx, &shell));
    assert!(strip_open(cx, &shell));
    hold_confirm(cx, handle);
    let body = last_body(cx, &shell, methods::SURFACE_ACTION);
    let current = with_store(cx, &shell, |s| s.surface_hash(&MomentId::new("m-form")).cloned());
    assert_eq!(body["surface_hash"], serde_json::json!(current.expect("hash")));
    assert_eq!(body["action_id"], "deploy");
}

#[gpui_kit::test]
fn a_reloaded_world_drops_the_review_notice(cx: &mut TestAppContext) {
    let (handle, shell) = play(cx, "l2_interactive", 2_500);
    click_in(cx, handle, "moment-m-form", "action-deploy");
    advance(cx, 700);
    assert!(review_again(cx, &shell));
    // A reloaded world never inherits the notice (view state resets).
    let scenario = load_conformance("l2_interactive").expect("fixture");
    let store = store_handle(cx, &shell);
    cx.update(|cx| store.update(cx, |store, cx| store.load_scenario(&scenario, cx)));
    advance(cx, 2_500);
    assert!(!review_again(cx, &shell), "view state of the old world is dropped");
}

#[gpui_kit::test]
fn a_click_after_a_patch_binds_the_new_version(cx: &mut TestAppContext) {
    let (handle, shell) = play(cx, "l2_interactive", 500);
    // The patch lands between two frames; the next frame re-renders the row,
    // so the click binds the patched version and the hold strip opens.
    let store = store_handle(cx, &shell);
    cx.update(|cx| store.update(cx, |store, cx| store.advance(2_600, cx)));
    click_in(cx, handle, "moment-m-form", "action-deploy");
    assert!(strip_open(cx, &shell), "the re-rendered row binds the new version");
    assert_eq!(surface_actions(cx, &shell), 0);
}

#[gpui_kit::test]
fn over_limit_and_out_of_date_surfaces_offer_no_active_actions(cx: &mut TestAppContext) {
    let (handle, shell) = play(cx, "l1_documents", 650);
    assert!(exists_in(cx, handle, "moment-m-l1", "surface-conflict"), "out of date is shown");
    advance(cx, 500);
    assert!(!exists_in(cx, handle, "moment-m-l1", "surface-conflict"), "a snapshot resolves it");
    assert!(exists_in(cx, handle, "moment-m-over", "surface-over-limit"));
    assert!(exists_in(cx, handle, "moment-m-over", "block-fallback"), "the summary is shown");
    assert!(!exists_in(cx, handle, "moment-m-over", "surface-actions"), "no partial rendering");
    assert!(!exists_in(cx, handle, "moment-m-over", "action-opt-01"));
    let _ = shell;
}

#[gpui_kit::test]
fn redacted_and_unsupported_blocks_render_as_fallbacks(cx: &mut TestAppContext) {
    let (handle, _shell) = play(cx, "l1_documents", 200);
    click_in(cx, handle, "moment-m-l1", "surface-disclosure");
    assert!(exists_in(cx, handle, "block-b-key", "block-redacted"));
    assert!(!has_label(cx, handle, "signing key passphrase: amber-otter-42"));
    assert!(exists_in(cx, handle, "block-b-broken", "block-fallback"));
    assert!(exists_in(cx, handle, "block-b-v2", "block-fallback"));
}

#[gpui_kit::test]
fn a_refused_approval_fails_closed_inline_and_in_the_dock(cx: &mut TestAppContext) {
    let (handle, shell) = play(cx, "l2_interactive", 500);
    click_in(cx, handle, "moment-m-tool", "approve");
    hold_confirm(cx, handle);
    advance(cx, 300);
    let failure =
        with_store(cx, &shell, |s| s.grant(&GrantId::new("g-tool")).and_then(|g| g.failure));
    assert_eq!(failure, Some(GrantFailure::SignedApprovalRequired));
    assert!(exists_in(cx, handle, "moment-m-tool", "grant-failure"), "shown in the moment");
    assert!(!exists_in(cx, handle, "moment-m-tool", "approve"), "no decision is offered again");
    click(cx, handle, "rail-approvals");
    assert!(exists_in(cx, handle, "approvals-pending-g-tool", "grant-failure"), "and in the dock");
    assert!(!exists_in(cx, handle, "approvals-pending-g-tool", "approve"));
    advance(cx, 2_000);
    let approvals = effects(cx, &shell).iter().filter(|l| *l == methods::GRANT_APPROVE).count();
    assert_eq!(approvals, 1, "an approval is never retried");
}
