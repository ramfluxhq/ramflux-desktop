// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The strangers area (design F17, F21, F22): people who are not contacts
//! and asked to talk. The list shows who asked, their note or first held
//! message, and how many messages wait; the detail
//! ([`super::stranger_detail`]) shows the facts and the three fixed
//! actions, Decline, Reply and Accept, plus Block and Report.
//!
//! Replying is not accepting. The backend has no reply-without-acceptance,
//! report or open-account contract yet, so those controls are visible and
//! disabled with the typed reason; nothing is accepted on the way.
//!
//! Batch mode selects several requests. Accept and Decline are validated as
//! a whole against the current store with the core's batch rule before any
//! request leaves; a stale or expired row refuses the whole batch. Block
//! keeps its own confirmation, so it is offered for one request at a time.

use crate::style::DesignButton as _;
use ramflux_desktop_navigation::scale::{space, text_caption};
use std::collections::BTreeSet;
use std::rc::Rc;

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, EventEmitter, IntoElement, Render, Role, SharedString,
    Subscription, Window, div,
};
use ramflux_desktop_core::projection::requests::{BatchError, prepare_batch};
use ramflux_desktop_core::{
    ContactRequest, RequestAnswer, Scope, Store, Timestamp, UiEventKind, selectors,
};
use ramflux_desktop_navigation::direction::{DirectionalExt as _, back_icon, h_flex};
use ramflux_desktop_navigation::generated::catalog;

use super::directory::{self, DirectoryRow, Search};
use crate::actions::{NextConversation, PreviousConversation};
use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{tr, tr_args};

/// What the area reports to the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrangersEvent {
    /// A request was chosen (its detail is shown).
    Selected(String),
    /// The heading's back chevron: return to the contacts page.
    Back,
}

/// Incoming requests that still wait for an answer, in display order.
pub fn pending(store: &Store) -> Vec<&ContactRequest> {
    selectors::pending_contact_requests(store)
}

/// The strangers area's view state.
pub struct StrangersArea {
    pub(super) store: Entity<StoreHandle>,
    pub(super) sink: EventSink,
    search: Search,
    pub(super) selected: Option<String>,
    batch: bool,
    checked: BTreeSet<String>,
    /// Why the last batch was refused (cleared by the next change).
    notice: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<StrangersEvent> for StrangersArea {}

impl StrangersArea {
    /// The area over `store`, emitting through `sink` (the root sink).
    pub fn new(
        store: Entity<StoreHandle>,
        sink: EventSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = Search::new("sections.strangers.search", window, cx);
        let subscriptions = vec![
            cx.subscribe(&store, |this, store, event: &StoreEvent, cx| {
                if let StoreEvent::Changed(changes) = event {
                    if changes.is_reset() {
                        this.selected = None;
                        this.checked.clear();
                        this.batch = false;
                    }
                    // Answered or expired rows leave the selection.
                    let open: BTreeSet<String> =
                        pending(store.read(cx).store()).into_iter().map(|r| r.id.clone()).collect();
                    this.checked.retain(|id| open.contains(id));
                    this.notice = None;
                    cx.notify();
                }
            }),
            cx.subscribe(search.input(), |this, _, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this.search.active = 0;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.pick_active(cx),
                _ => {}
            }),
        ];
        Self {
            store,
            sink: sink.scoped(Scope::Panel("section-strangers".into())),
            search,
            selected: None,
            batch: false,
            checked: BTreeSet::new(),
            notice: None,
            _subscriptions: subscriptions,
        }
    }

    /// Focuses the search field.
    pub fn focus_search(&self, window: &mut Window, cx: &mut App) {
        self.search.focus(window, cx);
    }

    /// Whether batch selection is on.
    pub fn is_batch(&self) -> bool {
        self.batch
    }

    /// The checked request ids.
    pub fn checked(&self) -> &BTreeSet<String> {
        &self.checked
    }

    /// Shows the detail of request `id`.
    pub fn select(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.selected = id;
        cx.notify();
    }

    fn entries<'a>(&self, store: &'a Store, cx: &App) -> Vec<&'a ContactRequest> {
        let query = self.search.query(cx);
        pending(store)
            .into_iter()
            .filter(|request| {
                let name = store
                    .participant(&request.peer)
                    .map_or_else(|| request.peer.to_string(), |p| p.display.clone());
                directory::matches(
                    &query,
                    &[&name, request.peer.as_str(), request.note.as_deref().unwrap_or("")],
                )
            })
            .collect()
    }

    fn ids(&self, cx: &App) -> Vec<String> {
        let store = self.store.read(cx).store();
        self.entries(store, cx).into_iter().map(|r| r.id.clone()).collect()
    }

    fn pick_active(&mut self, cx: &mut Context<Self>) {
        let ids = self.ids(cx);
        if let Some(id) = ids.get(self.search.active.min(ids.len().saturating_sub(1))) {
            self.selected = Some(id.clone());
            cx.emit(StrangersEvent::Selected(id.clone()));
            cx.notify();
        }
    }

    fn step(&mut self, step: isize, cx: &mut Context<Self>) {
        let len = self.ids(cx).len();
        self.search.step(step, len);
        cx.notify();
    }

    fn toggle_batch(&mut self, cx: &mut Context<Self>) {
        self.batch = !self.batch;
        self.checked.clear();
        self.notice = None;
        cx.notify();
    }

    fn toggle_checked(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.checked.remove(id) {
            self.checked.insert(id.to_owned());
        }
        cx.notify();
    }

    fn now(&self, cx: &App) -> Timestamp {
        self.store
            .read(cx)
            .session()
            .map_or_else(crate::app::wall_clock_now, |session| session.now())
    }

    /// Answers every checked request with `answer` after validating the
    /// whole selection; a refused batch sends nothing.
    pub fn answer_checked(
        &mut self,
        answer: RequestAnswer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids: Vec<String> = self.checked.iter().cloned().collect();
        let now = self.now(cx);
        let refusal = {
            let store = self.store.read(cx).store();
            prepare_batch(store, &ids, answer, now).err()
        };
        if let Some(error) = refusal {
            self.notice = Some(batch_refusal(&error));
            cx.notify();
            return;
        }
        for request_id in ids {
            self.sink.emit(UiEventKind::ContactRequestAnswered { request_id, answer }, window, cx);
        }
        self.checked.clear();
        self.batch = false;
        cx.notify();
    }

    fn batch_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self.checked.len();
        let none = count == 0;
        let block_one = count == 1;
        let theme = ramflux_desktop_navigation::palette(cx);
        v_flex()
            .id("strangers-batch")
            .test_support()
            .role(Role::Toolbar)
            .aria_label(tr("sections.strangers.batch"))
            .gap(space(2.))
            .px(space(3.))
            .py(space(2.))
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div().text_size(text_caption()).text_color(cx.theme().muted_foreground).child(
                    tr_args("sections.strangers.selected", &[("count", &count.to_string())]),
                ),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap(space(2.))
                    .child(
                        Button::new("strangers-batch-decline")
                            .small()
                            .design_secondary(cx)
                            .design_label(tr("sections.strangers.decline"))
                            .design_disabled(none, cx)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.answer_checked(RequestAnswer::Decline, window, cx)
                            })),
                    )
                    .child(
                        Button::new("strangers-batch-accept")
                            .small()
                            .design_primary(cx)
                            .design_label(tr("sections.strangers.accept"))
                            .design_disabled(none, cx)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.answer_checked(RequestAnswer::Accept, window, cx)
                            })),
                    )
                    .child({
                        let button = Button::new("strangers-batch-block")
                            .small()
                            .design_danger(cx)
                            .design_label(tr("sections.strangers.block"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.answer_checked(RequestAnswer::Block, window, cx)
                            }));
                        if block_one {
                            button
                        } else {
                            button.disabled(true).tooltip(tr("sections.strangers.block_one"))
                        }
                    }),
            )
            .children(self.notice.clone().map(|notice| {
                div()
                    .id("strangers-batch-notice")
                    .test_support()
                    .role(Role::Alert)
                    .aria_label(notice.clone())
                    .text_size(text_caption())
                    .text_color(theme.danger)
                    .whitespace_normal()
                    .child(notice)
            }))
            .into_any_element()
    }
}

/// What a refused batch says.
fn batch_refusal(error: &BatchError) -> SharedString {
    match error {
        BatchError::Size => tr("sections.strangers.batch_size"),
        BatchError::Duplicate | BatchError::Policy(_) => tr("sections.strangers.batch_stale"),
        BatchError::Expired => tr("sections.strangers.batch_expired"),
    }
}

impl Render for StrangersArea {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (rows, listed, read_error, policy) = {
            let store = self.store.read(cx).store();
            let rows: Vec<DirectoryRow> = self
                .entries(store, cx)
                .into_iter()
                .map(|request| {
                    let name = store
                        .participant(&request.peer)
                        .map_or_else(|| request.peer.short(), |p| p.display.clone());
                    let unverified = store
                        .participant(&request.peer)
                        .is_none_or(|p| p.safety != ramflux_desktop_core::SafetyState::Verified);
                    let body = match (request.note.as_deref(), request.preview.as_deref()) {
                        (_, Some(preview)) => ramflux_desktop_navigation::row::list_text(preview),
                        (Some(note), None) => tr_args(
                            "sections.strangers.note_prefix",
                            &[("note", &ramflux_desktop_navigation::row::list_text(note))],
                        ),
                        (None, None) => tr_args(
                            "sections.strangers.held",
                            &[("count", &request.held_messages.to_string())],
                        ),
                    };
                    DirectoryRow {
                        key: request.id.clone().into(),
                        title: ramflux_desktop_navigation::row::list_text(&name),
                        subtitle: SharedString::default(),
                        avatar: catalog::AvatarKind::Person,
                        trailing: request.expires_at.map(|at| crate::i18n::date_time(at).into()),
                        selected: self.selected.as_deref() == Some(request.id.as_str()),
                        checked: self.batch.then(|| self.checked.contains(&request.id)),
                        card: Some(directory::RowCard {
                            handle: super::stranger_detail::handle(&request.peer),
                            chip: unverified.then(|| tr("sections.strangers.unverified_chip")),
                            body,
                        }),
                    }
                })
                .collect();
            (
                rows,
                store.contact_requests_read_error().is_none(),
                crate::workspace::docks::contacts::requests::read_error(store, cx),
                crate::workspace::docks::contacts::requests::policy_switch(store, &self.sink, cx),
            )
        };
        let count = rows.len();
        let empty = rows.is_empty().then(|| {
            directory::placeholder(
                "strangers-empty",
                tr(if listed { "sections.strangers.empty" } else { "sections.strangers.unread" }),
                tr("sections.strangers.empty_help"),
                cx,
            )
        });
        let this = cx.weak_entity();
        let pick: directory::RowAction = Rc::new(move |key, _, cx| {
            let id = key.to_owned();
            this.update(cx, |area, cx| {
                area.selected = Some(id.clone());
                cx.emit(StrangersEvent::Selected(id));
                cx.notify();
            })
            .ok();
        });
        let this = cx.weak_entity();
        let check: directory::RowAction = Rc::new(move |key, _, cx| {
            this.update(cx, |area, cx| area.toggle_checked(key, cx)).ok();
        });
        let batch = self.batch;
        v_flex()
            .id("strangers-area")
            .test_support()
            .key_context("ConversationList")
            .on_action(cx.listener(|this, _: &NextConversation, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousConversation, _, cx| this.step(-1, cx)))
            .size_full()
            .min_w_0()
            .bg(ramflux_desktop_navigation::palette(cx).nav)
            .border_end_1()
            .border_color(ramflux_desktop_navigation::palette(cx).line)
            .px(space(3.))
            .pt(gpui_kit::rems(24.0 / 15.0))
            .gap(space(4.))
            // Board F17: back chevron, Strangers, Batch at the right.
            .child(
                h_flex()
                    .gap(space(1.))
                    .child(
                        Button::new("strangers-back")
                            .small()
                            .ghost()
                            .icon(back_icon())
                            .accessibility_label(tr("sections.strangers.back"))
                            .tooltip(tr("sections.strangers.back"))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(StrangersEvent::Back))),
                    )
                    .child(
                        div().flex_1().min_w_0().child(directory::header(
                            tr("sections.strangers.heading"),
                            Button::new("strangers-batch-toggle")
                                .small()
                                .ghost()
                                .label(tr("sections.strangers.batch"))
                                .accessibility_label(tr_args(
                                    "sections.strangers.title",
                                    &[("count", &count.to_string())],
                                ))
                                .selected(batch)
                                .toggled(batch)
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_batch(cx))),
                            cx,
                        )),
                    ),
            )
            .child(self.search.field("strangers-search", tr("sections.strangers.search"), cx))
            .children(read_error)
            .children(empty)
            .child(directory::rows(
                directory::RowList {
                    id: "strangers-list",
                    label: tr("sections.strangers.list"),
                    rows,
                    active: self.search.cursor(window, cx),
                    on_pick: pick,
                    on_check: Some(check),
                },
                cx,
            ))
            .when(batch, |area| area.child(self.batch_bar(cx)))
            .child(
                v_flex()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .py(space(1.))
                    .child(policy)
                    .child(
                        h_flex().px(space(3.)).py(space(1.)).gap(space(2.)).child(
                            crate::gate::gated(
                                Button::new("strangers-open-account")
                                    .xsmall()
                                    .design_secondary(cx)
                                    .design_label(tr("sections.strangers.open_account")),
                                Some(directory::unavailable(
                                    ramflux_desktop_sdk::business::ContractGap::AccountPrivacy,
                                )),
                                cx,
                            ),
                        ),
                    ),
            )
    }
}
