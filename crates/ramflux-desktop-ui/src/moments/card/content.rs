// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Content nodes: Text (plain), Icon, Image (never loaded), Chart with its
//! table alternative, Table and Task. Values are untrusted and drawn as text.

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, IntoElement, div, px};
use ramflux_desktop_core::{CardNode, JsonValue};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};

use super::{View, attr_bool, attr_f64, attr_str};
use crate::i18n::{tr, tr_args};

/// A Text node: its `text`, plain. Empty text draws nothing.
pub fn text(view: &View<'_>, node: &CardNode, cx: &App) -> AnyElement {
    let _ = view;
    let Some(text) = attr_str(node, "text").filter(|t| !t.is_empty()) else {
        return div().into_any_element();
    };
    let caption = attr_str(node, "variant") == Some("caption");
    div()
        .w_full()
        .text_size(if caption { text_caption() } else { text_secondary() })
        .text_color(if caption { cx.theme().muted_foreground } else { cx.theme().foreground })
        .child(text.to_owned())
        .into_any_element()
}

/// An Icon by its name (the platform set's common names); unknown names draw
/// the generic info glyph, with the node's label as its accessible name.
pub fn icon(node: &CardNode, cx: &App) -> AnyElement {
    let name = match attr_str(node, "name").unwrap_or_default() {
        "check" => IconName::Check,
        "close" | "x" => IconName::Close,
        "warning" | "alert" => IconName::TriangleAlert,
        "error" => IconName::CircleX,
        "success" => IconName::CircleCheck,
        "bot" | "agent" => IconName::Bot,
        "user" | "person" => IconName::User,
        "plus" | "add" => IconName::Plus,
        _ => IconName::Info,
    };
    Icon::new(name).small().text_color(cx.theme().muted_foreground).into_any_element()
}

/// An Image: the spec's placeholder box with the description as its name.
/// Byte delivery does not exist in this build, so the image is never
/// fetched and its URL never read.
pub fn image(node: &CardNode, cx: &App) -> AnyElement {
    let description = attr_str(node, "description").unwrap_or_default().to_owned();
    v_flex()
        .w(px(160.))
        .h(px(96.))
        .items_center()
        .justify_center()
        .gap(space(1.))
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted)
        .text_size(text_caption())
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(IconName::Info).small())
        .child(tr("moment.card.image_unavailable"))
        .when(!description.is_empty(), |col| col.child(description))
        .into_any_element()
}

/// A Chart: one bar per point, scaled to the largest value, with the values
/// also listed as text (the table alternative).
pub fn chart(node: &CardNode, cx: &App) -> AnyElement {
    let series: Vec<(String, Vec<(String, f64)>)> = node
        .attrs
        .get("series")
        .and_then(JsonValue::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    let label = row.get("label").and_then(JsonValue::as_str).unwrap_or_default();
                    let points = row
                        .get("points")
                        .and_then(JsonValue::as_array)
                        .map(|ps| {
                            ps.iter()
                                .filter_map(|p| {
                                    let name = p.get("label")?.as_str()?.to_owned();
                                    let value = p.get("value")?.as_f64()?;
                                    Some((name, value))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    (label.to_owned(), points)
                })
                .collect()
        })
        .unwrap_or_default();
    let peak = series
        .iter()
        .flat_map(|(_, points)| points.iter().map(|(_, v)| v.abs()))
        .fold(0.0_f64, f64::max);
    let scale = attr_f64(node, "scale").unwrap_or(1.0);
    let bars = series.iter().map(|(label, points)| {
        let columns = points.iter().map(|(name, value)| {
            let height = if peak > 0.0 { (value.abs() / peak * 96.0).max(2.0) } else { 2.0 };
            v_flex()
                .flex_1()
                .items_center()
                .gap(space(1.))
                .child(
                    div()
                        .mt_auto()
                        .w_full()
                        .h(px(height as f32))
                        .rounded(cx.theme().radius)
                        .bg(cx.theme().primary),
                )
                .child(div().text_size(text_caption()).child(name.clone()))
        });
        v_flex()
            .w_full()
            .gap(space(1.))
            .child(
                div()
                    .text_size(text_caption())
                    .text_color(cx.theme().muted_foreground)
                    .child(label.clone()),
            )
            .child(h_flex().w_full().h(px(112.)).items_end().gap(space(1.)).children(columns))
    });
    let values = series.iter().flat_map(|(label, points)| {
        points.iter().map(move |(name, value)| {
            tr_args(
                "moment.card.chart_value",
                &[("series", label), ("point", name), ("value", &(value * scale).to_string())],
            )
            .to_string()
        })
    });
    v_flex()
        .w_full()
        .gap(space(2.))
        .children(bars)
        .child(
            v_flex()
                .gap(space(0.5))
                .text_size(text_caption())
                .text_color(cx.theme().muted_foreground)
                .children(values.map(|v| div().child(v))),
        )
        .into_any_element()
}

/// A Table: a header row of column labels and one row per table row.
pub fn table(node: &CardNode, cx: &App) -> AnyElement {
    let columns: Vec<(String, String)> = node
        .attrs
        .get("columns")
        .and_then(JsonValue::as_array)
        .map(|cols| {
            cols.iter()
                .map(|c| {
                    let key = c.get("key").and_then(JsonValue::as_str).unwrap_or_default();
                    let label = c.get("label").and_then(JsonValue::as_str).unwrap_or(key);
                    (key.to_owned(), label.to_owned())
                })
                .collect()
        })
        .unwrap_or_default();
    let rows: Vec<Vec<String>> = node
        .attrs
        .get("rows")
        .and_then(JsonValue::as_array)
        .map(|rs| {
            rs.iter()
                .map(|r| {
                    r.get("cells")
                        .and_then(JsonValue::as_array)
                        .map(|cells| {
                            cells
                                .iter()
                                .map(|c| c.as_str().map_or_else(|| c.to_string(), str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default();
    let header = h_flex().w_full().gap(space(2.)).children(columns.iter().map(|(_, label)| {
        div().flex_1().font_semibold().text_size(text_caption()).child(label.clone())
    }));
    let body = rows.into_iter().map(|cells| {
        h_flex().w_full().gap(space(2.)).children(
            cells.into_iter().map(|cell| div().flex_1().text_size(text_caption()).child(cell)),
        )
    });
    v_flex()
        .w_full()
        .gap(space(1.))
        .border_1()
        .border_color(cx.theme().border)
        .rounded(cx.theme().radius)
        .p(space(2.))
        .child(header)
        .children(body)
        .into_any_element()
}

/// A Task: its owner, requester, status, progress and steps (plain text).
pub fn task(node: &CardNode, cx: &App) -> AnyElement {
    let owner = attr_str(node, "owner").unwrap_or_default().to_owned();
    let requester = attr_str(node, "requester").unwrap_or_default().to_owned();
    let status = attr_str(node, "status").unwrap_or_default().to_owned();
    let progress = attr_f64(node, "progress").unwrap_or(0.0).clamp(0.0, 100.0);
    let steps: Vec<(String, String)> = node
        .attrs
        .get("steps")
        .and_then(JsonValue::as_array)
        .map(|rows| {
            rows.iter()
                .map(|s| {
                    let text = s.get("text").and_then(JsonValue::as_str).unwrap_or_default();
                    let status = s.get("status").and_then(JsonValue::as_str).unwrap_or_default();
                    (text.to_owned(), status.to_owned())
                })
                .collect()
        })
        .unwrap_or_default();
    let _ = attr_bool(node, "disabled");
    v_flex()
        .w_full()
        .gap(space(2.))
        .child(
            div().text_size(text_caption()).text_color(cx.theme().muted_foreground).child(tr_args(
                "moment.card.task_by",
                &[("requester", &requester), ("owner", &owner)],
            )),
        )
        .child(div().text_size(text_secondary()).font_semibold().child(status))
        .child(div().w_full().h(px(6.)).rounded(cx.theme().radius).bg(cx.theme().muted).child(
            div().h_full().w(gpui_kit::relative(progress as f32 / 100.0)).bg(cx.theme().primary),
        ))
        .children(steps.into_iter().map(|(text, status)| {
            h_flex()
                .gap(space(2.))
                .text_size(text_caption())
                .child(div().child(text))
                .child(div().text_color(cx.theme().muted_foreground).child(status))
        }))
        .into_any_element()
}
