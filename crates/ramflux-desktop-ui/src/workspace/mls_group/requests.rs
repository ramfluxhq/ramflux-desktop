// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The join requests of an MLS group (derived design D-M3-1, from the
//! strangers list): a virtualized list, oldest request first, for an owner or
//! admin of a group that needs approval.
//!
//! - **Nothing is sent by looking.** Opening the screen loads the first page
//!   once (never while it is loaded or loading); the next page loads when the
//!   list is within 20 rows of its end, once per cursor; a failed page waits
//!   for the user's "Try again".
//! - **Approve and Decline open a confirmation** that names the person (the
//!   display name, or "this member"; never an id). Only its confirm button
//!   sends. A row shows "Approving…", "Adding to the group…" or "Declining…"
//!   from the store, and leaves the list only when the store says so: after
//!   the committed reply and the `joined` row for Approve (A5), after the
//!   reply for Decline.
//! - **Expiry** is read against the clock: a request past its `expires_at`
//!   reads "Expired" and its buttons are off. A timer wakes the view to
//!   redraw; it calls nothing, and the row leaves on the next list read.
//! - **Busy and paced** disable Approve with the typed text; the view
//!   re-enables it at the time the SDK gave, through one wake that only
//!   redraws.
//! - With approval off the screen shows the notice and no list.

use std::rc::Rc;

use crate::style::DesignButton as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, VirtualListScrollHandle, v_flex, v_virtual_list};
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, Entity, FocusHandle, IntoElement, Pixels, Render, Role, SharedString, Size,
    Subscription, Window, div, px, size,
};
use ramflux_desktop_core::mls_group::{self as mls, OpaqueId, RequestRow, RequestRowState};
use ramflux_desktop_core::{Scope, Timestamp, UiEventKind};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};

use super::{name_text, when};
use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{tr, tr_args};

/// How close to the end of the list the next page is asked for.
const NEAR_END: usize = 20;
/// Height of a row, in design pixels.
const ROW_PX: f32 = 88.0;

/// One line of the list.
#[derive(Clone)]
enum Line {
    Row(Box<Item>),
    /// A later page is on its way.
    Loading,
    /// The page error row.
    PageError,
}

/// What a row shows, read from the store.
#[derive(Clone)]
struct Item {
    id: OpaqueId,
    name: SharedString,
    ai: bool,
    requested: Option<SharedString>,
    expires: Option<SharedString>,
    expired: bool,
    state: RequestRowState,
    failure: Option<&'static str>,
    can_approve: bool,
    can_decline: bool,
}

/// The screen and its view-local state.
pub struct MlsRequests {
    store: Entity<StoreHandle>,
    sink: EventSink,
    group: Option<String>,
    lines: Rc<Vec<Line>>,
    sizes: Rc<Vec<Size<Pixels>>>,
    scroll: VirtualListScrollHandle,
    /// The cursor the last page was asked with (once per cursor).
    asked: Option<Option<String>>,
    /// The wake for the next expiry or the end of pacing: it only redraws.
    wake: Option<(Timestamp, gpui_kit::Task<()>)>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl MlsRequests {
    /// A closed screen.
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
            // The window going to the background closes the screen; the store
            // keeps every outcome.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.close(cx);
                }
            }),
        ];
        Self {
            store,
            sink: sink.scoped(Scope::Panel("mls_requests".to_owned())),
            group: None,
            lines: Rc::new(Vec::new()),
            sizes: Rc::new(Vec::new()),
            scroll: VirtualListScrollHandle::new(),
            asked: None,
            wake: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Opens the screen of `group_id`. The first page is asked once, and not
    /// at all while it is loaded or loading, or when the group takes no
    /// requests.
    pub fn open(&mut self, group_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let store = self.store.read(cx).store();
        let wanted = store.mls_is_admin(&group_id)
            && store.mls_group(&group_id).is_some_and(|g| g.approval)
            && store.mls_requests(&group_id).is_none_or(|p| !p.loaded && !p.loading);
        self.asked = None;
        if wanted {
            self.sink.emit(
                UiEventKind::MlsRequestsRequested { group_id: group_id.clone(), cursor: None },
                window,
                cx,
            );
        }
        self.group = Some(group_id);
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Closes the screen.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.group.take().is_some() {
            self.wake = None;
            cx.notify();
        }
    }

    /// Whether the screen is open.
    pub fn is_open(&self) -> bool {
        self.group.is_some()
    }

    fn ask(&mut self, cursor: Option<String>, window: &mut Window, cx: &mut gpui_kit::App) {
        let Some(group_id) = self.group.clone() else { return };
        self.asked = Some(cursor.clone());
        self.sink.emit(UiEventKind::MlsRequestsRequested { group_id, cursor }, window, cx);
    }

    fn panel(&mut self, panel: mls::MlsPanel, window: &mut Window, cx: &mut Context<Self>) {
        self.sink.emit(UiEventKind::MlsPanelRequested { panel }, window, cx);
    }

    /// Arms one wake for `at` (an expiry, or the end of pacing); it only
    /// redraws. Dropped when the screen closes.
    fn follow(&mut self, at: Option<Timestamp>, now: Timestamp, cx: &mut Context<Self>) {
        match at {
            None => self.wake = None,
            Some(at) if self.wake.as_ref().map(|(w, _)| *w) != Some(at) => {
                let wait = std::time::Duration::from_millis(at.saturating_since(now) + 50);
                let task = cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(wait).await;
                    this.update(cx, |screen, cx| {
                        screen.wake = None;
                        cx.notify();
                    })
                    .ok();
                });
                self.wake = Some((at, task));
            }
            Some(_) => {}
        }
    }

    fn items(
        &self,
        group: &str,
        now: Timestamp,
        cx: &gpui_kit::App,
    ) -> (Vec<Line>, Option<Timestamp>) {
        let store = self.store.read(cx).store();
        let Some(pager) = store.mls_requests(group) else { return (Vec::new(), None) };
        let paced = store.mls_paced_until(group, now).is_some();
        let busy = store.mls_admin_busy(group);
        let mut next: Option<Timestamp> = store.mls_paced_until(group, now);
        let mut lines: Vec<Line> = pager
            .rows
            .iter()
            .map(|row: &RequestRow| {
                let request = &row.request;
                let expired = request.expires_at.is_some_and(|at| at.0 <= now.0);
                if let Some(at) = request.expires_at.filter(|at| at.0 > now.0) {
                    next = Some(next.map_or(at, |n| n.min(at)));
                }
                let idle = row.state == RequestRowState::Idle;
                Line::Row(Box::new(Item {
                    id: request.request_id.clone(),
                    name: name_text(request.display.as_deref(), false),
                    ai: request.ai_flag || store.mls_is_ai(group, request.anchor.expose()),
                    requested: request.requested_at.map(|at| when(at).into()),
                    expires: request.expires_at.map(|at| when(at).into()),
                    expired,
                    state: row.state,
                    failure: row.failure.as_deref().map(mls::error_key),
                    can_approve: idle && !expired && !busy && !paced,
                    can_decline: idle && !expired,
                }))
            })
            .collect();
        if pager.loading_more {
            lines.push(Line::Loading);
        } else if pager.page_error.is_some() {
            lines.push(Line::PageError);
        }
        (lines, next)
    }
}

impl Render for MlsRequests {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(group) = self.group.clone() else {
            return div().id("mls-requests").into_any_element();
        };
        let now = self.store.read(cx).session().map(|s| s.now()).unwrap_or_default();
        let (approval, admin, group_title, error, loaded, loading, cursor, page_error, count) = {
            let store = self.store.read(cx).store();
            let pager = store.mls_requests(&group);
            (
                store.mls_group(&group).is_some_and(|g| g.approval),
                store.mls_is_admin(&group),
                store.mls_group(&group).map(|g| super::group_name(&g.name)),
                pager.and_then(|p| p.error.clone()),
                pager.is_some_and(|p| p.loaded),
                pager.is_some_and(|p| p.loading),
                pager.and_then(|p| p.cursor.clone()),
                pager.is_some_and(|p| p.page_error.is_some()),
                pager.map_or(0, |p| p.rows.len()),
            )
        };
        let (lines, next) = self.items(&group, now, cx);
        self.follow(next, now, cx);
        let rows_px = window.rem_size() * (ROW_PX / 15.0);
        self.sizes = Rc::new(vec![size(px(0.), rows_px); lines.len()]);
        self.lines = Rc::new(lines);
        let muted = cx.theme().muted_foreground;
        let danger = crate::tone::text(cx.theme().danger, cx);
        let (status, status_danger) = {
            let store = self.store.read(cx).store();
            if let Some(until) = store.mls_paced_until(&group, now) {
                (
                    Some(tr_args(
                        "group.error.paced",
                        &[("time", &crate::i18n::time_of_day(until))],
                    )),
                    false,
                )
            } else if store.mls_admin_busy(&group) {
                (Some(tr("group.member.action.busy")), false)
            } else if let Some(failure) =
                store.mls_admin_failure(&group).filter(|f| f.kind == mls::AdminKind::Approve)
            {
                (Some(tr(mls::error_key(&failure.code))), true)
            } else {
                (None, false)
            }
        };
        let this = cx.weak_entity();
        let list_group = group.clone();
        let list =
            v_virtual_list(
                cx.entity(),
                "mls-request-rows",
                self.sizes.clone(),
                move |this, range, window, cx| {
                    let cursor = this
                        .store
                        .read(cx)
                        .store()
                        .mls_requests(&list_group)
                        .and_then(|p| p.cursor.clone());
                    if range.end + NEAR_END >= this.lines.len()
                        && let Some(next) = cursor
                        && this.asked.as_ref() != Some(&Some(next.clone()))
                        && this
                            .store
                            .read(cx)
                            .store()
                            .mls_requests(&list_group)
                            .is_some_and(|p| !p.loading_more && p.page_error.is_none() && p.loaded)
                    {
                        this.asked = Some(Some(next.clone()));
                        let weak = cx.weak_entity();
                        window.defer(cx, move |window, cx| {
                            weak.update(cx, |this, cx| this.ask(Some(next), window, cx)).ok();
                        });
                    }
                    let muted = cx.theme().muted_foreground;
                    let danger = crate::tone::text(cx.theme().danger, cx);
                    let group = list_group.clone();
                    range
                        .filter_map(|ix| {
                            let line = this.lines.get(ix)?.clone();
                            Some(match line {
                                Line::Loading => div()
                                    .id("mls-requests-loading-more")
                                    .role(Role::Status)
                                    .h(gpui_kit::rems(ROW_PX / 15.0))
                                    .flex()
                                    .items_center()
                                    .text_size(text_caption())
                                    .text_color(muted)
                                    .child(tr("list.loading"))
                                    .into_any_element(),
                                Line::PageError => h_flex()
                                    .id("mls-requests-page-error")
                                    .role(Role::Alert)
                                    .h(gpui_kit::rems(ROW_PX / 15.0))
                                    .gap(space(2.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(text_secondary())
                                            .child(tr("group.requests.page_error")),
                                    )
                                    .child(
                                        Button::new("mls-requests-page-retry")
                                            .design_secondary(cx)
                                            .design_label(tr("group.requests.retry"))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                let cursor = this
                                                    .group
                                                    .as_deref()
                                                    .and_then(|g| {
                                                        this.store.read(cx).store().mls_requests(g)
                                                    })
                                                    .and_then(|p| p.cursor.clone());
                                                this.ask(cursor, window, cx);
                                            })),
                                    )
                                    .into_any_element(),
                                Line::Row(item) => {
                                    let status = match item.state {
                                        RequestRowState::Approving => {
                                            Some(tr("group.requests.approving"))
                                        }
                                        RequestRowState::Adding => {
                                            Some(tr("group.requests.adding"))
                                        }
                                        RequestRowState::Declining => {
                                            Some(tr("group.requests.declining"))
                                        }
                                        RequestRowState::Idle => None,
                                    };
                                    let when_line = if item.expired {
                                        Some(tr("group.requests.expired"))
                                    } else {
                                        item.expires.as_ref().map(|t| {
                                            tr_args("group.requests.expires", &[("time", t)])
                                        })
                                    };
                                    let asked = item.requested.as_ref().map(|t| {
                                        tr_args("group.requests.requested", &[("time", t)])
                                    });
                                    let approve_id = item.id.clone();
                                    let decline_id = item.id.clone();
                                    let approve_group = group.clone();
                                    let decline_group = group.clone();
                                    let ix_id = ix;
                                    h_flex()
                                        .id(SharedString::from(format!("mls-request-{ix_id}")))
                                        .test_support()
                                        .role(Role::ListItem)
                                        .h(gpui_kit::rems(ROW_PX / 15.0))
                                        .gap(space(2.))
                                        .px(space(2.))
                                        .child(
                                            v_flex()
                                                .flex_1()
                                                .min_w_0()
                                                .gap(space(0.5))
                                                .child(
                                                    h_flex()
                                                        .gap(space(2.))
                                                        .child(
                                                            div()
                                                                .text_size(text_secondary())
                                                                .child(item.name.clone()),
                                                        )
                                                        .children(item.ai.then(|| {
                                                            crate::identity::ai_mark(
                                                                "mls-request-ai",
                                                                cx,
                                                            )
                                                        })),
                                                )
                                                .children(asked.map(|text| {
                                                    div()
                                                        .text_size(text_caption())
                                                        .text_color(muted)
                                                        .child(text)
                                                }))
                                                .children(when_line.map(|text| {
                                                    div()
                                                        .text_size(text_caption())
                                                        .text_color(muted)
                                                        .child(text)
                                                }))
                                                .children(status.map(|text| {
                                                    div()
                                                        .id(SharedString::from(format!(
                                                            "mls-request-status-{ix_id}"
                                                        )))
                                                        .role(Role::Status)
                                                        .text_size(text_caption())
                                                        .text_color(muted)
                                                        .child(text)
                                                }))
                                                .children(item.failure.map(|key| {
                                                    div()
                                                        .id(SharedString::from(format!(
                                                            "mls-request-failure-{ix_id}"
                                                        )))
                                                        .role(Role::Alert)
                                                        .text_size(text_caption())
                                                        .text_color(danger)
                                                        .child(tr(key))
                                                })),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "mls-request-decline-{ix_id}"
                                            )))
                                            .design_secondary(cx)
                                            .design_label(tr("group.requests.decline"))
                                            .design_disabled(!item.can_decline, cx)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.panel(
                                                    mls::MlsPanel::ConfirmDecline {
                                                        group_id: decline_group.clone(),
                                                        request_id: decline_id.clone(),
                                                    },
                                                    window,
                                                    cx,
                                                );
                                            })),
                                        )
                                        .child(
                                            Button::new(SharedString::from(format!(
                                                "mls-request-approve-{ix_id}"
                                            )))
                                            .design_primary(cx)
                                            .design_label(tr("group.requests.approve"))
                                            .design_disabled(!item.can_approve, cx)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.panel(
                                                    mls::MlsPanel::ConfirmApprove {
                                                        group_id: approve_group.clone(),
                                                        request_id: approve_id.clone(),
                                                    },
                                                    window,
                                                    cx,
                                                );
                                            })),
                                        )
                                        .into_any_element()
                                }
                            })
                        })
                        .collect()
                },
            )
            .track_scroll(&self.scroll);
        let first_loading = loading || (!loaded && error.is_none());
        let body = if !approval || !admin {
            // Approval is off (or the role changed): no list, no calls.
            v_flex().id("mls-requests-body").test_support().gap(space(3.)).child(
                div()
                    .id("mls-requests-approval-off")
                    .role(Role::Status)
                    .text_size(text_secondary())
                    .text_color(muted)
                    .child(tr("group.requests.approval_off")),
            )
        } else if let Some(_code) = error {
            v_flex()
                .id("mls-requests-body")
                .test_support()
                .gap(space(3.))
                .child(
                    div()
                        .id("mls-requests-load-error")
                        .role(Role::Alert)
                        .text_size(text_secondary())
                        .text_color(danger)
                        .child(tr("group.requests.load_error")),
                )
                .child(
                    Button::new("mls-requests-retry")
                        .design_secondary(cx)
                        .design_label(tr("group.requests.retry"))
                        .on_click(cx.listener(|this, _, window, cx| this.ask(None, window, cx))),
                )
        } else if first_loading {
            v_flex().id("mls-requests-body").test_support().child(
                div()
                    .id("mls-requests-loading")
                    .role(Role::Status)
                    .text_size(text_secondary())
                    .text_color(muted)
                    .child(tr("list.loading")),
            )
        } else if count == 0 && cursor.is_none() && !page_error {
            v_flex().id("mls-requests-body").test_support().child(
                div()
                    .id("mls-requests-empty")
                    .role(Role::Status)
                    .text_size(text_secondary())
                    .text_color(muted)
                    .child(tr("group.requests.empty")),
            )
        } else {
            v_flex()
                .id("mls-requests-body")
                .test_support()
                .gap(space(3.))
                .children(status.map(|text| {
                    div()
                        .id("mls-requests-status")
                        .role(Role::Status)
                        .text_size(text_caption())
                        .text_color(if status_danger { danger } else { muted })
                        .child(text)
                }))
                .child(div().h(gpui_kit::rems(360.0 / 15.0)).min_h_0().child(list))
        };
        let title = match group_title {
            Some(name) => crate::i18n::labeled(&tr("group.requests.title"), &name),
            None => tr("group.requests.title"),
        };
        crate::modal::Modal::new("mls-requests", title, &self.focus, move |_, cx| {
            this.update(cx, |screen, cx| screen.close(cx)).ok();
        })
        .child(body)
        .render(cx)
    }
}
