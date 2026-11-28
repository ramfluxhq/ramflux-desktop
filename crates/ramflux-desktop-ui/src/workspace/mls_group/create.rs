// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The new MLS group dialog (boards C01, C30): a name (trimmed, 1 to 64
//! characters, checked here before the call), the approval switch (on by
//! default, with its help text) and the size line.
//!
//! One tap on Create is one call. While it runs both buttons are disabled; a
//! typed refusal shows inline and the dialog stays open for the user to
//! change and press again (nothing is repeated by itself). A group opens
//! only with the `group_id` the SDK answered (S6): the dialog reports
//! [`MlsCreateEvent::Created`] then, and the workspace opens the group, the
//! invite panel and the one-time host notice.

use crate::style::DesignButton as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, Entity, EventEmitter, FocusHandle, IntoElement, Render, Role, SharedString,
    Subscription, Window, div,
};
use ramflux_desktop_core::mls_group::{self as mls, error_key};
use ramflux_desktop_core::{Scope, UiEventKind};
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};

use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{self, tr};

/// Reported to the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MlsCreateEvent {
    /// The group exists: show it.
    Created(String),
}

/// Where the dialog is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    Editing,
    /// Create was pressed; the store's outcome count then.
    Creating(u64),
    /// Refused: the error code, or a local problem.
    Failed(SharedString),
}

/// The dialog and its view-local state.
pub struct MlsCreate {
    store: Entity<StoreHandle>,
    sink: EventSink,
    open: bool,
    name: Entity<InputState>,
    approval: bool,
    phase: Phase,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<MlsCreateEvent> for MlsCreate {}

impl MlsCreate {
    /// A closed dialog.
    pub fn new(
        store: Entity<StoreHandle>,
        sink: EventSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = i18n::localized_input("group.create.name", window, cx, |input| input);
        let subscriptions = vec![
            cx.subscribe(&store, |this, _, event: &StoreEvent, cx| {
                if matches!(event, StoreEvent::Changed(_)) {
                    this.observe(cx);
                }
            }),
            cx.subscribe_in(&name, window, |this, _, event, window, cx| match event {
                InputEvent::PressEnter { .. } => this.create(window, cx),
                InputEvent::Change => {
                    if matches!(this.phase, Phase::Failed(_)) {
                        this.phase = Phase::Editing;
                    }
                    cx.notify();
                }
                _ => {}
            }),
        ];
        Self {
            store,
            sink: sink.scoped(Scope::Panel("mls_create".to_owned())),
            open: false,
            name,
            approval: true,
            phase: Phase::Editing,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Opens an empty dialog with approval on.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = true;
        self.approval = true;
        self.phase = Phase::Editing;
        self.name.update(cx, |name, cx| {
            name.set_value("", window, cx);
            name.focus(window, cx);
        });
        cx.notify();
    }

    /// Closes the dialog.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.open = false;
            cx.notify();
        }
    }

    /// Whether the dialog is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open || matches!(self.phase, Phase::Creating(_)) {
            return;
        }
        let name = self.name.read(cx).value().to_string();
        let trimmed = name.trim();
        if trimmed.is_empty() {
            self.phase = Phase::Failed(tr("group.create.name_required"));
        } else if !mls::check_name(trimmed) {
            self.phase = Phase::Failed(tr("group.create.name_too_long"));
        } else {
            let seq = self.store.read(cx).store().mls_create().seq;
            self.phase = Phase::Creating(seq);
            self.sink.emit(
                UiEventKind::MlsCreatePressed { name: trimmed.to_owned(), approval: self.approval },
                window,
                cx,
            );
        }
        cx.notify();
    }

    /// Follows the store: the outcome of the create this dialog asked for.
    fn observe(&mut self, cx: &mut Context<Self>) {
        let Phase::Creating(since) = self.phase else { return };
        let create = self.store.read(cx).store().mls_create().clone();
        if create.seq <= since {
            return;
        }
        match create.outcome {
            Some(Ok(group_id)) => {
                self.open = false;
                self.phase = Phase::Editing;
                cx.emit(MlsCreateEvent::Created(group_id));
            }
            Some(Err(code)) => self.phase = Phase::Failed(tr(error_key(&code))),
            None => {}
        }
        cx.notify();
    }
}

impl Render for MlsCreate {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().id("mls-create").into_any_element();
        }
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let danger = crate::tone::text(theme.danger, cx);
        let creating = matches!(self.phase, Phase::Creating(_));
        let status = match &self.phase {
            Phase::Creating(_) => Some((tr("group.create.creating"), muted)),
            Phase::Failed(reason) => Some((reason.clone(), danger)),
            Phase::Editing => None,
        };
        let help = if self.approval {
            tr("group.create.approval_on_help")
        } else {
            tr("group.create.approval_off_help")
        };
        let this = cx.weak_entity();
        let create = Button::new("mls-create-submit")
            .design_primary(cx)
            .design_label(tr("group.create.submit"))
            .loading(creating)
            .design_disabled(creating, cx)
            .on_click(cx.listener(|this, _, window, cx| this.create(window, cx)));
        let cancel = Button::new("mls-create-cancel")
            .design_secondary(cx)
            .design_label(tr("common.cancel"))
            .design_disabled(creating, cx)
            .on_click(cx.listener(|this, _, _, cx| this.close(cx)));
        let form = v_flex()
            .id("mls-create-form")
            .test_support()
            .gap(space(4.))
            .child(
                v_flex()
                    .gap(space(1.))
                    .child(div().text_size(text_secondary()).child(tr("group.create.name")))
                    .child(
                        Input::new(&self.name)
                            .id("mls-create-name")
                            .aria_label(tr("group.create.name")),
                    ),
            )
            .child(
                v_flex()
                    .gap(space(1.))
                    .child(
                        Switch::new("mls-create-approval")
                            .label(tr("group.create.approval"))
                            .checked(self.approval)
                            .disabled(creating)
                            .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                this.approval = *checked;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("mls-create-approval-help")
                            .test_support()
                            .text_size(text_caption())
                            .text_color(muted)
                            .child(help),
                    ),
            )
            .child(div().text_size(text_caption()).text_color(muted).child(tr("group.create.size")))
            .children(status.map(|(text, color)| {
                div()
                    .id("mls-create-status")
                    .role(Role::Status)
                    .aria_label(text.clone())
                    .test_support()
                    .text_size(text_secondary())
                    .text_color(color)
                    .child(text)
            }));
        crate::modal::Modal::new(
            "mls-create",
            tr("group.create.title"),
            &self.focus,
            move |_, cx| {
                this.update(cx, |dialog, cx| {
                    if !matches!(dialog.phase, Phase::Creating(_)) {
                        dialog.close(cx);
                    }
                })
                .ok();
            },
        )
        .child(form)
        .button(cancel)
        .button(create)
        .render(cx)
    }
}
