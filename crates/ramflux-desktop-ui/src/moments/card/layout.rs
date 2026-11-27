// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Layout nodes: Row, Column, List, Card, Divider, Tabs and the Modal
//! trigger. Tabs and Modal presentation are the host's: their selection and
//! sheet are held locally, and the runtime is told of each change.

use crate::ids::domain_id;
use gpui_kit::component::Selectable as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, IntoElement, Window, div, px};
use ramflux_desktop_core::UiEventKind;
use ramflux_desktop_core::{CardInput, CardNode};
use ramflux_desktop_navigation::direction::h_flex_in;
use ramflux_desktop_navigation::scale::{space, text_caption};

use super::{View, attr_str, errors, slots};
use crate::i18n::tr_args;
use crate::style::DesignButton as _;

/// Items a List shows before its "Show all" control.
const LIST_PREVIEW: usize = 5;

/// A Row: children in a line, in the tree's direction.
pub fn row(view: &View<'_>, node: &CardNode, window: &mut Window, cx: &mut App) -> AnyElement {
    let children: Vec<AnyElement> =
        view.children(node).into_iter().map(|c| view.render(c, window, cx)).collect();
    h_flex_in(view.dir)
        .flex_wrap()
        .gap(space(2.))
        .items_center()
        .children(children)
        .into_any_element()
}

/// A Column: children stacked, in order.
pub fn column(view: &View<'_>, node: &CardNode, window: &mut Window, cx: &mut App) -> AnyElement {
    let children: Vec<AnyElement> =
        view.children(node).into_iter().map(|c| view.render(c, window, cx)).collect();
    v_flex().gap(space(3.)).children(children).into_any_element()
}

/// A List: a vertical list with rules between items (first five, then
/// "Show all"), or a horizontal strip that clips at the card's edge.
pub fn list(view: &View<'_>, node: &CardNode, window: &mut Window, cx: &mut App) -> AnyElement {
    let items = view.children(node);
    let horizontal = attr_str(node, "direction") == Some("horizontal");
    if items.is_empty() {
        return div()
            .h(px(48.))
            .flex()
            .items_center()
            .justify_center()
            .text_size(text_caption())
            .text_color(cx.theme().muted_foreground)
            .child(crate::i18n::tr("moment.card.nothing_to_show"))
            .into_any_element();
    }
    if horizontal {
        let cells: Vec<AnyElement> =
            items.into_iter().map(|c| view.render(c, window, cx)).collect();
        return div()
            .w_full()
            .overflow_hidden()
            .child(h_flex_in(view.dir).gap(space(2.)).children(cells))
            .into_any_element();
    }
    let expanded = view.local.borrow().expanded.contains(&node.key);
    let total = items.len();
    let shown = if expanded { total } else { total.min(LIST_PREVIEW) };
    let mut rows: Vec<AnyElement> = Vec::new();
    for (ix, item) in items.into_iter().take(shown).enumerate() {
        if ix > 0 {
            rows.push(div().h(px(1.)).w_full().bg(cx.theme().border).into_any_element());
        }
        rows.push(div().py(space(1.5)).child(view.render(item, window, cx)).into_any_element());
    }
    let mut col = v_flex().w_full().children(rows);
    if !expanded && total > LIST_PREVIEW {
        let local = view.local.clone();
        let key = node.key.clone();
        col = col.child(
            Button::new(domain_id("card-id", &format!("list-more-{key}")))
                .small()
                .design_secondary(cx)
                .design_label(tr_args("moment.card.show_all", &[("count", &total.to_string())]))
                .on_click(move |_, window, _| {
                    local.borrow_mut().expanded.insert(key.clone());
                    window.refresh();
                }),
        );
    }
    col.into_any_element()
}

/// A nested Card: an inset block with no border of its own.
pub fn card(view: &View<'_>, node: &CardNode, window: &mut Window, cx: &mut App) -> AnyElement {
    let children: Vec<AnyElement> =
        view.children(node).into_iter().map(|c| view.render(c, window, cx)).collect();
    v_flex()
        .w_full()
        .gap(space(2.))
        .p(space(3.))
        .rounded(cx.theme().radius)
        .bg(cx.theme().muted)
        .children(children)
        .into_any_element()
}

/// A Divider: a 1 px rule, decorative.
pub fn divider(node: &CardNode, cx: &App) -> AnyElement {
    let vertical = attr_str(node, "axis") == Some("vertical");
    let rule = div().bg(cx.theme().border);
    if vertical {
        rule.w(px(1.)).h_full().into_any_element()
    } else {
        rule.h(px(1.)).w_full().my(space(1.)).into_any_element()
    }
}

/// Tabs: a segmented row of titles and the selected panel (the second child
/// onward are the panels, in order of the tabs).
pub fn tabs(view: &View<'_>, node: &CardNode, window: &mut Window, cx: &mut App) -> AnyElement {
    let titles: Vec<String> = node
        .attrs
        .get("tabs")
        .and_then(|t| t.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|t| t.get("title").and_then(|v| v.as_str()).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let panels = view.children(node);
    let selected = view
        .local
        .borrow()
        .tabs
        .get(&node.key)
        .copied()
        .unwrap_or(0)
        .min(titles.len().saturating_sub(1) as u64);
    let buttons: Vec<AnyElement> = titles
        .into_iter()
        .enumerate()
        .map(|(index, title)| {
            let local = view.local.clone();
            let key = node.key.clone();
            let emit_view = view.sink.clone();
            let key_for_event = node.key.clone();
            let is_selected = index as u64 == selected;
            let enabled = view.enabled(node);
            Button::new(domain_id("card-id", &format!("tab-{}-{index}", node.key)))
                .small()
                .design_secondary(cx)
                .selected(is_selected)
                .design_disabled(!enabled, cx)
                .design_label(title)
                .on_click(move |_, window, cx| {
                    local.borrow_mut().tabs.insert(key.clone(), index as u64);
                    emit_view.emit(
                        UiEventKind::CardInteracted {
                            key: key_for_event.clone(),
                            event: CardInput::SelectTab { index: index as u64 },
                        },
                        window,
                        cx,
                    );
                    window.refresh();
                })
                .into_any_element()
        })
        .collect();
    let panel = panels
        .get(selected as usize)
        .map(|p| view.render(p, window, cx))
        .unwrap_or_else(|| div().into_any_element());
    v_flex()
        .w_full()
        .gap(space(3.))
        .child(h_flex_in(view.dir).w_full().gap(space(1.)).flex_wrap().children(buttons))
        .child(panel)
        .children(
            errors(node)
                .into_iter()
                .map(|e| div().text_size(text_caption()).text_color(cx.theme().danger).child(e)),
        )
        .into_any_element()
}

/// A Modal: its trigger (first child) and its content (second). The trigger
/// opens the host's sheet; the sheet is drawn by [`slots::modal_sheet`].
pub fn modal(view: &View<'_>, node: &CardNode, window: &mut Window, cx: &mut App) -> AnyElement {
    let children = view.children(node);
    let Some(trigger) = children.first().copied() else {
        return div().into_any_element();
    };
    let trigger_el = view.render(trigger, window, cx);
    let opened = view.local.borrow().modal.as_deref() == Some(node.key.as_str());
    let trigger_key = node.key.clone();
    let local = view.local.clone();
    let emit_view = view.sink.clone();
    let enabled = view.enabled(node);
    let open_trigger = div()
        .id(domain_id("card-id", &format!("modal-trigger-{}", node.key)))
        .cursor_pointer()
        .when(enabled, |el| {
            el.on_click(move |_, window, cx| {
                local.borrow_mut().modal = Some(trigger_key.clone());
                emit_view.emit(
                    UiEventKind::CardInteracted {
                        key: trigger_key.clone(),
                        event: CardInput::ModalOpen,
                    },
                    window,
                    cx,
                );
                window.refresh();
            })
        })
        .child(trigger_el);
    let mut col = v_flex().w_full().child(open_trigger);
    if opened && let Some(content) = children.get(1).copied() {
        col = col.child(slots::modal_sheet(view, node, content, window, cx));
    }
    col.into_any_element()
}
