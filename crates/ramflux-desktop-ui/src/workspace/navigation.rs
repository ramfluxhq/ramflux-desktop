// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Workspace navigation reuses the existing feature and confirmation owners.

use super::{DockTool, Workspace, context_stack::ListFilter};
use crate::actions::OpenSettings;
use crate::i18n::tr;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{
    Disableable as _, Icon, Selectable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};
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
    /// The agent roster and the conversation with the chosen agent (04).
    Agents,
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
            WorkspaceRoute::Tasks => {
                self.close_dock(window, cx);
                self.tasks_area.update(cx, |area, cx| area.focus_search(window, cx));
            }
            WorkspaceRoute::Agents => {
                let current = self.current.clone();
                let agent = current.as_ref().and_then(|id| {
                    let store = self.store.read(cx).store();
                    let context = store.context(id)?;
                    (context.kind == ramflux_desktop_core::ContextKind::Direct)
                        .then(|| {
                            ramflux_desktop_core::selectors::participants(store, id)
                                .into_iter()
                                .find(|p| p.kind == ramflux_desktop_core::ParticipantKind::Agent)
                                .map(|p| p.id.clone())
                        })
                        .flatten()
                });
                self.agent_roster.update(cx, |roster, cx| roster.set_selected(agent, cx));
                self.agent_roster.update(cx, |roster, cx| roster.focus_search(window, cx));
            }
            WorkspaceRoute::Messages => {
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
            WorkspaceRoute::Tasks => self.tasks_area.clone().into_any_element(),
            WorkspaceRoute::Agents => self.agent_roster.clone().into_any_element(),
            WorkspaceRoute::Messages => self.stack.clone().into_any_element(),
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
            // Design board 07: the people table until a contact is chosen.
            WorkspaceRoute::Contacts if self.contact_profile.read(cx).participant().is_none() => {
                Some(self.contact_directory.update(cx, |list, cx| list.detail(None, window, cx)))
            }
            WorkspaceRoute::Contacts => Some(self.contact_profile.clone().into_any_element()),
            WorkspaceRoute::Strangers => {
                Some(self.strangers.update(cx, |area, cx| area.detail(window, cx)))
            }
            WorkspaceRoute::Tasks => {
                Some(self.tasks_area.update(cx, |area, cx| area.detail(None, window, cx)))
            }
            WorkspaceRoute::Agents if self.agent_roster.read(cx).selected().is_none() => {
                Some(super::sections::directory::placeholder(
                    "agents-detail-empty",
                    tr("sections.agents.choose"),
                    tr("sections.agents.choose_help"),
                    cx,
                ))
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
            WorkspaceRoute::Tasks => {
                self.tasks_area.update(cx, |area, cx| area.focus_search(window, cx));
            }
            WorkspaceRoute::Agents => {
                self.agent_roster.update(cx, |roster, cx| roster.focus_search(window, cx));
            }
            WorkspaceRoute::Messages => {
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
                    // A section chosen from the rail replaces settings.
                    if settings_open {
                        window.dispatch_action(Box::new(crate::actions::CloseSettings), cx);
                    }
                    this.navigate(route, window, cx)
                }))
                .child(rail_label(icon, key, palette))
        };
        let agents = item("nav-agents", "navigation.agents", IconName::Bot, WorkspaceRoute::Agents);
        let account = self.store.read(cx);
        let active = account.active_account().and_then(|id| account.store().account(id));
        let initial = active
            .map_or_else(|| "?".into(), |a| ramflux_desktop_navigation::row::initial(&a.label));
        // The button names the active account, not only "Current identity".
        let account_name = active.map_or_else(
            || tr("status.account.current"),
            |a| {
                crate::i18n::tr_args(
                    "status.account.current_named",
                    &[("name", &ramflux_desktop_navigation::row::list_text(&a.label))],
                )
            },
        );
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
            // macOS draws its traffic lights here (design: a 52 × 12 reserve
            // at the top of the rail, the monogram below it).
            .when(cfg!(target_os = "macos"), |rail| rail.child(div().h(rems(12.0 / 15.0))))
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
            // Design I01 "Edit navigation": customising the rail is not built
            // yet; the item says so instead of doing nothing.
            .child(
                Button::new("nav-edit")
                    .ghost()
                    .small()
                    .p_0()
                    .w_full()
                    .h(rems(52.0 / 15.0))
                    .rounded(gpui_kit::px(palette.radius_sm))
                    .disabled(true)
                    .accessibility_label(tr("navigation.edit_unavailable"))
                    .tooltip(tr("navigation.edit_unavailable"))
                    .child(rail_label(IconName::Pencil, "navigation.edit", palette)),
            )
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
            .child({
                // Design B01: the avatar opens the identity popover (switch
                // account, lock the current one).
                let store = self.store.downgrade();
                let sink = crate::root::IntentRoot::sink(&self.root);
                gpui_kit::component::popover::Popover::new("identity-popover")
                    .anchor(gpui_kit::Anchor::BottomLeft)
                    .trigger(
                        Button::new("nav-account")
                            .ghost()
                            .p_0()
                            .size(rems(40.0 / 15.0))
                            .rounded(gpui_kit::px(palette.radius_full))
                            .accessibility_label(account_name.clone())
                            .tooltip(account_name)
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
                    .content(move |_, _, cx| identity_popover(&store, &sink, cx).into_any_element())
            })
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

/// The identity popover (design B01): every account of this window with
/// its kind and state (and what needs the user there), "Current" on the
/// shown one and "Switch" on the others, then "Lock the current account".
fn identity_popover(
    store: &gpui_kit::WeakEntity<crate::app::StoreHandle>,
    sink: &crate::bubble::EventSink,
    cx: &mut Context<gpui_kit::component::popover::PopoverState>,
) -> AnyElement {
    use gpui_kit::DismissEvent;
    let palette = ramflux_desktop_navigation::palette(cx);
    let rem = |px: f32| rems(px / 15.0);
    let Some(handle) = store.upgrade() else { return v_flex().into_any_element() };
    let (rows, lockable) = {
        let handle = handle.read(cx);
        let active = handle.active_account().cloned();
        let rows: Vec<_> = handle
            .accounts()
            .map(|id| {
                let account = handle.store().account(id);
                let name = match handle.account_label(id) {
                    Some(name) => name,
                    None if id.as_str() == crate::app::LOCAL_ACCOUNT => {
                        tr("status.account.local").to_string()
                    }
                    None => id.to_string(),
                };
                let detail = account.map_or_else(
                    || tr("status.account.local_detail"),
                    |a| crate::i18n::account_kind(a.kind),
                );
                (id.clone(), name, detail, active.as_ref() == Some(id), handle.needs_me_count(id))
            })
            .collect();
        let lockable = handle
            .active_account()
            .and_then(|id| handle.store().account(id))
            .filter(|a| a.lock_state == ramflux_desktop_core::LockState::Unlocked)
            .map(|a| a.id.clone());
        (rows, lockable)
    };
    v_flex()
        .id("identity-menu")
        .test_support()
        .w(rem(300.0))
        .gap_1()
        .child(
            h_flex()
                .px_1()
                .pb_1()
                .text_size(rem(palette.size_secondary))
                .text_color(palette.secondary)
                .child(div().flex_1().child(tr("status.account.current")))
                .child(
                    div()
                        .id("identity-info")
                        .tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(tr("status.account.info"))
                                .build(window, cx)
                        })
                        .child(Icon::new(IconName::Info).small()),
                ),
        )
        .children(rows.into_iter().map(|(id, name, detail, current, count)| {
            let target = id.clone();
            let store = store.clone();
            let spoken = if count > 0 {
                crate::i18n::join(
                    &ramflux_desktop_navigation::row::list_text(&name),
                    &crate::i18n::tr_args("status.needs_me", &[("count", &count.to_string())]),
                )
            } else {
                ramflux_desktop_navigation::row::list_text(&name)
            };
            h_flex()
                .id(gpui_kit::SharedString::from(format!("identity-{id}")))
                .test_support()
                .aria_label(spoken)
                .gap_3()
                .px_1()
                .py_2()
                .border_b_1()
                .border_color(palette.line)
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(rem(palette.size_body))
                                .font_medium()
                                .child(ramflux_desktop_navigation::row::list_text(&name)),
                        )
                        .child(
                            div()
                                .text_size(rem(palette.size_secondary))
                                .text_color(palette.secondary)
                                .child(if count > 0 {
                                    crate::i18n::join(
                                        &detail,
                                        &crate::i18n::tr_args(
                                            "status.needs_me",
                                            &[("count", &count.to_string())],
                                        ),
                                    )
                                } else {
                                    detail
                                }),
                        ),
                )
                .when(current, |row| {
                    row.child(
                        div()
                            .px_2()
                            .py(rem(4.0))
                            .rounded(rem(palette.radius_chip))
                            .bg(palette.surface)
                            .text_size(rem(palette.size_meta))
                            .child(tr("settings.account.current_chip")),
                    )
                })
                .when(!current, |row| {
                    row.child(
                        crate::style::secondary_button(
                            gpui_kit::SharedString::from(format!("identity-switch-{id}")),
                            cx,
                        )
                        .label(tr("status.account.switch"))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            store
                                .update(cx, |store, cx| store.set_active_account(&target, cx))
                                .ok();
                            cx.emit(DismissEvent);
                        })),
                    )
                })
        }))
        .when_some(lockable, |menu, account_id| {
            let sink = sink.clone();
            menu.child(
                Button::new("identity-lock")
                    .ghost()
                    .w_full()
                    .justify_start()
                    .small()
                    .label(tr("status.account.lock"))
                    .on_click(cx.listener(move |_, _, window, cx| {
                        sink.emit(
                            ramflux_desktop_core::UiEventKind::LockPressed {
                                account_id: account_id.clone(),
                            },
                            window,
                            cx,
                        );
                        cx.emit(DismissEvent);
                    })),
            )
        })
        .into_any_element()
}
