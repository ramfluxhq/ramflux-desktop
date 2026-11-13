// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Workspace navigation reuses the existing feature and confirmation owners.

use super::{DockTool, Workspace, context_stack::ListFilter};
use crate::actions::{OpenAgentsSettings, OpenSettings};
use crate::i18n::tr;
use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, Selectable as _, Sizable as _, StyledExt as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, IntoElement, TestSupportExt as _, Window, div, rems};
use ramflux_desktop_navigation::theme;

/// Navigation selects projections/features, never a source or an account writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceRoute {
    /// Complete conversation list.
    Messages,
    /// Group conversations.
    Groups,
    /// Conversations containing existing tasks.
    Tasks,
    /// Existing contacts feature.
    Contacts,
    /// Existing incoming-request feature; no implied acceptance.
    Strangers,
}

impl Workspace {
    /// Current navigation destination.
    pub fn route(&self) -> WorkspaceRoute {
        self.route
    }

    /// Routes through existing account-scoped feature owners. Every route
    /// change first cancels a pending hold, so it can never be confirmed
    /// after its reviewed facts left the screen.
    pub fn navigate(&mut self, route: WorkspaceRoute, window: &mut Window, cx: &mut Context<Self>) {
        self.root.update(cx, |root, cx| root.cancel_for_navigation(cx));
        self.route = route;
        let filter = match route {
            WorkspaceRoute::Groups => ListFilter::Groups,
            WorkspaceRoute::Tasks => ListFilter::Tasks,
            _ => ListFilter::Messages,
        };
        self.stack.update(cx, |stack, cx| stack.set_filter(filter, cx));
        self.list_visible = true;
        match route {
            WorkspaceRoute::Groups => {
                self.group_shown = false;
                let current = self.current.clone();
                self.groups.update(cx, |groups, cx| groups.set_selected(current, cx));
                self.groups.update(cx, |groups, cx| groups.focus_search(window, cx));
            }
            WorkspaceRoute::Contacts => {
                self.close_dock(window, cx);
                self.contact_directory.update(cx, |list, cx| list.focus_search(window, cx));
            }
            WorkspaceRoute::Strangers => {
                self.close_dock(window, cx);
                self.strangers.update(cx, |area, cx| area.focus_search(window, cx));
            }
            WorkspaceRoute::Messages | WorkspaceRoute::Tasks => {
                self.close_dock(window, cx);
                self.stack.update(cx, |stack, cx| stack.focus_search(window, cx));
            }
        }
        cx.notify();
    }

    /// Shows a group chosen in the group directory with its member panel
    /// (design C02): the conversation and the members tool side by side.
    pub fn show_group(
        &mut self,
        id: ramflux_desktop_core::ContextId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.group_shown = true;
        self.select_context(Some(id.clone()), window, cx);
        self.groups.update(cx, |groups, cx| groups.set_selected(Some(id), cx));
        self.open_tool(DockTool::Members, window, cx);
        cx.notify();
    }

    /// The list pane of the current route.
    pub(super) fn section_list(&self) -> AnyElement {
        match self.route {
            WorkspaceRoute::Groups => self.groups.clone().into_any_element(),
            WorkspaceRoute::Contacts => self.contact_directory.clone().into_any_element(),
            WorkspaceRoute::Strangers => self.strangers.clone().into_any_element(),
            WorkspaceRoute::Messages | WorkspaceRoute::Tasks => {
                self.stack.clone().into_any_element()
            }
        }
    }

    /// The detail pane of a route that has its own (contacts, strangers, a
    /// group directory with no group chosen yet); `None` shows the
    /// conversation workspace.
    pub(super) fn section_detail(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        match self.route {
            WorkspaceRoute::Contacts => Some(self.contact_profile.clone().into_any_element()),
            WorkspaceRoute::Strangers => {
                Some(self.strangers.update(cx, |area, cx| area.detail(window, cx)))
            }
            WorkspaceRoute::Groups if !self.group_shown => {
                Some(super::sections::directory::placeholder(
                    "groups-detail-empty",
                    tr("sections.groups.choose"),
                    tr("sections.groups.choose_help"),
                    cx,
                ))
            }
            _ => None,
        }
    }

    /// Whether the rail item of `route` is the current one (the strangers
    /// area lives under contacts).
    fn is_route_shown(&self, route: WorkspaceRoute) -> bool {
        self.route == route
            || (route == WorkspaceRoute::Contacts && self.route == WorkspaceRoute::Strangers)
    }

    pub(crate) fn show_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.root.update(cx, |root, cx| root.cancel_for_navigation(cx));
        self.list_visible = true;
        match self.route {
            WorkspaceRoute::Groups => {
                self.groups.update(cx, |groups, cx| groups.focus_search(window, cx));
            }
            WorkspaceRoute::Contacts => {
                self.contact_directory.update(cx, |list, cx| list.focus_search(window, cx));
            }
            WorkspaceRoute::Strangers => {
                self.strangers.update(cx, |area, cx| area.focus_search(window, cx));
            }
            WorkspaceRoute::Messages | WorkspaceRoute::Tasks => {
                self.stack.update(cx, |stack, cx| stack.focus_search(window, cx));
            }
        }
        cx.notify();
    }

    pub(super) fn navigation_rail(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut rail = v_flex()
            .id("navigation-rail")
            .test_support()
            .h_full()
            .w(rems(theme::RAIL_REM))
            .flex_none()
            .gap_2()
            .px_2()
            .py_3()
            .bg(theme::colors(cx).rail)
            .child(
                div()
                    .h_10()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xl()
                    .font_semibold()
                    .child(tr("app.mark")),
            );
        for (id, key, icon, route) in [
            (
                "nav-messages",
                "navigation.messages",
                IconName::MessagesSquare,
                WorkspaceRoute::Messages,
            ),
            ("nav-groups", "navigation.groups", IconName::Users, WorkspaceRoute::Groups),
            ("nav-contacts", "navigation.contacts", IconName::Contact, WorkspaceRoute::Contacts),
            ("nav-tasks", "navigation.tasks", IconName::SquareCheck, WorkspaceRoute::Tasks),
        ] {
            rail = rail.child(
                Button::new(id)
                    .ghost()
                    .small()
                    .p_0()
                    .w_full()
                    .h(rems(52.0 / 15.0))
                    .selected(self.is_route_shown(route))
                    .toggled(self.is_route_shown(route))
                    .accessibility_label(tr(key))
                    .tooltip(tr(key))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.navigate(route, window, cx)),
                    )
                    .child(
                        v_flex()
                            .items_center()
                            .gap_1()
                            .child(Icon::new(icon).small())
                            .child(div().text_xs().child(tr(key))),
                    ),
            );
        }
        rail.child(
            // Base accepts the retained focus handle used after settings closes.
            gpui_kit::base::Button::new("nav-agents")
                .track_focus(&self.agents_focus)
                .flex()
                .items_center()
                .justify_center()
                .w_full()
                .h(rems(52.0 / 15.0))
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().transparent)
                .hover(|style| style.bg(cx.theme().secondary_hover))
                .focus_visible(|style| style.border_color(cx.theme().ring))
                .accessibility_label(tr("navigation.agents"))
                .on_click(cx.listener(|this, _, window, cx| {
                    window.focus(&this.agents_focus, cx);
                    window.dispatch_action(Box::new(OpenAgentsSettings), cx);
                }))
                .child(
                    v_flex()
                        .items_center()
                        .gap_1()
                        .child(Icon::new(IconName::Bot).small())
                        .child(div().text_xs().child(tr("navigation.agents"))),
                ),
        )
        .child(div().flex_1())
        .child(
            Button::new("nav-settings")
                .ghost()
                .small()
                .w_full()
                .icon(IconName::Settings)
                .accessibility_label(tr("settings.title"))
                .tooltip(tr("settings.title"))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings), cx)),
        )
        .into_any_element()
    }
}
