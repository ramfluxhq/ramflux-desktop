// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The info panel of an MLS group (boards C02, C23, C24) and the one-time
//! notice about the group's server.
//!
//! The panel shows the name, the member count (and the agent count when it
//! is above zero), the row "the group's server can see your network
//! address" with the server's name, the approval mode and this device's
//! notification mode (both read-only), and the entries: Invite links (any
//! member), Members, Refresh, and Leave. Leave asks for a confirmation;
//! "owner_protected" answers that the owner cannot leave, and on "committed"
//! the group becomes read-only. The group is read again when the panel
//! opens.

use crate::style::DesignButton as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, Entity, FocusHandle, IntoElement, Render, Role, Subscription, Window, div,
};
use ramflux_desktop_core::mls_group::{self as mls, LeaveResult, PushSetting};
use ramflux_desktop_core::{Scope, UiEventKind};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};

use super::group_name;
use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args, tr_count};

/// The info panel.
pub struct MlsInfo {
    store: Entity<StoreHandle>,
    sink: EventSink,
    group: Option<String>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl MlsInfo {
    /// A closed panel.
    pub fn new(
        store: Entity<StoreHandle>,
        sink: EventSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.subscribe(&store, |this, _, event: &StoreEvent, cx| {
                if matches!(event, StoreEvent::Changed(_)) && this.group.is_some() {
                    cx.notify();
                }
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.close(cx);
                }
            }),
        ];
        Self {
            store,
            sink: sink.scoped(Scope::Panel("mls_info".to_owned())),
            group: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Opens the panel of `group_id` and reads the group.
    pub fn open(&mut self, group_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.sink.emit(UiEventKind::MlsInfoRequested { group_id: group_id.clone() }, window, cx);
        self.group = Some(group_id);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Closes the panel.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.group.take().is_some() {
            cx.notify();
        }
    }

    /// Whether the panel is open.
    pub fn is_open(&self) -> bool {
        self.group.is_some()
    }

    fn panel(&mut self, panel: mls::MlsPanel, window: &mut Window, cx: &mut Context<Self>) {
        self.sink.emit(UiEventKind::MlsPanelRequested { panel }, window, cx);
    }
}

impl Render for MlsInfo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(group) = self.group.clone() else {
            return div().id("mls-info").into_any_element();
        };
        let store = self.store.read(cx).store();
        let summary = store.mls_group(&group).cloned();
        let read_only = store.mls_read_only(&group);
        let leave_failure = store.mls_leave_failure(&group).cloned();
        let leaving = store.mls_leaving(&group);
        let muted = cx.theme().muted_foreground;
        let danger = crate::tone::text(cx.theme().danger, cx);
        let this = cx.weak_entity();
        let title = tr("group.info.title");
        let Some(summary) = summary else {
            // A group that left the list has no info to show.
            return crate::modal::Modal::new("mls-info", title, &self.focus, move |_, cx| {
                this.update(cx, |panel, cx| panel.close(cx)).ok();
            })
            .child(div().text_size(text_secondary()).text_color(muted).child(tr(match read_only {
                Some(mls::ReadOnly::Removed) => "group.state.removed",
                Some(mls::ReadOnly::Banned) => "group.state.banned",
                _ => "group.leave.done",
            })))
            .render(cx);
        };
        let fact = |text: gpui_kit::SharedString| div().text_size(text_secondary()).child(text);
        let push = match summary.push {
            PushSetting::MentionsReplies => "group.push.mentions_replies",
            PushSetting::All => "group.push.all",
            PushSetting::None => "group.push.none",
            PushSetting::Unknown(_) => "group.push.mentions_replies",
        };
        let approval =
            if summary.approval { "group.info.approval_on" } else { "group.info.approval_off" };
        let leave_text = match &leave_failure {
            Some(LeaveResult::OwnerProtected) => Some(tr("group.leave.owner")),
            Some(LeaveResult::Failed { code }) => Some(tr(mls::error_key(code))),
            _ => None,
        };
        let group_for = group.clone();
        let body = v_flex()
            .id("mls-info-body")
            .test_support()
            .gap(space(3.))
            .child(
                div()
                    .id("mls-info-name")
                    .test_support()
                    .text_size(gpui_kit::rems(
                        ramflux_desktop_navigation::palette(cx).size_subheading / 15.0,
                    ))
                    .font_semibold()
                    .child(group_name(&summary.name)),
            )
            .child(fact(i18n::join_all(&{
                let mut facts =
                    vec![tr_count("group.info.members", u64::from(summary.members)).to_string()];
                if summary.agents > 0 {
                    facts
                        .push(tr_count("group.info.agents", u64::from(summary.agents)).to_string());
                }
                facts
            })))
            .children(summary.host_ip_visible().then(|| {
                div().id("mls-info-host").test_support().text_size(text_secondary()).child(tr_args(
                    "group.warning.host_ip_visible",
                    &[("host", &i18n::bidi::ltr(&summary.host_node))],
                ))
            }))
            .child(fact(tr(approval)))
            .child(fact(i18n::labeled(&tr("group.info.push"), &tr(push))))
            .children(read_only.map(|reason| {
                div()
                    .id("mls-info-read-only")
                    .role(Role::Status)
                    .text_size(text_secondary())
                    .text_color(danger)
                    .child(tr(match reason {
                        mls::ReadOnly::Left => "group.leave.done",
                        mls::ReadOnly::Removed => "group.state.removed",
                        mls::ReadOnly::Banned => "group.state.banned",
                    }))
            }))
            .children(leave_text.map(|text| {
                div()
                    .id("mls-info-leave-error")
                    .role(Role::Alert)
                    .text_size(text_caption())
                    .text_color(danger)
                    .child(text)
            }))
            .child(
                h_flex()
                    .gap(space(2.))
                    .flex_wrap()
                    // The invites entry is gone once the group is read only.
                    .children(read_only.is_none().then(|| {
                        Button::new("mls-info-invites")
                            .design_secondary(cx)
                            .design_label(tr("group.invite.list"))
                            .on_click({
                                let group = group.clone();
                                cx.listener(move |this, _, window, cx| {
                                    this.panel(
                                        mls::MlsPanel::Invites { group_id: group.clone() },
                                        window,
                                        cx,
                                    );
                                })
                            })
                    }))
                    .child(
                        Button::new("mls-info-members")
                            .design_secondary(cx)
                            .design_label(tr("group.info.members_entry"))
                            .on_click({
                                let group = group.clone();
                                cx.listener(move |this, _, window, cx| {
                                    this.panel(
                                        mls::MlsPanel::Members { group_id: group.clone() },
                                        window,
                                        cx,
                                    );
                                })
                            }),
                    )
                    .child(
                        Button::new("mls-info-refresh")
                            .design_secondary(cx)
                            .design_label(tr("group.info.refresh"))
                            .design_disabled(read_only.is_some(), cx)
                            .on_click({
                                let group = group.clone();
                                cx.listener(move |this, _, window, cx| {
                                    this.sink.emit(
                                        UiEventKind::MlsRefreshPressed { group_id: group.clone() },
                                        window,
                                        cx,
                                    );
                                    this.sink.emit(
                                        UiEventKind::MlsInfoRequested { group_id: group.clone() },
                                        window,
                                        cx,
                                    );
                                })
                            }),
                    )
                    .child(
                        Button::new("mls-info-leave")
                            .design_danger_quiet(cx)
                            .design_label(tr("group.info.leave"))
                            .design_disabled(read_only.is_some() || leaving, cx)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.panel(
                                    mls::MlsPanel::ConfirmLeave { group_id: group_for.clone() },
                                    window,
                                    cx,
                                );
                            })),
                    ),
            );
        crate::modal::Modal::new("mls-info", title, &self.focus, move |_, cx| {
            this.update(cx, |panel, cx| panel.close(cx)).ok();
        })
        .child(body)
        .render(cx)
    }
}

/// The one-time notice about the group's server (shown after a group is
/// created, once per account and group while the app runs).
pub struct MlsHostNotice {
    store: Entity<StoreHandle>,
    group: Option<String>,
    focus: FocusHandle,
}

impl MlsHostNotice {
    /// A closed notice.
    pub fn new(store: Entity<StoreHandle>, cx: &mut Context<Self>) -> Self {
        Self { store, group: None, focus: cx.focus_handle() }
    }

    /// Shows the notice for `group_id`.
    pub fn open(&mut self, group_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.group = Some(group_id);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Whether the notice is open.
    pub fn is_open(&self) -> bool {
        self.group.is_some()
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if self.group.take().is_some() {
            cx.notify();
        }
    }
}

impl Render for MlsHostNotice {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(group) = self.group.clone() else {
            return div().id("mls-host-notice").into_any_element();
        };
        let host = self
            .store
            .read(cx)
            .store()
            .mls_group(&group)
            .map(|g| g.host_node.clone())
            .unwrap_or_default();
        let this = cx.weak_entity();
        let ok = Button::new("mls-host-notice-ok")
            .design_primary(cx)
            .design_label(tr("group.notice.host_ip.ok"))
            .on_click(cx.listener(|this, _, _, cx| this.close(cx)));
        crate::modal::Modal::new(
            "mls-host-notice",
            tr("group.notice.host_ip.title"),
            &self.focus,
            move |_, cx| {
                this.update(cx, |notice, cx| notice.close(cx)).ok();
            },
        )
        .width(crate::modal::width::NARROW)
        .child(
            div()
                .id("mls-host-notice-body")
                .test_support()
                .text_size(text_secondary())
                .child(tr_args("group.notice.host_ip.body", &[("host", &i18n::bidi::ltr(&host))])),
        )
        .button(ok)
        .render(cx)
    }
}
