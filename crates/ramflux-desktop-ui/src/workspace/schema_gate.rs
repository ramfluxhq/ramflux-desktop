// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The schema gate on screen (S10c D1/D7).
//!
//! While the daemon's answer is awaited, or its status could not be read,
//! the last view stays (read-only, composer disabled) under a status line.
//! When the account cannot be opened at all (an older daemon, a newer
//! schema, an unknown schema, or an account made by an older development
//! build), a blocking screen replaces the conversation area. The reset
//! screen offers only a new account or another account: nothing on it
//! deletes, converts or retries.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, IntoElement, SharedString, WeakEntity, div};
use ramflux_desktop_core::{AccountId, LockState, SchemaGate};

use crate::actions::OpenSettings;
use crate::app::StoreHandle;
use crate::i18n::tr;

/// The account in view and its gate verdict.
pub(super) fn active(handle: &StoreHandle) -> Option<(AccountId, SchemaGate)> {
    let store = handle.store();
    let account = handle.active_account().cloned().or_else(|| {
        store.accounts().find(|a| a.lock_state == LockState::Unlocked).map(|a| a.id.clone())
    })?;
    let gate = store.schema_gate(&account);
    Some((account, gate))
}

/// Why the composer cannot send, while the gate is closed.
pub(super) fn composer_reason(gate: SchemaGate) -> Option<SharedString> {
    (!gate.is_open()).then(|| tr(key(gate, "composer")))
}

fn key(gate: SchemaGate, part: &str) -> &'static str {
    match (gate, part) {
        (SchemaGate::Unresolved, "composer") => "schema.composer.connecting",
        (SchemaGate::Failed, "composer") => "schema.composer.failed",
        (_, "composer") => "schema.composer.blocked",
        (SchemaGate::Unresolved, _) => "schema.connecting",
        (SchemaGate::Failed, _) => "schema.failed",
        (SchemaGate::OlderDaemon, "title") => "schema.older_daemon.title",
        (SchemaGate::OlderDaemon, _) => "schema.older_daemon.body",
        (SchemaGate::UpdateApp, "title") => "schema.update_app.title",
        (SchemaGate::UpdateApp, _) => "schema.update_app.body",
        (SchemaGate::Unsupported, "title") => "schema.unsupported.title",
        (SchemaGate::Unsupported, _) => "schema.unsupported.body",
        (SchemaGate::ResetRequired, "title") => "schema.reset.title",
        (SchemaGate::ResetRequired, _) => "schema.reset.body",
        (SchemaGate::Open, _) => "schema.connecting",
    }
}

/// The status line over the kept, read-only view.
pub(super) fn banner(gate: SchemaGate) -> Option<AnyElement> {
    matches!(gate, SchemaGate::Unresolved | SchemaGate::Failed).then(|| {
        div()
            .id("schema-banner")
            .aria_label(tr(key(gate, "line")))
            .test_support()
            .px_3()
            .pt_3()
            .child(Alert::info("schema-banner-alert", tr(key(gate, "line"))).small())
            .into_any_element()
    })
}

/// The screen that replaces the conversation area.
pub(super) fn blocking(
    gate: SchemaGate,
    other_account: Option<AccountId>,
    store: WeakEntity<StoreHandle>,
    cx: &App,
) -> AnyElement {
    let title = tr(key(gate, "title"));
    let body = tr(key(gate, "body"));
    let reset = gate == SchemaGate::ResetRequired;
    v_flex()
        .id("schema-block")
        .aria_label(title.clone())
        .test_support()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .p_6()
        .child(
            div()
                .id("schema-block-title")
                .aria_label(title.clone())
                .test_support()
                .text_lg()
                .text_color(cx.theme().foreground)
                .child(title),
        )
        .child(
            div()
                .id("schema-block-body")
                .aria_label(body.clone())
                .test_support()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(body),
        )
        .when(reset, |col| {
            col.child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("schema-switch-account")
                            .outline()
                            .label(tr("schema.reset.switch"))
                            .disabled(other_account.is_none())
                            .on_click(move |_, _, cx| {
                                if let Some(target) = &other_account {
                                    store
                                        .update(cx, |store, cx| {
                                            store.set_active_account(target, cx)
                                        })
                                        .ok();
                                }
                            }),
                    )
                    .child(
                        Button::new("schema-create-account")
                            .outline()
                            .label(tr("schema.reset.create"))
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(OpenSettings), cx);
                            }),
                    ),
            )
        })
        .into_any_element()
}
