// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Summoned tools: the dock rail and the right-dock panels.
//!
//! A tool opens when summoned from the rail or the command palette. A
//! summoned tool closes again when the user moves to another context; a
//! pinned tool stays open and reopens on the next launch, and the whole dock
//! layout persists under a schema version ([`prefs`], [`layout`]).
//! Every panel shares one frame ([`DockToolPanel`]: title, pin toggle) around
//! its content view:
//!
//! - [`objects`]: transfers in a table, with their actions.
//! - [`contacts`]: contacts, request and safety state, verification.
//! - [`members`]: members and roles of the current group, governance.
//! - [`approvals`]: pending grants, grants, standing approvals, audit.
//! - [`evidence`]: read-only protocol evidence for the current context.

pub mod approvals;
pub mod contacts;
pub mod evidence;
pub mod layout;
pub mod members;
pub mod objects;
pub mod prefs;

use std::rc::Rc;

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::{
    ActiveTheme as _, IconName, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyView, App, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    SharedString, Window, div,
};
use serde::{Deserialize, Serialize};

use crate::i18n::tr;

/// One summonable tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DockTool {
    /// Objects and transfers.
    Objects,
    /// Contacts.
    Contacts,
    /// Members of the current context.
    Members,
    /// Approvals and grants.
    Approvals,
    /// Security and audit evidence.
    Evidence,
}

impl DockTool {
    /// Every tool in rail order.
    pub const ALL: [Self; 5] =
        [Self::Objects, Self::Contacts, Self::Members, Self::Approvals, Self::Evidence];

    /// Stable id fragment, used for element ids, scopes and persisted layouts.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Objects => "objects",
            Self::Contacts => "contacts",
            Self::Members => "members",
            Self::Approvals => "approvals",
            Self::Evidence => "evidence",
        }
    }

    /// Dock engine panel name, used in persisted layouts.
    pub const fn panel_name(self) -> &'static str {
        match self {
            Self::Objects => "ObjectsPanel",
            Self::Contacts => "ContactsPanel",
            Self::Members => "MembersPanel",
            Self::Approvals => "ApprovalsPanel",
            Self::Evidence => "EvidencePanel",
        }
    }

    /// Scope name of the tool's panel level in the bubbling path.
    pub fn scope_name(self) -> String {
        format!("dock.{}", self.key())
    }

    /// Display name.
    pub fn title(self) -> SharedString {
        tr(match self {
            Self::Objects => "dock.title.objects",
            Self::Contacts => "dock.title.contacts",
            Self::Members => "dock.title.members",
            Self::Approvals => "dock.title.approvals",
            Self::Evidence => "dock.title.evidence",
        })
    }

    /// Rail icon.
    pub fn icon(self) -> IconName {
        match self {
            Self::Objects => IconName::Folder,
            Self::Contacts => IconName::User,
            Self::Members => IconName::Building2,
            Self::Approvals => IconName::CircleCheck,
            Self::Evidence => IconName::Eye,
        }
    }
}

/// Reported by a dock panel frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockToolEvent {
    /// The user pinned or unpinned the tool.
    PinToggled(DockTool),
}

/// The shared dock panel frame around one tool's content view.
pub struct DockToolPanel {
    tool: DockTool,
    content: AnyView,
    pinned: bool,
    focus_handle: FocusHandle,
}

impl DockToolPanel {
    /// A panel for `tool` showing `content`.
    pub fn new(tool: DockTool, content: AnyView, cx: &mut Context<Self>) -> Self {
        Self { tool, content, pinned: false, focus_handle: cx.focus_handle() }
    }

    /// The tool shown.
    pub fn tool(&self) -> DockTool {
        self.tool
    }

    /// Marks the tool pinned (stays open, restored on launch).
    pub fn set_pinned(&mut self, pinned: bool, cx: &mut Context<Self>) {
        if self.pinned != pinned {
            self.pinned = pinned;
            cx.notify();
        }
    }
}

impl EventEmitter<PanelEvent> for DockToolPanel {}
impl EventEmitter<DockToolEvent> for DockToolPanel {}

impl Focusable for DockToolPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for DockToolPanel {
    fn panel_name(&self) -> &'static str {
        self.tool.panel_name()
    }

    fn closable(&self, _: &App) -> bool {
        false
    }
}

impl Panel for DockToolPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.tool.title()
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for DockToolPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tool = self.tool;
        let pinned = self.pinned;
        v_flex()
            .id(SharedString::from(format!("dock-panel-{}", tool.key())))
            .size_full()
            .min_h_0()
            .child(
                h_flex()
                    .px_3()
                    .py_1()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().flex_1().text_sm().font_semibold().child(tool.title()))
                    .child(
                        Button::new(SharedString::from(format!("dock-pin-{}", tool.key())))
                            .ghost()
                            .xsmall()
                            .icon(if pinned { IconName::StarFill } else { IconName::Star })
                            .label(tr(if pinned { "dock.unpin" } else { "dock.pin" }))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(DockToolEvent::PinToggled(tool));
                            })),
                    ),
            )
            .child(
                div()
                    .id(SharedString::from(format!("dock-content-{}", tool.key())))
                    .flex_1()
                    .min_h_0()
                    .child(self.content.clone())
                    .test_support(),
            )
    }
}

/// Opens or closes a tool.
type ToggleTool = dyn Fn(DockTool, &mut Window, &mut App);

/// The vertical rail of tool buttons at the trailing edge.
#[derive(IntoElement)]
pub struct DockRail {
    open: Option<DockTool>,
    on_toggle: Rc<ToggleTool>,
}

impl DockRail {
    /// A rail showing `open` as selected; `on_toggle` opens or closes a tool.
    pub fn new(
        open: Option<DockTool>,
        on_toggle: impl Fn(DockTool, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { open, on_toggle: Rc::new(on_toggle) }
    }
}

impl RenderOnce for DockRail {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let buttons = DockTool::ALL.into_iter().map(|tool| {
            let on_toggle = self.on_toggle.clone();
            Button::new(SharedString::from(format!("rail-{}", tool.key())))
                .ghost()
                .small()
                .icon(tool.icon())
                .tooltip(tool.title())
                .selected(self.open == Some(tool))
                .on_click(move |_: &ClickEvent, window, cx| on_toggle(tool, window, cx))
        });
        v_flex()
            .id("dock-rail")
            .h_full()
            .flex_shrink_0()
            .gap_1()
            .p_1()
            .border_l_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().sidebar)
            .children(buttons)
    }
}

/// A small muted line for empty dock sections.
pub(crate) fn empty_line(key: &str, cx: &App) -> gpui_kit::AnyElement {
    div()
        .px_3()
        .py_2()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(tr(key))
        .into_any_element()
}

/// A section title inside a dock.
pub(crate) fn section(key: &str, cx: &App) -> gpui_kit::AnyElement {
    div()
        .px_3()
        .pt_3()
        .pb_1()
        .text_xs()
        .font_semibold()
        .text_color(cx.theme().muted_foreground)
        .child(tr(key))
        .into_any_element()
}
