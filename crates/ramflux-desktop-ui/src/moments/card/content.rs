// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Content nodes: Text (plain), Icon, Image (never loaded), Chart with its
//! table alternative, Table and Task. Values are untrusted and drawn as text.

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, IntoElement, SharedString, Window, div, px};
use ramflux_desktop_core::{CardImage, CardNode, JsonValue};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};

use super::images::{self, Decoded};
use super::{View, attr_bool, attr_f64, attr_str, attr_text, shown};
use crate::i18n::{tr, tr_args};
use crate::ids::domain_id;

/// A Text node: its `text`, plain. Empty text draws nothing.
pub fn text(view: &View<'_>, node: &CardNode, cx: &App) -> AnyElement {
    let _ = view;
    let text = attr_text(node, "text");
    if text.is_empty() {
        return div().into_any_element();
    }
    let caption = attr_str(node, "variant") == Some("caption");
    div()
        .w_full()
        .text_size(if caption { text_caption() } else { text_secondary() })
        .text_color(if caption { cx.theme().muted_foreground } else { cx.theme().foreground })
        .child(text)
        .into_any_element()
}

/// An Icon by its name (the platform set's common names); unknown names draw
/// the generic info glyph, with the node's label as its accessible name. No
/// name maps to a glyph the host frame or a security notice uses (the shield,
/// the warning triangle), so a card cannot imitate them.
pub fn icon(node: &CardNode, cx: &App) -> AnyElement {
    let name = match attr_str(node, "name").unwrap_or_default() {
        "check" => IconName::Check,
        "close" | "x" => IconName::Close,
        "error" => IconName::CircleX,
        "success" => IconName::CircleCheck,
        "bot" | "agent" => IconName::Bot,
        "user" | "person" => IconName::User,
        "plus" | "add" => IconName::Plus,
        _ => IconName::Info,
    };
    Icon::new(name).small().text_color(cx.theme().muted_foreground).into_any_element()
}

/// An Image (spec 4.15): the box of its variant with the picture the host
/// read through the object API, drawn with `fit`; while the bytes arrive, or
/// when there are none to show, the placeholder box ("Image unavailable" on
/// failure). `description` is the accessible name and the tooltip; a click
/// opens the full view. Nothing is read from the node but its own
/// attributes: no URL, path or data URI.
pub fn image(view: &View<'_>, node: &CardNode, window: &mut Window, cx: &mut App) -> AnyElement {
    let description = attr_text(node, "description");
    let shown = shown_of(view, node, window, cx);
    let (frame, round) = frame(node, cx);
    let frame = frame.id(domain_id("card-image", &node.key)).aria_label(description.clone());
    let frame = if description.is_empty() {
        frame
    } else {
        let tip = SharedString::from(description.clone());
        frame.tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
    };
    match shown {
        Shown::Picture(image) => {
            let (local, key) = (view.local.clone(), node.key.clone());
            frame
                .cursor_pointer()
                .child(
                    gpui_kit::img(image.clone())
                        .size_full()
                        .when(round, |picture| picture.rounded_full())
                        .object_fit(fit(node)),
                )
                .on_click(move |_, window, _| {
                    local.borrow_mut().image = Some(key.clone());
                    window.refresh();
                })
                .into_any_element()
        }
        Shown::Waiting => frame
            .bg(cx.theme().muted)
            .child(Icon::new(IconName::Info).small().text_color(cx.theme().muted_foreground))
            .into_any_element(),
        Shown::Unavailable => frame
            .bg(cx.theme().muted)
            .flex_col()
            .gap(space(1.))
            .text_size(text_caption())
            .text_color(cx.theme().muted_foreground)
            .child(Icon::new(IconName::Info).small())
            .child(tr("moment.card.image_unavailable"))
            .into_any_element(),
    }
}

/// The full view of an image (spec 4.15): a dialog over a scrim with the
/// picture fitted with `contain` and its description below. Escape, the close
/// button or a click outside it closes the view.
pub fn image_sheet(view: &View<'_>, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let key = view.local.borrow().image.clone()?;
    let node = view.node(&key)?;
    let description = attr_text(node, "description");
    let Shown::Picture(image) = shown_of(view, node, window, cx) else {
        // The picture is gone (released or not decodable): nothing to show.
        view.local.borrow_mut().image = None;
        return None;
    };
    let focus = window
        .use_keyed_state(domain_id("card-image-view", &key), cx, |_, cx| cx.focus_handle())
        .read(cx)
        .clone();
    let (first, local) = {
        let mut local = view.local.borrow_mut();
        let first = !local.image_focused;
        local.image_focused = true;
        (first, view.local.clone())
    };
    if first {
        let focus = focus.clone();
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
    }
    let close = {
        let local = local.clone();
        move |window: &mut Window| {
            let mut local = local.borrow_mut();
            local.image = None;
            local.image_focused = false;
            window.refresh();
        }
    };
    let (escape, outside, button) = (close.clone(), close.clone(), close);
    Some(
        div()
            .id(domain_id("card-image-sheet", &key))
            .absolute()
            .inset_0()
            .bg(cx.theme().overlay)
            .flex()
            .items_center()
            .justify_center()
            .track_focus(&focus)
            .on_key_down(move |event, window, _| {
                if event.keystroke.key == "escape" {
                    escape(window);
                }
            })
            .on_mouse_down(gpui_kit::MouseButton::Left, move |_, window, _| outside(window))
            .child(
                v_flex()
                    .w(px(480.))
                    .max_w_full()
                    .max_h_full()
                    .gap(space(3.))
                    .p(space(4.))
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        // The close button sits at the inline end (a spacer
                        // first, so the row mirrors under a right-to-left locale).
                        h_flex().child(div().flex_1()).child(
                            gpui_kit::component::button::Button::new(domain_id(
                                "card-image-close",
                                &key,
                            ))
                            .small()
                            .label(tr("moment.card.modal_close"))
                            .on_click(move |_, window, _| button(window)),
                        ),
                    )
                    .child(div().w_full().h(px(320.)).child(
                        gpui_kit::img(image).size_full().object_fit(gpui_kit::ObjectFit::Contain),
                    ))
                    .when(!description.is_empty(), |col| {
                        col.child(
                            div()
                                .text_size(text_secondary())
                                .text_color(cx.theme().muted_foreground)
                                .child(description),
                        )
                    }),
            )
            .into_any_element(),
    )
}

/// What `node` draws now (see [`Shown`]): its slot's object, where the bridge
/// has its bytes, and whether they are decoded.
fn shown_of(view: &View<'_>, node: &CardNode, window: &mut Window, cx: &mut App) -> Shown {
    let object = view.tree.host_slots().iter().find_map(|slot| {
        let image = slot.request.get("kind").and_then(JsonValue::as_str) == Some("image")
            && slot.key == node.key;
        let attachment = slot.request.get("attachment")?;
        (image && attachment.get("retrieval").and_then(JsonValue::as_str) == Some("object"))
            .then(|| attachment.get("object_id").and_then(JsonValue::as_str))
            .flatten()
    });
    // A slot with no object selector never has a picture.
    match object {
        None => Shown::Unavailable,
        Some(object) => match view.images.get(object) {
            None | Some(CardImage::Loading) => Shown::Waiting,
            Some(CardImage::Unavailable) => Shown::Unavailable,
            Some(CardImage::Ready { path, width, height }) => {
                match images::lookup(path, *width, *height, window, cx) {
                    Decoded::Ready(image) => Shown::Picture(image),
                    Decoded::Pending => Shown::Waiting,
                    Decoded::Failed => Shown::Unavailable,
                }
            }
        },
    }
}

/// What an Image node draws.
enum Shown {
    /// The decoded picture.
    Picture(std::sync::Arc<gpui_kit::RenderImage>),
    /// The bytes are on their way or being decoded.
    Waiting,
    /// "Image unavailable".
    Unavailable,
}

/// The box of an image variant (spec 4.15) and whether it is a circle.
fn frame(node: &CardNode, cx: &App) -> (gpui_kit::Div, bool) {
    let radius = cx.theme().radius;
    let base = div().flex().items_center().justify_center().overflow_hidden();
    match attr_str(node, "variant").unwrap_or("mediumFeature") {
        "icon" => (base.size(px(24.)).rounded(px(4.)), false),
        "avatar" => (base.size(px(40.)).rounded_full(), true),
        "smallFeature" => (base.size(px(96.)).rounded(radius), false),
        "largeFeature" => {
            (base.w_full().max_h(px(420.)).aspect_ratio(4.0 / 3.0).rounded(radius), false)
        }
        "header" => (base.w_full().max_h(px(240.)).aspect_ratio(2.0).rounded(radius), false),
        _ => (base.w_full().max_h(px(280.)).aspect_ratio(16.0 / 9.0).rounded(radius), false),
    }
}

/// The node's `fit` (default `fill`).
fn fit(node: &CardNode) -> gpui_kit::ObjectFit {
    match attr_str(node, "fit") {
        Some("contain") => gpui_kit::ObjectFit::Contain,
        Some("cover") => gpui_kit::ObjectFit::Cover,
        Some("scaleDown" | "scale-down") => gpui_kit::ObjectFit::ScaleDown,
        Some("none") => gpui_kit::ObjectFit::None,
        _ => gpui_kit::ObjectFit::Fill,
    }
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
                    let label =
                        shown(row.get("label").and_then(JsonValue::as_str).unwrap_or_default());
                    let points = row
                        .get("points")
                        .and_then(JsonValue::as_array)
                        .map(|ps| {
                            ps.iter()
                                .filter_map(|p| {
                                    let name = shown(p.get("label")?.as_str()?);
                                    let value = p.get("value")?.as_f64()?;
                                    Some((name, value))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    (label, points)
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
                    (key.to_owned(), shown(label))
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
                                .map(|c| {
                                    shown(&c.as_str().map_or_else(|| c.to_string(), str::to_owned))
                                })
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
    let owner = attr_text(node, "owner");
    let requester = attr_text(node, "requester");
    let status = attr_text(node, "status");
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
                    (shown(text), shown(status))
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
