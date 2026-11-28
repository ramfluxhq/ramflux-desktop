// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The members of an MLS group (boards C02, C04, H03): a virtualized list
//! under role headers (Owner, Admins, Members), the AI marker from the SDK's
//! `ai_flag` alone (S8), "You", the device count, and "Member" for a member
//! with no name (never the anchor).
//!
//! The first page (200 rows) loads when the panel opens; the next one when
//! the list is within 20 rows of its end, once per cursor. A page that
//! failed shows an error row with "Try again": the retry is the user's tap,
//! never automatic. Search filters the rows loaded; while a search runs and
//! pages remain, the remaining pages load, with a progress line.
//!
//! The same list serves the mention picker: a row chosen there adds that
//! member to the mentions of the next post (at most 50).

use std::rc::Rc;

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _, VirtualListScrollHandle, v_flex,
    v_virtual_list,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, Entity, FocusHandle, IntoElement, Pixels, Render, Role, SharedString, Size,
    Subscription, Window, div, px, size,
};
use ramflux_desktop_core::mls_group::{MemberRow, MlsRole};
use ramflux_desktop_core::{Scope, UiEventKind};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};

use super::{MentionPick, member_name, picked_mentions, toggle_mention};
use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args, tr_count};

/// How close to the end of the list the next page is asked for.
const NEAR_END: usize = 20;
/// Height of a row, in design pixels.
const ROW_PX: f32 = 44.0;

/// What the panel is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// The member list.
    List,
    /// Picking the members the next post mentions.
    Mentions,
}

/// One line of the list.
#[derive(Clone)]
enum Line {
    Header(SharedString),
    Member(Box<Item>),
    /// The page error row.
    Error,
    /// The progress line of a search that loads the remaining pages.
    Progress(SharedString),
}

#[derive(Clone)]
struct Item {
    anchor: String,
    name: SharedString,
    ai: bool,
    devices: u32,
    me: bool,
}

/// The panel and its view-local state.
pub struct MlsMembers {
    store: Entity<StoreHandle>,
    sink: EventSink,
    group: Option<String>,
    mode: Mode,
    search: Entity<InputState>,
    lines: Rc<Vec<Line>>,
    sizes: Rc<Vec<Size<Pixels>>>,
    scroll: VirtualListScrollHandle,
    /// The cursor the page last asked for was asked with.
    asked: Option<Option<String>>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl MlsMembers {
    /// A closed panel.
    pub fn new(
        store: Entity<StoreHandle>,
        sink: EventSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = i18n::localized_input("group.members.search", window, cx, |input| input);
        let subscriptions = vec![
            cx.subscribe(&store, |this, _, event: &StoreEvent, cx| {
                if matches!(event, StoreEvent::Changed(_)) && this.group.is_some() {
                    cx.notify();
                }
            }),
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
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
            sink: sink.scoped(Scope::Panel("mls_members".to_owned())),
            group: None,
            mode: Mode::List,
            search,
            lines: Rc::new(Vec::new()),
            sizes: Rc::new(Vec::new()),
            scroll: VirtualListScrollHandle::new(),
            asked: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Opens the member list of `group_id`; the first page is asked once.
    pub fn open(
        &mut self,
        group_id: String,
        mentions: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mode = if mentions { Mode::Mentions } else { Mode::List };
        self.asked = None;
        self.search.update(cx, |search, cx| search.set_value("", window, cx));
        let loaded = self.store.read(cx).store().mls_members(&group_id).is_some_and(|p| p.loaded);
        if !loaded {
            self.asked = Some(None);
            self.sink.emit(
                UiEventKind::MlsMembersRequested { group_id: group_id.clone(), cursor: None },
                window,
                cx,
            );
        }
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

    fn ask(&mut self, cursor: Option<String>, window: &mut Window, cx: &mut gpui_kit::App) {
        let Some(group_id) = self.group.clone() else { return };
        self.asked = Some(cursor.clone());
        self.sink.emit(UiEventKind::MlsMembersRequested { group_id, cursor }, window, cx);
    }

    fn build(&self, cx: &gpui_kit::App) -> (Vec<Line>, bool) {
        let Some(group) = self.group.as_deref() else { return (Vec::new(), false) };
        let store = self.store.read(cx).store();
        let Some(pages) = store.mls_members(group) else { return (Vec::new(), false) };
        let query = self.search.read(cx).value().trim().to_lowercase();
        let mut rows: Vec<&MemberRow> = pages.rows.values().collect();
        rows.sort_by(|a, b| {
            let key =
                |r: &MemberRow| (!r.is_me, r.display.clone().unwrap_or_default().to_lowercase());
            key(a).cmp(&key(b))
        });
        let item = |row: &MemberRow| Item {
            anchor: row.anchor.clone(),
            name: member_name(row),
            ai: row.ai_flag || store.mls_is_ai(group, &row.anchor),
            devices: row.devices,
            me: row.is_me,
        };
        let matches = |row: &&MemberRow| {
            query.is_empty()
                || row.display.as_deref().unwrap_or_default().to_lowercase().contains(&query)
        };
        let mut lines = Vec::new();
        for (role, key) in [
            (MlsRole::Owner, "group.members.section.owner"),
            (MlsRole::Admin, "group.members.section.admins"),
        ] {
            let members: Vec<&MemberRow> =
                rows.iter().copied().filter(|r| r.role == role).filter(matches).collect();
            if !members.is_empty() {
                lines.push(Line::Header(tr(key)));
                lines.extend(members.into_iter().map(|r| Line::Member(Box::new(item(r)))));
            }
        }
        let plain: Vec<&MemberRow> = rows
            .iter()
            .copied()
            .filter(|r| !matches!(r.role, MlsRole::Owner | MlsRole::Admin))
            .filter(matches)
            .collect();
        if !plain.is_empty() {
            lines.push(Line::Header(tr("group.members.section.members")));
            lines.extend(plain.into_iter().map(|r| Line::Member(Box::new(item(r)))));
        }
        let pages_remain = pages.cursor.is_some();
        if pages.failed.is_some() {
            lines.push(Line::Error);
        } else if !query.is_empty() && pages_remain {
            let total = store.mls_group(group).map_or(pages.rows.len(), |g| g.members as usize);
            lines.push(Line::Progress(tr_args(
                "group.members.loading_all",
                &[("done", &pages.rows.len().to_string()), ("total", &total.to_string())],
            )));
        }
        (lines, !query.is_empty())
    }
}

impl Render for MlsMembers {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(group) = self.group.clone() else {
            return div().id("mls-members").into_any_element();
        };
        let (lines, searching) = self.build(cx);
        let (cursor, failed, loaded) = {
            let store = self.store.read(cx).store();
            store
                .mls_members(&group)
                .map_or((None, false, false), |p| (p.cursor.clone(), p.failed.is_some(), p.loaded))
        };
        // While a search runs and pages remain, the next page is asked
        // (once per cursor); a failed page waits for the user's tap.
        if searching
            && !failed
            && loaded
            && self.asked.as_ref() != Some(&cursor)
            && let Some(next) = cursor.clone()
        {
            let weak = cx.weak_entity();
            window.defer(cx, move |window, cx| {
                weak.update(cx, |this, cx| this.ask(Some(next), window, cx)).ok();
            });
        }
        let rows_px = window.rem_size() * (ROW_PX / 15.0);
        self.sizes = Rc::new(vec![size(px(0.), rows_px); lines.len()]);
        self.lines = Rc::new(lines);
        let muted = cx.theme().muted_foreground;
        let mentions = self.mode == Mode::Mentions;
        let list_group = group.clone();
        let sizes = self.sizes.clone();
        let list = v_virtual_list(
            cx.entity(),
            "mls-member-rows",
            sizes,
            move |this, range, window, cx| {
                let cursor = this
                    .store
                    .read(cx)
                    .store()
                    .mls_members(&list_group)
                    .and_then(|p| p.cursor.clone());
                if range.end + NEAR_END >= this.lines.len()
                    && let Some(next) = cursor
                    && this.asked.as_ref() != Some(&Some(next.clone()))
                {
                    this.asked = Some(Some(next.clone()));
                    let weak = cx.weak_entity();
                    window.defer(cx, move |window, cx| {
                        weak.update(cx, |this, cx| this.ask(Some(next), window, cx)).ok();
                    });
                }
                let muted = cx.theme().muted_foreground;
                range
                    .filter_map(|ix| {
                        let line = this.lines.get(ix)?.clone();
                        Some(match line {
                            Line::Header(text) => div()
                                .h(gpui_kit::rems(ROW_PX / 15.0))
                                .flex()
                                .items_end()
                                .pb(space(1.))
                                .text_size(text_caption())
                                .text_color(muted)
                                .child(text)
                                .into_any_element(),
                            Line::Progress(text) => div()
                                .id("mls-members-progress")
                                .role(Role::Status)
                                .h(gpui_kit::rems(ROW_PX / 15.0))
                                .flex()
                                .items_center()
                                .text_size(text_caption())
                                .text_color(muted)
                                .child(text)
                                .into_any_element(),
                            Line::Error => h_flex()
                                .id("mls-members-error")
                                .role(Role::Alert)
                                .h(gpui_kit::rems(ROW_PX / 15.0))
                                .gap(space(2.))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(text_secondary())
                                        .child(tr("group.members.page_error")),
                                )
                                .child(
                                    Button::new("mls-members-retry")
                                        .small()
                                        .label(tr("group.list.retry"))
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            let cursor = this
                                                .group
                                                .as_deref()
                                                .and_then(|g| {
                                                    this.store.read(cx).store().mls_members(g)
                                                })
                                                .and_then(|p| p.cursor.clone());
                                            this.ask(cursor, window, cx);
                                        })),
                                )
                                .into_any_element(),
                            Line::Member(item) => {
                                let anchor = item.anchor.clone();
                                let devices =
                                    tr_count("group.members.devices", u64::from(item.devices));
                                let ai = item.ai;
                                let selected = this.mode == Mode::Mentions
                                    && picked_mentions(this.group.as_deref().unwrap_or_default())
                                        .iter()
                                        .any(|(a, _)| *a == anchor);
                                let label = item.name.clone();
                                let pick_name = item.name.clone();
                                let me = item.me;
                                let content = h_flex()
                                    .h(gpui_kit::rems(ROW_PX / 15.0))
                                    .gap(space(2.))
                                    .px(space(2.))
                                    .child(div().text_size(text_secondary()).child(label.clone()))
                                    .children(
                                        ai.then(|| crate::identity::ai_mark("mls-member-ai", cx)),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        div()
                                            .text_size(text_caption())
                                            .text_color(muted)
                                            .child(devices),
                                    );
                                if this.mode == Mode::Mentions && !me {
                                    Button::new(SharedString::from(format!("mls-member-{anchor}")))
                                        .ghost()
                                        .w_full()
                                        .selected(selected)
                                        .accessibility_label(label)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(group) = this.group.clone() {
                                                toggle_mention(
                                                    &group,
                                                    &anchor,
                                                    MentionPick { name: pick_name.clone() },
                                                );
                                            }
                                            cx.notify();
                                        }))
                                        .child(content)
                                        .into_any_element()
                                } else {
                                    div()
                                        .id(SharedString::from(format!("mls-member-{anchor}")))
                                        .test_support()
                                        .role(Role::ListItem)
                                        .child(content)
                                        .into_any_element()
                                }
                            }
                        })
                    })
                    .collect()
            },
        )
        .track_scroll(&self.scroll);
        let empty = self.lines.is_empty() && loaded;
        let this = cx.weak_entity();
        let body = v_flex()
            .id("mls-members-body")
            .test_support()
            .gap(space(3.))
            .child(crate::workspace::sections::directory::search_input(
                "mls-members-search",
                &self.search,
                tr("group.members.search"),
                None,
                cx,
            ))
            .child(div().h(gpui_kit::rems(360.0 / 15.0)).min_h_0().child(list))
            .children(empty.then(|| {
                div()
                    .id("mls-members-empty")
                    .role(Role::Status)
                    .text_size(text_secondary())
                    .text_color(muted)
                    .child(tr("group.members.no_results"))
            }));
        let _ = Input::new;
        let title = tr(if mentions { "group.members.mention" } else { "group.members.title" });
        crate::modal::Modal::new("mls-members", title, &self.focus, move |_, cx| {
            this.update(cx, |panel, cx| panel.close(cx)).ok();
        })
        .child(body)
        .render(cx)
    }
}
