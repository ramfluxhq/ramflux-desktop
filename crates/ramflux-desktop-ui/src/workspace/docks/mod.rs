// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Summoned tools: the dock rail and the right-dock panels.
//!
//! Panels open only when summoned from the rail; they are never permanent
//! inspectors. In this build each panel is an empty shell; their content
//! arrives with the dock cards.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ClickEvent, Context, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    SharedString, Window, div,
};

use crate::i18n::tr;

/// One summonable tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

    /// Stable id fragment, used for element ids and persisted layouts.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Objects => "objects",
            Self::Contacts => "contacts",
            Self::Members => "members",
            Self::Approvals => "approvals",
            Self::Evidence => "evidence",
        }
    }

    const fn panel_name(self) -> &'static str {
        match self {
            Self::Objects => "ObjectsPanel",
            Self::Contacts => "ContactsPanel",
            Self::Members => "MembersPanel",
            Self::Approvals => "ApprovalsPanel",
            Self::Evidence => "EvidencePanel",
        }
    }

    /// Display name.
    pub fn title(self) -> SharedString {
        tr(match self {
            Self::Objects => "dock.objects",
            Self::Contacts => "dock.contacts",
            Self::Members => "dock.members",
            Self::Approvals => "dock.approvals",
            Self::Evidence => "dock.evidence",
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

/// An empty dock panel for one tool.
pub struct DockToolPanel {
    tool: DockTool,
    focus_handle: FocusHandle,
}

impl DockToolPanel {
    /// A panel for `tool`.
    pub fn new(tool: DockTool, cx: &mut Context<Self>) -> Self {
        Self { tool, focus_handle: cx.focus_handle() }
    }

    /// The tool shown.
    pub fn tool(&self) -> DockTool {
        self.tool
    }
}

impl EventEmitter<PanelEvent> for DockToolPanel {}

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
}

impl Render for DockToolPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id(SharedString::from(format!("dock-panel-{}", self.tool.key())))
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .p_4()
            .text_color(cx.theme().muted_foreground)
            .child(Icon::new(self.tool.icon()))
            .child(div().text_sm().child(tr("dock.empty")))
    }
}

/// Opens or closes a tool.
type ToggleTool = dyn Fn(DockTool, &mut Window, &mut App);

/// The vertical rail of tool buttons at the trailing edge.
#[derive(IntoElement)]
pub struct DockRail {
    open: Option<DockTool>,
    on_toggle: std::rc::Rc<ToggleTool>,
}

impl DockRail {
    /// A rail showing `open` as selected; `on_toggle` opens or closes a tool.
    pub fn new(
        open: Option<DockTool>,
        on_toggle: impl Fn(DockTool, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self { open, on_toggle: std::rc::Rc::new(on_toggle) }
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
