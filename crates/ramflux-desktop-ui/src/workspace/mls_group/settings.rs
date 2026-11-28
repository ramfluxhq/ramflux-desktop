// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The settings of an MLS group (board C23): the name and the approval
//! switch, for an owner or admin.
//!
//! - **One field per call.** Save sends the name alone (`group.settings.set`
//!   with the name); the switch sends the approval alone. The notification
//!   mode is a per-device setting of the info panel and is never sent here.
//! - **The name** is trimmed and checked locally (1 to 64 characters,
//!   `chars`); an unchanged name leaves Save off and calls nothing.
//! - **The confirmed value shows.** The field and the switch read the group
//!   as the store has it, never the requested value: a change shows only
//!   after the committed reply and the `settings` event (A5), with "Saving…"
//!   meanwhile. Turning approval off asks first (the confirmation carries the
//!   explanation and the hint); turning it on needs no confirmation.

use crate::style::DesignButton as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, Entity, FocusHandle, IntoElement, Render, Role, SharedString, Subscription, Window,
    div,
};
use ramflux_desktop_core::mls_group::{
    self as mls, AdminKind, AdminTarget, SettingsField, error_key,
};
use ramflux_desktop_core::{Scope, UiEventKind};
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};

use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{self, tr};

/// The settings form and its view-local state.
pub struct MlsSettings {
    store: Entity<StoreHandle>,
    sink: EventSink,
    group: Option<String>,
    name: Entity<InputState>,
    /// A local problem with the name typed (no call was made).
    local: Option<SharedString>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl MlsSettings {
    /// A closed form.
    pub fn new(
        store: Entity<StoreHandle>,
        sink: EventSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = i18n::localized_input("group.settings.name", window, cx, |input| input);
        let subscriptions = vec![
            cx.subscribe(&store, |this, _, event: &StoreEvent, cx| {
                if matches!(event, StoreEvent::Changed(_)) && this.group.is_some() {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&name, window, |this, _, event, window, cx| match event {
                InputEvent::PressEnter { .. } => this.save(window, cx),
                InputEvent::Change => {
                    this.local = None;
                    cx.notify();
                }
                _ => {}
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.close(cx);
                }
            }),
        ];
        Self {
            store,
            sink: sink.scoped(Scope::Panel("mls_settings".to_owned())),
            group: None,
            name,
            local: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Opens the form of `group_id` with its current name.
    pub fn open(&mut self, group_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let current = self
            .store
            .read(cx)
            .store()
            .mls_group(&group_id)
            .map(|g| g.name.clone())
            .unwrap_or_default();
        self.local = None;
        self.name.update(cx, |name, cx| {
            name.set_value(current, window, cx);
            name.focus(window, cx);
        });
        self.group = Some(group_id);
        cx.notify();
    }

    /// Closes the form.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.group.take().is_some() {
            cx.notify();
        }
    }

    /// Whether the form is open.
    pub fn is_open(&self) -> bool {
        self.group.is_some()
    }

    /// Save: the name alone, after the local check. An unchanged name calls
    /// nothing.
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(group_id) = self.group.clone() else { return };
        let typed = self.name.read(cx).value().to_string();
        let trimmed = typed.trim();
        let store = self.store.read(cx).store();
        let current = store.mls_group(&group_id).map(|g| g.name.clone()).unwrap_or_default();
        if trimmed == current || store.mls_admin_busy(&group_id) {
            return;
        }
        if trimmed.is_empty() {
            self.local = Some(tr("group.create.name_required"));
        } else if !mls::check_name(trimmed) {
            self.local = Some(tr("group.create.name_too_long"));
        } else {
            self.sink.emit(
                UiEventKind::MlsSettingsPressed {
                    group_id,
                    field: SettingsField::Name(trimmed.to_owned()),
                },
                window,
                cx,
            );
        }
        cx.notify();
    }

    /// The approval switch: on needs no confirmation, off opens one.
    fn approval(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(group_id) = self.group.clone() else { return };
        if self.store.read(cx).store().mls_admin_busy(&group_id) {
            return;
        }
        if on {
            self.sink.emit(
                UiEventKind::MlsSettingsPressed { group_id, field: SettingsField::Approval(true) },
                window,
                cx,
            );
        } else {
            self.sink.emit(
                UiEventKind::MlsPanelRequested {
                    panel: mls::MlsPanel::ConfirmApprovalOff { group_id },
                },
                window,
                cx,
            );
        }
    }
}

impl Render for MlsSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(group) = self.group.clone() else {
            return div().id("mls-settings").into_any_element();
        };
        let (current, approval, saving_name, saving_approval, busy, failure) = {
            let store = self.store.read(cx).store();
            let summary = store.mls_group(&group);
            let action = store.mls_admin_action(&group);
            let is = |target: AdminTarget| {
                action.is_some_and(|a| a.kind == AdminKind::Settings && a.target == target)
            };
            (
                summary.map(|g| g.name.clone()).unwrap_or_default(),
                summary.is_some_and(|g| g.approval),
                is(AdminTarget::Name),
                is(AdminTarget::Approval),
                store.mls_admin_busy(&group),
                store.mls_admin_failure(&group).filter(|f| f.kind == AdminKind::Settings).cloned(),
            )
        };
        let typed = self.name.read(cx).value().to_string();
        let changed = typed.trim() != current;
        let muted = cx.theme().muted_foreground;
        let danger = crate::tone::text(cx.theme().danger, cx);
        let this = cx.weak_entity();
        let save = Button::new("mls-settings-save")
            .design_primary(cx)
            .design_label(if saving_name {
                tr("group.settings.saving")
            } else {
                tr("group.settings.save")
            })
            .design_disabled(!changed || busy, cx)
            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx)));
        let text = self.local.clone().or_else(|| failure.map(|f| tr(error_key(&f.code))));
        let body = v_flex()
            .id("mls-settings-body")
            .test_support()
            .gap(space(4.))
            .child(
                v_flex()
                    .gap(space(1.))
                    .child(div().text_size(text_secondary()).child(tr("group.settings.name")))
                    .child(
                        Input::new(&self.name)
                            .id("mls-settings-name")
                            .aria_label(tr("group.settings.name")),
                    ),
            )
            .child(
                v_flex()
                    .gap(space(1.))
                    .child(
                        Switch::new("mls-settings-approval")
                            .label(tr("group.settings.approval"))
                            .checked(approval)
                            .disabled(busy)
                            .on_change(cx.listener(|this, checked: &bool, window, cx| {
                                this.approval(*checked, window, cx);
                            })),
                    )
                    .children(saving_approval.then(|| {
                        div()
                            .id("mls-settings-approval-saving")
                            .role(Role::Status)
                            .text_size(text_caption())
                            .text_color(muted)
                            .child(tr("group.settings.saving"))
                    })),
            )
            .children(text.map(|text| {
                div()
                    .id("mls-settings-error")
                    .role(Role::Alert)
                    .text_size(text_secondary())
                    .text_color(danger)
                    .child(text)
            }));
        crate::modal::Modal::new(
            "mls-settings",
            tr("group.settings.title"),
            &self.focus,
            move |_, cx| {
                this.update(cx, |form, cx| form.close(cx)).ok();
            },
        )
        .child(body)
        .button(save)
        .render(cx)
    }
}
