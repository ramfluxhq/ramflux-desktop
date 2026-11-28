// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The confirmation of the actions of an MLS group that cannot be taken
//! back: revoking an invite, deleting an own post for everyone, leaving.
//! One tap on the confirm button emits the action's event, once; Escape, the
//! scrim and Cancel close it with no action. Nothing is confirmed by itself.

use crate::style::DesignButton as _;
use gpui_kit::component::button::Button;
use gpui_kit::prelude::*;
use gpui_kit::{Context, Entity, FocusHandle, IntoElement, Render, Subscription, Window, div};
use ramflux_desktop_core::mls_group::MlsPanel;
use ramflux_desktop_core::{Scope, UiEventKind};
use ramflux_desktop_navigation::scale::text_secondary;

use crate::app::StoreHandle;
use crate::bubble::EventSink;
use crate::i18n::tr;

/// The confirmation dialog and what it asks.
pub struct MlsConfirm {
    _store: Entity<StoreHandle>,
    sink: EventSink,
    panel: Option<MlsPanel>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl MlsConfirm {
    /// A closed dialog.
    pub fn new(
        store: Entity<StoreHandle>,
        sink: EventSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            // The window going to the background closes it with no action.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.close(cx);
                }
            }),
        ];
        Self {
            _store: store,
            sink: sink.scoped(Scope::Panel("mls_confirm".to_owned())),
            panel: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Asks the question `panel` names (a confirm panel); anything else is
    /// not a question.
    pub fn open(&mut self, panel: MlsPanel, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            panel,
            MlsPanel::ConfirmDelete { .. }
                | MlsPanel::ConfirmLeave { .. }
                | MlsPanel::ConfirmRevoke { .. }
        ) {
            self.panel = Some(panel);
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    /// Closes the dialog with no action.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.panel.take().is_some() {
            cx.notify();
        }
    }

    /// Whether a question is open.
    pub fn is_open(&self) -> bool {
        self.panel.is_some()
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = self.panel.take() else { return };
        let kind = match panel {
            MlsPanel::ConfirmDelete { group_id, post_id } => {
                UiEventKind::MlsDeletePressed { group_id, post_id }
            }
            MlsPanel::ConfirmLeave { group_id } => UiEventKind::MlsLeavePressed { group_id },
            MlsPanel::ConfirmRevoke { group_id, invite_id } => {
                UiEventKind::MlsInviteRevokePressed { group_id, invite_id }
            }
            _ => return,
        };
        self.sink.emit(kind, window, cx);
        cx.notify();
    }
}

impl Render for MlsConfirm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(panel) = self.panel.clone() else {
            return div().id("mls-confirm").into_any_element();
        };
        let (title, body, confirm) = match panel {
            MlsPanel::ConfirmDelete { .. } => {
                (tr("group.delete.title"), None, tr("group.delete.confirm"))
            }
            MlsPanel::ConfirmLeave { .. } => {
                (tr("group.leave.title"), Some(tr("group.leave.body")), tr("group.leave.confirm"))
            }
            _ => (
                tr("group.invite.revoke_confirm.title"),
                Some(tr("group.invite.revoke_confirm.body")),
                tr("group.invite.revoke"),
            ),
        };
        let this = cx.weak_entity();
        let cancel = Button::new("mls-confirm-cancel")
            .design_secondary(cx)
            .design_label(tr("common.cancel"))
            .on_click(cx.listener(|this, _, _, cx| this.close(cx)));
        let ok = Button::new("mls-confirm-ok")
            .design_danger(cx)
            .design_label(confirm)
            .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx)));
        crate::modal::Modal::new("mls-confirm", title, &self.focus, move |_, cx| {
            this.update(cx, |dialog, cx| dialog.close(cx)).ok();
        })
        .width(crate::modal::width::NARROW)
        .children_of(body.map(|text| div().text_size(text_secondary()).child(text)))
        .button(cancel)
        .button(ok)
        .render(cx)
    }
}
