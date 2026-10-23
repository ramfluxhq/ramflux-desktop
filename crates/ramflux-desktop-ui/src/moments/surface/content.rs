// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Read-only content blocks: text, markdown, heading, key/value, metric,
//! list, table, file preview, and the fallback block.
//!
//! Each renderer returns `None` when the (untrusted) data lacks what it
//! needs; the caller then shows the fallback block.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, IntoElement, SharedString, div};
use ramflux_desktop_core::{BlockId, JsonValue};

use crate::i18n::{self, tr};
use crate::ids::domain_id;

fn str_field<'a>(data: &'a JsonValue, key: &str) -> Option<&'a str> {
    data.get(key).and_then(JsonValue::as_str)
}

fn strings(value: &JsonValue) -> Option<Vec<String>> {
    value.as_array().map(|items| {
        items.iter().map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned)).collect()
    })
}

/// Plain text.
pub fn text(data: &JsonValue, cx: &App) -> Option<AnyElement> {
    let text = str_field(data, "text")?.to_owned();
    Some(div().text_sm().text_color(cx.theme().foreground).child(text).into_any_element())
}

/// Markdown rendered without scripts. A streaming block re-renders as text
/// is appended.
pub fn markdown(id: &BlockId, data: &JsonValue, cx: &App) -> Option<AnyElement> {
    let source = str_field(data, "text")?.to_owned();
    Some(
        div()
            .text_sm()
            .text_color(cx.theme().foreground)
            .child(TextView::markdown(domain_id("markdown", id), source))
            .into_any_element(),
    )
}

/// A heading.
pub fn heading(data: &JsonValue, cx: &App) -> Option<AnyElement> {
    let text = str_field(data, "text")?.to_owned();
    Some(
        div()
            .text_base()
            .font_semibold()
            .text_color(cx.theme().foreground)
            .child(text)
            .into_any_element(),
    )
}

/// Key/value pairs from `items: [[key, value], ...]`.
pub fn key_value(data: &JsonValue) -> Option<AnyElement> {
    let items = data.get("items")?.as_array()?;
    let mut list = DescriptionList::new().columns(1);
    for pair in items {
        let pair = strings(pair)?;
        let [key, value] = pair.as_slice() else { return None };
        list = list.item(SharedString::from(key.clone()), SharedString::from(value.clone()), 1);
    }
    Some(list.into_any_element())
}

/// One metric with an optional change.
pub fn metric(data: &JsonValue, cx: &App) -> Option<AnyElement> {
    let label = str_field(data, "label")?.to_owned();
    let value = str_field(data, "value")?.to_owned();
    let delta = str_field(data, "delta").map(str::to_owned);
    Some(
        v_flex()
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label))
            .child(
                h_flex()
                    .gap_2()
                    .items_baseline()
                    .child(div().text_lg().font_semibold().child(value))
                    .when_some(delta, |row, delta| {
                        row.child(
                            div().text_xs().text_color(cx.theme().muted_foreground).child(delta),
                        )
                    }),
            )
            .into_any_element(),
    )
}

/// A bulleted list from `items`.
pub fn list(data: &JsonValue, cx: &App) -> Option<AnyElement> {
    let items = strings(data.get("items")?)?;
    Some(
        v_flex()
            .gap_1()
            .text_sm()
            .children(items.into_iter().map(|item| {
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(div().text_color(cx.theme().muted_foreground).child("•"))
                    .child(div().min_w_0().child(item))
            }))
            .into_any_element(),
    )
}

/// A small table from `columns` and `rows`, rendered as a plain grid.
pub fn table(data: &JsonValue, cx: &App) -> Option<AnyElement> {
    let columns = strings(data.get("columns")?)?;
    let rows: Vec<Vec<String>> =
        data.get("rows")?.as_array()?.iter().map(strings).collect::<Option<_>>()?;
    let width = columns.len();
    if width == 0 || rows.iter().any(|r| r.len() != width) {
        return None;
    }
    let cell = |text: String, header: bool| {
        div()
            .flex_1()
            .min_w_0()
            .px_2()
            .py_1()
            .truncate()
            .when(header, |c| c.font_medium().text_color(cx.theme().muted_foreground))
            .child(text)
    };
    let header = h_flex()
        .w_full()
        .border_b_1()
        .border_color(cx.theme().border)
        .children(columns.into_iter().map(|c| cell(c, true)));
    Some(
        v_flex()
            .w_full()
            .text_sm()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .child(header)
            .children(
                rows.into_iter()
                    .map(|row| h_flex().w_full().children(row.into_iter().map(|c| cell(c, false)))),
            )
            .into_any_element(),
    )
}

/// A file preview card: name, size and an excerpt supplied by the agent.
pub fn file_preview(data: &JsonValue, cx: &App) -> Option<AnyElement> {
    let name = str_field(data, "name")?.to_owned();
    let size = data.get("size").and_then(JsonValue::as_u64).map(i18n::bytes);
    let excerpt = str_field(data, "excerpt").map(str::to_owned);
    Some(
        h_flex()
            .gap_3()
            .items_start()
            .p_2()
            .border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .child(Icon::new(IconName::FileText).text_color(cx.theme().muted_foreground))
            .child(
                v_flex()
                    .min_w_0()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().text_sm().font_medium().child(name))
                            .when_some(size, |row, size| {
                                row.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(size),
                                )
                            }),
                    )
                    .when_some(excerpt, |col, excerpt| {
                        col.child(
                            div().text_xs().text_color(cx.theme().muted_foreground).child(excerpt),
                        )
                    }),
            )
            .into_any_element(),
    )
}

/// The fallback block: the surface's fallback text, marked as a downgrade.
pub fn fallback(text: &SharedString, cx: &App) -> AnyElement {
    v_flex()
        .id("block-fallback")
        .gap_1()
        .p_2()
        .border_1()
        .border_dashed()
        .border_color(cx.theme().border)
        .rounded(cx.theme().radius)
        .child(
            h_flex()
                .gap_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::EyeOff).xsmall())
                .child(tr("surface.fallback")),
        )
        .child(div().text_sm().text_color(cx.theme().foreground).child(text.clone()))
        .test_support()
        .into_any_element()
}
