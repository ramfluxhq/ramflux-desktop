// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The parts every section directory shares: the retained search field and
//! keyboard cursor, the rows drawn with the generated RF list row, and the
//! loading, offline, failed and empty states drawn with the generated state
//! block.
//!
//! Rows carry stable domain keys (participant, context or request ids),
//! never positions. Search is local over what the store already holds and
//! is bounded; it is not a server search.

use std::rc::Rc;

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, ElementId, Entity, Focusable as _, Role, SharedString, Window, div,
};
use ramflux_desktop_core::{ListState, SourceStatus};
use ramflux_desktop_navigation::generated::catalog;
use ramflux_desktop_navigation::palette;

use crate::i18n::tr;

/// Longest query compared, in characters (longer input is cut).
pub(crate) const QUERY_LIMIT: usize = 256;

/// Most rows drawn at once; a longer result asks to refine the search.
pub(crate) const ROW_LIMIT: usize = 500;

/// One directory row.
pub(crate) struct DirectoryRow {
    /// Stable domain key (also the generated row's id).
    pub key: SharedString,
    /// Display title (already cleaned for a single line).
    pub title: SharedString,
    /// Second line.
    pub subtitle: SharedString,
    /// Avatar shape: a source kind fact, never a trust claim.
    pub avatar: catalog::AvatarKind,
    /// Short trailing text (a role, a time, a count).
    pub trailing: Option<SharedString>,
    /// Shown in the detail pane.
    pub selected: bool,
    /// Batch selection: `Some(checked)` draws a checkbox.
    pub checked: Option<bool>,
}

/// The retained search field and keyboard cursor of a directory.
pub(crate) struct Search {
    input: Entity<InputState>,
    /// Row the keyboard cursor is on.
    pub active: usize,
}

impl Search {
    /// A search field with the translated placeholder `key`.
    pub fn new<T: 'static>(key: &'static str, window: &mut Window, cx: &mut Context<T>) -> Self {
        let cx: &mut App = cx;
        let input = crate::i18n::localized_input(key, window, cx, |input| input);
        Self { input, active: 0 }
    }

    /// The input entity (subscribe to its events in the owner).
    pub fn input(&self) -> &Entity<InputState> {
        &self.input
    }

    /// The typed query, trimmed, lowercased and bounded.
    pub fn query(&self, cx: &App) -> String {
        self.input
            .read(cx)
            .value()
            .trim()
            .chars()
            .take(QUERY_LIMIT)
            .collect::<String>()
            .to_lowercase()
    }

    /// Moves the cursor by `step` within `len` rows (wrapping).
    pub fn step(&mut self, step: isize, len: usize) {
        if len == 0 {
            self.active = 0;
            return;
        }
        let len_i = isize::try_from(len).unwrap_or(isize::MAX);
        let current = isize::try_from(self.active.min(len - 1)).unwrap_or(0);
        self.active = usize::try_from((current + step).rem_euclid(len_i)).unwrap_or(0);
    }

    /// Focuses the field.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        let focus = self.input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    }

    /// The field element.
    pub fn field(&self, id: &'static str, label: SharedString) -> AnyElement {
        div()
            .id(id)
            .test_support()
            .px_3()
            .pb_2()
            .child(Input::new(&self.input).small().aria_label(label).cleanable(true))
            .into_any_element()
    }
}

/// Whether any of `fields` contains `query` (already lowercased).
pub(crate) fn matches(query: &str, fields: &[&str]) -> bool {
    query.is_empty() || fields.iter().any(|field| field.to_lowercase().contains(query))
}

/// The section header: title, count and trailing controls.
pub(crate) fn header(title: SharedString, trailing: impl IntoElement, cx: &App) -> AnyElement {
    div()
        .px_1()
        .pt_2()
        .child(catalog::section_header(title, trailing, palette(cx)))
        .into_any_element()
}

/// A row callback: receives the row key.
pub(crate) type RowAction = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// What a directory list draws.
pub(crate) struct RowList {
    /// Element id of the list.
    pub id: &'static str,
    /// Accessible name of the list.
    pub label: SharedString,
    /// The rows in display order.
    pub rows: Vec<DirectoryRow>,
    /// Row under the keyboard cursor.
    pub active: usize,
    /// Called with the key of a chosen row.
    pub on_pick: RowAction,
    /// Called with the key of a toggled checkbox (batch selection).
    pub on_check: Option<RowAction>,
}

/// Rows drawn with the generated list row.
pub(crate) fn rows(list: RowList, cx: &App) -> AnyElement {
    let RowList { id, label, rows, active, on_pick, on_check } = list;
    let theme = palette(cx);
    let total = rows.len();
    let selected_bg = cx.theme().list_active;
    let active_border = cx.theme().ring;
    v_flex()
        .id(id)
        .test_support()
        .role(Role::List)
        .aria_label(label)
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px_2()
        .gap_1()
        .children(rows.into_iter().take(ROW_LIMIT).enumerate().map(|(ix, row)| {
            let pick = on_pick.clone();
            let check = on_check.clone();
            let key = row.key.clone();
            let mut content =
                catalog::RowContent::new(row.key.clone(), row.title.clone(), row.subtitle).leading(
                    catalog::avatar(
                        ramflux_desktop_navigation::row::initial(&row.title),
                        row.avatar,
                        theme,
                    ),
                );
            if let Some(trailing) = row.trailing {
                content = content.trailing(
                    div().text_xs().text_color(cx.theme().muted_foreground).child(trailing),
                );
            }
            h_flex()
                .id(ElementId::from(format!("{id}-row-{}", row.key)))
                .test_support()
                .role(Role::ListItem)
                .aria_selected(row.selected)
                .gap_1()
                .min_w_0()
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(if ix == active {
                    active_border.opacity(0.5)
                } else {
                    cx.theme().transparent
                })
                .when(row.selected, |item| item.bg(selected_bg))
                .when_some(row.checked.zip(check), |item, (checked, check)| {
                    let key = key.clone();
                    item.child(
                        Checkbox::new(ElementId::from(format!("{id}-check-{key}")))
                            .checked(checked)
                            .accessibility_label(row.title.clone())
                            .on_click(move |_, window, cx| check(&key, window, cx)),
                    )
                })
                .child(div().flex_1().min_w_0().child(catalog::list_row(
                    content,
                    false,
                    theme,
                    move |key, window, cx| pick(key, window, cx),
                )))
        }))
        .when(total > ROW_LIMIT, |list| {
            list.child(
                div()
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("sections.refine")),
            )
        })
        .into_any_element()
}

/// The state block of a directory with no rows to show, or `None` when it
/// has rows. Failure wins over offline, offline over loading or empty.
/// What decides a directory's state block.
pub(crate) struct StateSpec {
    /// Element id of the block.
    pub id: &'static str,
    /// Whether the collection is loading, empty or has rows.
    pub list: ListState,
    /// The source connection.
    pub status: SourceStatus,
    /// The current read failure, translated.
    pub failure: Option<SharedString>,
    /// Locale keys of the four states.
    pub keys: StateKeys,
    /// An optional action under the text.
    pub action: Option<AnyElement>,
}

pub(crate) fn state(spec: StateSpec, cx: &App) -> Option<AnyElement> {
    let StateSpec { id, list, status, failure, keys, action } = spec;
    if list == ListState::Ready {
        return None;
    }
    let (title, description, kind) = if let Some(reason) = failure {
        (tr(keys.failed), reason, catalog::StateKind::Error)
    } else if status == SourceStatus::Disconnected {
        (tr(keys.offline), tr("sections.offline_help"), catalog::StateKind::Offline)
    } else if list == ListState::Loading {
        (tr(keys.loading), tr("sections.loading_help"), catalog::StateKind::Loading)
    } else {
        (tr(keys.empty), tr(keys.empty_help), catalog::StateKind::Empty)
    };
    Some(
        div()
            .id(id)
            .test_support()
            .role(if kind == catalog::StateKind::Error { Role::Alert } else { Role::Status })
            .aria_label(format!("{title}. {description}"))
            .p_3()
            .whitespace_normal()
            .child(catalog::state_block(
                SharedString::from(id),
                title,
                description,
                kind,
                action.unwrap_or_else(|| div().into_any_element()),
                palette(cx),
            ))
            .into_any_element(),
    )
}

/// Locale keys of a directory's four states.
#[derive(Clone, Copy)]
pub(crate) struct StateKeys {
    pub loading: &'static str,
    pub offline: &'static str,
    pub failed: &'static str,
    pub empty: &'static str,
    pub empty_help: &'static str,
}

/// A detail pane's empty state ("choose one on the left").
pub(crate) fn placeholder(
    id: &'static str,
    title: SharedString,
    text: SharedString,
    cx: &App,
) -> AnyElement {
    v_flex()
        .id(id)
        .test_support()
        .role(Role::Status)
        .aria_label(format!("{title}. {text}"))
        .size_full()
        .items_center()
        .justify_center()
        .p_6()
        .child(catalog::state_block(
            SharedString::from(id),
            title,
            text,
            catalog::StateKind::Empty,
            div(),
            palette(cx),
        ))
        .into_any_element()
}

/// Why a part the backend has no contract for yet is unavailable: the
/// typed gap, translated. The control stays visible and disabled.
pub(crate) fn unavailable(gap: ramflux_desktop_sdk::business::ContractGap) -> SharedString {
    use ramflux_desktop_sdk::business::ContractGap;
    tr(match gap {
        ContractGap::StrangerActions => "sections.unavailable.stranger_actions",
        ContractGap::AgentConfiguration => "sections.unavailable.agent_configuration",
        ContractGap::AccountPrivacy => "settings_ui.privacy_unavailable",
        ContractGap::DevicePairingApproval => "settings_ui.pairing_unavailable",
        ContractGap::WebDeviceFacts => "settings_ui.source_unavailable",
    })
}
