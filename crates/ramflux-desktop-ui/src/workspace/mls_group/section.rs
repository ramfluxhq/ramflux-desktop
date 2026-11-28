// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The MLS groups list (boards C05, C32 to C35, I15, I16), shown in the
//! Groups route while the app runs on the mock source.
//!
//! The list is virtualized and pages by the SDK's cursor: the next page is
//! asked when the list is scrolled within ten rows of its end, and no more
//! once the cursor ends. States: loading, offline, error with a Try again
//! tap (a read is never repeated by itself), and empty with "New group" and
//! "Join with a link". A join that waits for approval shows a row at the top
//! until it resolves or expires. Choosing a row shows the `mls:<group_id>`
//! context in the shared conversation view.

use std::rc::Rc;

use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _, VirtualListScrollHandle, v_flex,
    v_virtual_list,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, Entity, EventEmitter, IntoElement, Pixels, Render, SharedString, Size, Subscription,
    Window, div, px, size,
};
use ramflux_desktop_core::mls_group::{self as mls, ListPhase};
use ramflux_desktop_core::{ContextId, ListState, Store, Timestamp, UiEventKind};
use ramflux_desktop_navigation::direction::{DirectionalExt as _, h_flex};
use ramflux_desktop_navigation::row::{AvatarKind, Row};
use ramflux_desktop_navigation::scale::{space, text_caption};
use ramflux_desktop_navigation::theme;

use super::{group_name, when};
use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args, tr_count};
use crate::ids::domain_id;
use crate::workspace::sections::directory::{self, StateKeys, StateSpec};

/// How close to the end of the list the next page is asked for.
const NEAR_END: usize = 10;

/// Reported to the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MlsSectionEvent {
    /// Show this group.
    Selected(ContextId),
}

/// One listed group, as the row shows it.
struct Entry {
    id: ContextId,
    title: SharedString,
    preview: SharedString,
    time: SharedString,
    unread: u32,
}

/// The list view.
pub struct MlsGroupsSection {
    store: Entity<StoreHandle>,
    sink: EventSink,
    selected: Option<ContextId>,
    rows: Rc<Vec<Entry>>,
    sizes: Rc<Vec<Size<Pixels>>>,
    row_height: Pixels,
    scroll: VirtualListScrollHandle,
    /// The cursor the next page was asked with (asked once per cursor).
    asked: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<MlsSectionEvent> for MlsGroupsSection {}

impl MlsGroupsSection {
    /// A list over the rendered account's store, asking through `sink`.
    pub fn new(
        store: Entity<StoreHandle>,
        sink: EventSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.subscribe(&store, |_, _, event: &StoreEvent, cx| {
                if let StoreEvent::Changed(changes) = event
                    && (changes.is_reset() || !changes.domain().mls.is_empty())
                {
                    cx.notify();
                }
            }),
            i18n::on_locale_switch(cx, |_, cx| cx.notify()),
        ];
        let _ = window;
        Self {
            store,
            sink: sink.scoped(ramflux_desktop_core::Scope::Panel("mls_groups".to_owned())),
            selected: None,
            rows: Rc::new(Vec::new()),
            sizes: Rc::new(Vec::new()),
            row_height: px(0.),
            scroll: VirtualListScrollHandle::new(),
            asked: None,
            _subscriptions: subscriptions,
        }
    }

    /// Marks the shown group.
    pub fn set_selected(&mut self, id: Option<ContextId>, cx: &mut Context<Self>) {
        if self.selected != id {
            self.selected = id;
            cx.notify();
        }
    }

    /// The list was opened: the first page and the join requests the SDK
    /// holds are read once (a later read is the user's Refresh or a scroll).
    pub fn opened(&mut self, window: &mut Window, cx: &mut gpui_kit::App) {
        let idle = self.store.read(cx).store().mls_list().phase == ListPhase::Idle;
        if idle {
            self.asked = None;
            self.sink.emit(UiEventKind::MlsListRequested { cursor: None }, window, cx);
            self.sink.emit(UiEventKind::MlsJoinPendingRequested, window, cx);
        }
    }

    /// The ids in display order (tests).
    pub fn entry_ids(&self) -> Vec<ContextId> {
        self.rows.iter().map(|e| e.id.clone()).collect()
    }

    fn entries(store: &Store, now: Timestamp) -> Vec<Entry> {
        store
            .mls_groups()
            .into_iter()
            .map(|g| {
                let members = tr_args("group.list.members", &[("count", &g.members.to_string())]);
                let mut facts = vec![members];
                if g.agents > 0 {
                    facts.push(tr_args("group.list.agents", &[("count", &g.agents.to_string())]));
                }
                if g.unread > 0 && g.unread_mentions > 0 {
                    facts.push(tr("group.list.mention"));
                }
                let preview = match g.last_post.as_ref().and_then(|p| p.snippet.clone()) {
                    Some(snippet) => ramflux_desktop_navigation::row::list_text(&snippet),
                    None => i18n::join_all(&facts),
                };
                let time = g
                    .last_activity_at
                    .map(|at| {
                        if i18n::format::same_day(at, now) {
                            i18n::time_of_day(at).into()
                        } else {
                            i18n::format::date(at).into()
                        }
                    })
                    .unwrap_or_default();
                Entry {
                    id: mls::context_id(&g.group_id),
                    title: group_name(&g.name),
                    preview,
                    time,
                    unread: g.unread,
                }
            })
            .collect()
    }

    fn ask(&self, cursor: Option<String>, window: &mut Window, cx: &mut gpui_kit::App) {
        self.sink.emit(UiEventKind::MlsListRequested { cursor }, window, cx);
    }
}

impl Render for MlsGroupsSection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let handle = self.store.read(cx);
        let store = handle.store();
        let now = handle.session().map(|s| s.now()).unwrap_or_default();
        let source = store.source_status();
        let list = store.mls_list().clone();
        let entries = Self::entries(store, now);
        // A request past its expiry shows no more (nothing is called).
        let pending: Vec<_> = store
            .mls_join()
            .records
            .values()
            .filter(|r| r.expires_at.is_none_or(|at| at > now))
            .cloned()
            .collect();
        let failed = match &list.phase {
            ListPhase::Failed(code) => Some(tr(ramflux_desktop_core::mls_group::error_key(code))),
            _ => None,
        };
        let state_list = match (&list.phase, entries.is_empty()) {
            (_, false) => ListState::Ready,
            (ListPhase::Ready, true) => ListState::Empty,
            (ListPhase::Failed(_), true) => ListState::Empty,
            _ => ListState::Loading,
        };
        let theme = ramflux_desktop_navigation::palette(cx);
        let row_height = window.rem_size() * theme::ROW_REM + window.rem_size() * (16.0 / 15.0);
        if self.row_height != row_height || self.sizes.len() != entries.len() {
            self.row_height = row_height;
            self.sizes = Rc::new(vec![size(px(0.), row_height); entries.len()]);
        }
        let total = entries.len();
        self.rows = Rc::new(entries);
        let cursor = list.cursor.clone();
        let this = cx.weak_entity();

        let create = Button::new("mls-groups-create")
            .small()
            .ghost()
            .icon(IconName::Plus)
            .accessibility_label(tr("group.list.create"))
            .tooltip(tr("group.list.create"))
            .on_click(cx.listener(|this, _, window, cx| {
                this.sink.emit(
                    UiEventKind::MlsPanelRequested { panel: mls::MlsPanel::Create },
                    window,
                    cx,
                );
            }));
        let join = Button::new("mls-groups-join")
            .small()
            .ghost()
            .icon(IconName::Link)
            .accessibility_label(tr("group.list.join"))
            .tooltip(tr("group.list.join"))
            .on_click(cx.listener(|this, _, window, cx| {
                this.sink.emit(
                    UiEventKind::MlsPanelRequested { panel: mls::MlsPanel::Join },
                    window,
                    cx,
                );
            }));
        let refresh = Button::new("mls-groups-refresh")
            .small()
            .ghost()
            .icon(IconName::RefreshCw)
            .accessibility_label(tr("group.info.refresh"))
            .tooltip(tr("group.info.refresh"))
            .on_click(cx.listener(|this, _, window, cx| {
                // Refresh restarts from the first page; the rows stay until
                // it arrives.
                this.asked = None;
                this.ask(None, window, cx);
            }));
        let action = if failed.is_some() {
            Some(
                Button::new("mls-groups-retry")
                    .small()
                    .label(tr("group.list.retry"))
                    .on_click(cx.listener(|this, _, window, cx| this.ask(None, window, cx)))
                    .into_any_element(),
            )
        } else if state_list == ListState::Empty {
            Some(
                h_flex()
                    .gap(space(2.))
                    .child(
                        Button::new("mls-groups-empty-create")
                            .small()
                            .label(tr("group.list.create"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.sink.emit(
                                    UiEventKind::MlsPanelRequested { panel: mls::MlsPanel::Create },
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("mls-groups-empty-join")
                            .small()
                            .label(tr("group.list.join"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.sink.emit(
                                    UiEventKind::MlsPanelRequested { panel: mls::MlsPanel::Join },
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .into_any_element(),
            )
        } else {
            None
        };
        let state = directory::state(
            StateSpec {
                id: "mls-groups-state",
                list: state_list,
                status: source,
                failure: failed,
                keys: StateKeys {
                    loading: "group.list.loading",
                    offline: "group.list.offline",
                    failed: "group.list.error",
                    empty: "group.list.empty",
                    empty_help: "group.list.offline",
                },
                action,
            },
            cx,
        );
        let sizes = self.sizes.clone();
        let rows_list =
            v_virtual_list(cx.entity(), "mls-group-rows", sizes, move |this, range, window, cx| {
                // The next page is asked once the end is within reach, once
                // per cursor.
                if let Some(cursor) = this.store.read(cx).store().mls_list().cursor.clone()
                    && range.end + NEAR_END >= this.rows.len()
                    && this.asked.as_ref() != Some(&cursor)
                {
                    this.asked = Some(cursor.clone());
                    let weak = cx.weak_entity();
                    window.defer(cx, move |window, cx| {
                        weak.update(cx, |this, cx| this.ask(Some(cursor), window, cx)).ok();
                    });
                }
                range
                    .filter_map(|ix| {
                        let entry = this.rows.get(ix)?;
                        let id = entry.id.clone();
                        let selected = this.selected.as_ref() == Some(&id);
                        let mut content = Row::new(entry.title.clone(), entry.preview.clone());
                        content.time = entry.time.clone();
                        content.unread = entry.unread;
                        content.kind = AvatarKind::Person;
                        Some(
                            div()
                                .pb(gpui_kit::rems(16.0 / 15.0))
                                .child(
                                    Button::new(domain_id("mls-group", &id))
                                        .ghost()
                                        .w_full()
                                        .h(gpui_kit::rems(theme::ROW_REM))
                                        .px(space(3.))
                                        .rounded(gpui_kit::px(
                                            ramflux_desktop_navigation::palette(cx).radius_md,
                                        ))
                                        .selected(selected)
                                        .accessibility_label(if entry.unread > 0 {
                                            i18n::join(
                                                &entry.title,
                                                &tr_count(
                                                    "group.list.unread",
                                                    u64::from(entry.unread),
                                                ),
                                            )
                                        } else {
                                            entry.title.clone()
                                        })
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.emit(MlsSectionEvent::Selected(id.clone()))
                                        }))
                                        .child(content.content(cx)),
                                )
                                .into_any_element(),
                        )
                    })
                    .collect()
            })
            .track_scroll(&self.scroll);
        let _ = (this, total, cursor);
        v_flex()
            .id("mls-groups-section")
            .test_support()
            .h_full()
            .w_full()
            .min_w_0()
            .min_h_0()
            .bg(theme.nav)
            .border_end_1()
            .border_color(theme.line)
            .px(space(3.))
            .pt(gpui_kit::rems(24.0 / 15.0))
            .gap(space(4.))
            .child(directory::header(
                tr("group.list.title"),
                h_flex().gap(space(1.)).child(refresh).child(join).child(create),
                cx,
            ))
            .children(pending.into_iter().map(|record| {
                div()
                    .id(domain_id("mls-pending", &ContextId::new(record.request_id.clone())))
                    .test_support()
                    .role(gpui_kit::Role::Status)
                    .px(space(3.))
                    .py(space(2.))
                    .rounded(gpui_kit::px(theme.radius_md))
                    .bg(theme.selected)
                    .text_size(text_caption())
                    .text_color(cx.theme().muted_foreground)
                    .child(match record.expires_at {
                        Some(at) => i18n::join(&tr("group.join.pending.row"), &when(at)),
                        None => tr("group.join.pending.row"),
                    })
            }))
            .children(state)
            .child(div().flex_1().min_h_0().child(rows_list))
    }
}
