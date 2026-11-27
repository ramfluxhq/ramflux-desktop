// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Control nodes: Button, TextField, CheckBox, ChoicePicker, Slider and
//! DateTimeInput. Each change is sent to the runtime as a host event on the
//! node's key. A text draft is sent with the press of a Button that follows
//! it (the press is the submit's trigger), so the runtime sees what the user
//! typed before it acts.

use gpui_kit::component::button::Button;
use gpui_kit::component::input::Input;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Entity, IntoElement, Window, div, px};
use ramflux_desktop_core::{CardInput, CardNode, JsonValue};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption};

use super::{View, attr_bool, attr_f64, attr_str, errors};
use crate::ids::domain_id;
use crate::style::DesignButton as _;

/// The label a control shows: its own `label`, as text.
fn label_of(node: &CardNode, cx: &App) -> AnyElement {
    div()
        .text_size(text_caption())
        .text_color(cx.theme().muted_foreground)
        .child(attr_str(node, "label").unwrap_or_default().to_owned())
        .into_any_element()
}

/// Validation messages under a control (plain text, danger colour).
fn errors_of(node: &CardNode, cx: &App) -> Vec<AnyElement> {
    errors(node)
        .into_iter()
        .map(|e| {
            div()
                .text_size(text_caption())
                .text_color(cx.theme().danger)
                .child(e)
                .into_any_element()
        })
        .collect()
}

/// A Button: its label is the text of its first child; a press sends the
/// drafts of the card's text fields, then the activation.
pub fn button(view: &View<'_>, node: &CardNode, window: &mut Window, cx: &mut App) -> AnyElement {
    let _ = window;
    let label = view
        .children(node)
        .first()
        .and_then(|child| attr_str(child, "text"))
        .unwrap_or_default()
        .to_owned();
    let enabled = view.enabled(node);
    let variant = attr_str(node, "variant").unwrap_or("default");
    let key = node.key.clone();
    let sink = view.sink.clone();
    // Drafts of the text fields still in the tree, sent before the press.
    let drafts: Vec<(String, Entity<gpui_kit::component::input::InputState>)> = view
        .inputs
        .iter()
        .filter(|(k, _)| {
            view.node(k).is_some_and(|n| n.kind == "TextField" || n.kind == "DateTimeInput")
        })
        .map(|(k, e)| (k.clone(), e.clone()))
        .collect();
    let button = Button::new(domain_id("card-button", &node.key))
        .small()
        .design_disabled(!enabled, cx)
        .design_label(label);
    let button = match variant {
        "primary" => button.design_primary(cx),
        "borderless" => button.design_quiet(cx),
        _ => button.design_secondary(cx),
    };
    let on_press = move |_: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut App| {
        for (field, entity) in &drafts {
            let value = entity.read(cx).value().to_string();
            sink.emit(
                ramflux_desktop_core::UiEventKind::CardInteracted {
                    key: field.clone(),
                    event: CardInput::Input { value: JsonValue::String(value) },
                },
                window,
                cx,
            );
        }
        sink.emit(
            ramflux_desktop_core::UiEventKind::CardInteracted {
                key: key.clone(),
                event: CardInput::Activate,
            },
            window,
            cx,
        );
    };
    let mut el = div().child(if enabled {
        button.on_click(on_press).into_any_element()
    } else {
        button.into_any_element()
    });
    for e in errors_of(node, cx) {
        el = el.child(e);
    }
    el.into_any_element()
}

/// A TextField: a single-line input whose draft the runtime holds; the value
/// shown at first is the runtime's own.
pub fn text_field(view: &View<'_>, node: &CardNode, cx: &App) -> AnyElement {
    let label = attr_str(node, "label").unwrap_or_default().to_owned();
    let mut col = v_flex().w_full().gap(space(1.)).child(label_of(node, cx));
    match view.inputs.get(&node.key) {
        Some(input) => {
            let enabled = view.enabled(node);
            col = col.child(
                Input::new(input)
                    .id(domain_id("card-field", &node.key))
                    .aria_label(label)
                    .small()
                    .disabled(!enabled),
            );
        }
        None => {
            col = col.child(
                div()
                    .text_size(text_caption())
                    .child(attr_str(node, "value").unwrap_or_default().to_owned()),
            );
        }
    }
    col.children(errors_of(node, cx)).into_any_element()
}

/// A DateTimeInput: an ISO date (or date and time) typed in a single line.
pub fn date_time(view: &View<'_>, node: &CardNode, cx: &App) -> AnyElement {
    text_field(view, node, cx)
}

/// A CheckBox: a box and its label; a press sends the flipped value.
pub fn check_box(
    view: &View<'_>,
    node: &CardNode,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let _ = window;
    let checked = attr_bool(node, "value");
    let enabled = view.enabled(node);
    let key = node.key.clone();
    let sink = view.sink.clone();
    let row = h_flex()
        .id(domain_id("card-check", &node.key))
        .gap(space(2.))
        .items_center()
        .cursor_pointer()
        .child(
            div()
                .size(px(16.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().border)
                .when(checked, |b| b.bg(cx.theme().primary))
                .when(checked, |b| {
                    b.child(
                        Icon::new(IconName::Check)
                            .small()
                            .text_color(cx.theme().primary_foreground),
                    )
                }),
        )
        .child(
            div()
                .text_size(text_caption())
                .child(attr_str(node, "label").unwrap_or_default().to_owned()),
        )
        .when(enabled, |r| {
            r.on_click(move |_, window, cx| {
                sink.emit(
                    ramflux_desktop_core::UiEventKind::CardInteracted {
                        key: key.clone(),
                        event: CardInput::Input { value: JsonValue::Bool(!checked) },
                    },
                    window,
                    cx,
                );
            })
        });
    v_flex().w_full().child(row).children(errors_of(node, cx)).into_any_element()
}

/// A ChoicePicker: options as buttons. Mutually exclusive choices send one
/// value; multiple selection sends the list of chosen values.
pub fn choice_picker(
    view: &View<'_>,
    node: &CardNode,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let _ = window;
    let options: Vec<(String, String)> = node
        .attrs
        .get("options")
        .and_then(JsonValue::as_array)
        .map(|rows| {
            rows.iter()
                .map(|o| {
                    let value = o.get("value").and_then(JsonValue::as_str).unwrap_or_default();
                    let label = o.get("label").and_then(JsonValue::as_str).unwrap_or(value);
                    (value.to_owned(), label.to_owned())
                })
                .collect()
        })
        .unwrap_or_default();
    let current: Vec<String> = match node.attrs.get("value") {
        Some(JsonValue::Array(items)) => {
            items.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()
        }
        Some(JsonValue::String(text)) => vec![text.clone()],
        _ => Vec::new(),
    };
    let multiple = attr_str(node, "variant") == Some("multipleSelection");
    let enabled = view.enabled(node);
    let buttons = options.into_iter().map(|(value, label)| {
        let chosen = current.contains(&value);
        let next: JsonValue = if multiple {
            let mut list: Vec<String> = current.clone();
            if chosen {
                list.retain(|v| v != &value);
            } else {
                list.push(value.clone());
            }
            JsonValue::Array(list.into_iter().map(JsonValue::String).collect())
        } else {
            JsonValue::String(value.clone())
        };
        let key = node.key.clone();
        let sink = view.sink.clone();
        let button = Button::new(domain_id("card-choice", &format!("{}-{value}", node.key)))
            .small()
            .design_secondary(cx)
            .selected(chosen)
            .design_disabled(!enabled, cx)
            .design_label(label);
        if enabled {
            button
                .on_click(move |_, window, cx| {
                    sink.emit(
                        ramflux_desktop_core::UiEventKind::CardInteracted {
                            key: key.clone(),
                            event: CardInput::Input { value: next.clone() },
                        },
                        window,
                        cx,
                    );
                })
                .into_any_element()
        } else {
            button.into_any_element()
        }
    });
    v_flex()
        .w_full()
        .gap(space(1.))
        .child(label_of(node, cx))
        .child(h_flex().flex_wrap().gap(space(1.)).children(buttons))
        .children(errors_of(node, cx))
        .into_any_element()
}

/// A Slider: ten steps across `min..max`; a tap on a step sends its value.
pub fn slider(view: &View<'_>, node: &CardNode, window: &mut Window, cx: &mut App) -> AnyElement {
    let _ = window;
    let min = attr_f64(node, "min").unwrap_or(0.0);
    let max = attr_f64(node, "max").unwrap_or(100.0).max(min);
    let value = attr_f64(node, "value").unwrap_or(min).clamp(min, max);
    let enabled = view.enabled(node);
    let steps = 10_u32;
    let filled =
        if max > min { ((value - min) / (max - min) * f64::from(steps)).round() as u32 } else { 0 };
    let segments = (0..=steps).map(|step| {
        let target = min + (max - min) * f64::from(step) / f64::from(steps);
        let key = node.key.clone();
        let sink = view.sink.clone();
        let on = step <= filled;
        let seg = div()
            .id(domain_id("card-slider", &format!("{}-{step}", node.key)))
            .h(px(8.))
            .flex_1()
            .rounded(cx.theme().radius)
            .bg(if on { cx.theme().primary } else { cx.theme().muted });
        if enabled {
            seg.cursor_pointer()
                .on_click(move |_, window, cx| {
                    sink.emit(
                        ramflux_desktop_core::UiEventKind::CardInteracted {
                            key: key.clone(),
                            event: CardInput::Input { value: JsonValue::from(target) },
                        },
                        window,
                        cx,
                    );
                })
                .into_any_element()
        } else {
            seg.into_any_element()
        }
    });
    v_flex()
        .w_full()
        .gap(space(1.))
        .child(
            h_flex()
                .justify_between()
                .child(label_of(node, cx))
                .child(div().text_size(text_caption()).child(format!("{value}"))),
        )
        .child(h_flex().w_full().gap(space(0.5)).children(segments))
        .children(errors_of(node, cx))
        .into_any_element()
}
