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
    /// change first cancels a pending confirmation, so it can never be confirmed
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
        // Design board 02: monogram, Messages / Groups / Agents, a short
        // divider, Contacts / Tasks, then Settings and the account at the
        // bottom. Items are 52 px squares with a 20 px icon over a caption.
        let palette = ramflux_desktop_navigation::palette(cx);
        let settings_open = self.root.read(cx).host() == crate::root::ConfirmHost::Settings;
        let item = |id: &'static str, key: &'static str, icon: IconName, route: WorkspaceRoute| {
            let shown = !settings_open && self.is_route_shown(route);
            Button::new(id)
                .ghost()
                .small()
                .p_0()
                .w_full()
                .h(rems(52.0 / 15.0))
                .rounded(gpui_kit::px(palette.radius_sm))
                .selected(shown)
                .toggled(shown)
                .accessibility_label(tr(key))
                .tooltip(tr(key))
                .on_click(cx.listener(move |this, _, window, cx| {
                    window.dispatch_action(Box::new(crate::actions::CloseSettings), cx);
                    this.navigate(route, window, cx)
                }))
                .child(rail_label(icon, key, palette))
        };
        let agents = gpui_kit::base::Button::new("nav-agents")
            .track_focus(&self.agents_focus)
            .flex()
            .items_center()
            .justify_center()
            .w_full()
            .h(rems(52.0 / 15.0))
            .rounded(rems(palette.radius_sm / 15.0))
            .border_1()
            .border_color(cx.theme().transparent)
            .hover(|style| style.bg(cx.theme().secondary_hover))
            .focus_visible(|style| style.border_color(cx.theme().ring))
            .accessibility_label(tr("navigation.agents"))
            .on_click(cx.listener(|this, _, window, cx| {
                window.focus(&this.agents_focus, cx);
                window.dispatch_action(Box::new(OpenAgentsSettings), cx);
            }))
            .child(rail_label(IconName::Bot, "navigation.agents", palette));
        let account = self.store.read(cx);
        let initial = account
            .active_account()
            .and_then(|id| account.store().account(id))
            .map_or_else(|| "?".into(), |a| ramflux_desktop_navigation::row::initial(&a.label));
        v_flex()
            .id("navigation-rail")
            .test_support()
            .h_full()
            .w(rems(theme::RAIL_REM))
            .flex_none()
            .items_center()
            .gap_3()
            .px_2()
            .pt(rems(20.0 / 15.0))
            .pb(rems(20.0 / 15.0))
            .bg(theme::colors(cx).rail)
            .child(
                div()
                    .h(rems(34.0 / 15.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(rems(palette.size_page / 15.0))
                    .font_semibold()
                    .child(tr("app.mark")),
            )
            .child(item(
                "nav-messages",
                "navigation.messages",
                IconName::MessagesSquare,
                WorkspaceRoute::Messages,
            ))
            .child(item("nav-groups", "navigation.groups", IconName::Users, WorkspaceRoute::Groups))
            .child(agents)
            .child(
                div()
                    .id("navigation-divider")
                    .py(rems(4.0 / 15.0))
                    .child(div().w(rems(24.0 / 15.0)).h(gpui_kit::px(1.)).bg(palette.line)),
            )
            .child(item(
                "nav-contacts",
                "navigation.contacts",
                IconName::Contact,
                WorkspaceRoute::Contacts,
            ))
            .child(item(
                "nav-tasks",
                "navigation.tasks",
                IconName::SquareCheck,
                WorkspaceRoute::Tasks,
            ))
            .child(div().flex_1())
            .child(
                Button::new("nav-settings")
                    .ghost()
                    .small()
                    .w_full()
                    .h(rems(40.0 / 15.0))
                    .rounded(gpui_kit::px(palette.radius_sm))
                    .selected(settings_open)
                    .toggled(settings_open)
                    .child(Icon::new(IconName::Settings).with_size(gpui_kit::px(20.)))
                    .accessibility_label(tr("settings.title"))
                    .tooltip(tr("settings.title"))
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings), cx)),
            )
            .child(
                Button::new("nav-account")
                    .ghost()
                    .p_0()
                    .size(rems(40.0 / 15.0))
                    .rounded(gpui_kit::px(palette.radius_full))
                    .accessibility_label(tr("settings.section.account"))
                    .tooltip(tr("settings.section.account"))
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings), cx))
                    .child(
                        div()
                            .size(rems(40.0 / 15.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(rems(palette.radius_full / 15.0))
                            .bg(palette.avatar_person)
                            .text_color(palette.on_avatar_person)
                            .font_semibold()
                            .child(initial),
                    ),
            )
            .into_any_element()
    }
}

/// A rail item's content: the 20 px icon over its caption.
fn rail_label(
    icon: IconName,
    key: &'static str,
    palette: ramflux_desktop_navigation::generated::rf_tokens::RFTheme,
) -> impl IntoElement {
    v_flex()
        .items_center()
        .gap_1()
        .child(Icon::new(icon).with_size(gpui_kit::px(20.)))
        .child(div().text_size(rems(palette.size_caption / 15.0)).child(tr(key)))
}
