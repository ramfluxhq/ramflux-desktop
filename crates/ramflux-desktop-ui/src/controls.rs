// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The design's selection controls (components `RF / Checkbox` on, off and
//! disabled, `RF / Radio` on and off on board 00D).
//!
//! Both are 18 px:
//!
//! | Control | Fill | Edge | Mark |
//! |---|---|---|---|
//! | Checkbox on | `rf-primary` | – | check in `rf-on-primary` |
//! | Checkbox off | `rf-input` | 1 px `rf-control` | – |
//! | Checkbox disabled | `rf-surface` | 1 px `rf-line-strong` | – |
//! | Radio on | none | 1.5 px `rf-primary` | 8 px dot in `rf-primary` |
//! | Radio off | `rf-input` | 1 px `rf-control` | – |
//!
//! The checkbox corner is `rf-radius-xs`. The design draws its edges as
//! centred strokes; GPUI draws borders inside the box, so the outer edge is
//! the same and the inner half-pixel differs (the instrument's `stroke`
//! class). A checked and disabled checkbox has no board; it takes the
//! disabled edge with an `rf-control` fill.
//!
//! [`DesignCheckbox`] is the base checkbox of the UI kit (role, toggle state,
//! focus and keyboard activation) with the design's indicator; the Kit
//! checkbox's indicator is fixed at 15 px with the theme's input edge.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::{Checkbox as BaseCheckbox, CheckboxState};
use gpui_kit::component::ThemeStyled as _;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, ElementId, MouseButton, SharedString, Window, div, svg};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{design_px, space};

/// The design size of a checkbox or radio.
const CONTROL_PX: f32 = 18.0;
/// The radio's selected dot.
const DOT_PX: f32 = 8.0;
/// The check mark's icon box inside the checkbox.
const CHECK_PX: f32 = 12.0;
/// The selected radio's ring (a centred 1.5 px stroke on the board).
const RING_ON_PX: f32 = 1.5;

type ClickHandler = Rc<dyn Fn(&bool, &mut Window, &mut App) + 'static>;

/// A checkbox in the design's style.
#[derive(IntoElement)]
pub struct DesignCheckbox {
    id: ElementId,
    checked: bool,
    disabled: bool,
    label: Option<SharedString>,
    accessibility_label: Option<SharedString>,
    on_click: Option<ClickHandler>,
}

impl DesignCheckbox {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            checked: false,
            disabled: false,
            label: None,
            accessibility_label: None,
            on_click: None,
        }
    }

    #[must_use]
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The visible label after the box; also its accessible name.
    #[must_use]
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The accessible name when there is no visible label.
    #[must_use]
    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }

    /// Called with the new state when the person toggles the box.
    #[must_use]
    pub fn on_click(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    /// Same as [`Self::on_click`] (the Kit checkbox's name for it).
    #[must_use]
    pub fn on_change(self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_click(handler)
    }
}

/// The checkbox's box, alone (no behaviour): for captures and for views that
/// draw the state of a row they make interactive themselves.
pub fn checkbox_indicator(checked: bool, disabled: bool, cx: &App) -> AnyElement {
    let p = ramflux_desktop_navigation::palette(cx);
    let (fill, edge) = match (checked, disabled) {
        (true, false) => (p.primary, None),
        (true, true) => (p.control, Some(p.line_strong)),
        (false, false) => (p.input, Some(p.control)),
        (false, true) => (p.surface, Some(p.line_strong)),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(design_px(CONTROL_PX))
        .rounded(design_px(p.radius_xs))
        .bg(fill)
        .when_some(edge, |b, edge| b.border_1().border_color(edge))
        .when(checked, |b| {
            b.child(
                svg()
                    .path(IconName::Check.path())
                    .size(design_px(CHECK_PX))
                    .text_color(p.on_primary),
            )
        })
        .into_any_element()
}

/// The radio's circle, alone. Radios are drawn inside a control that
/// carries the selection semantics (a toggled list button).
pub fn radio_indicator(selected: bool, cx: &App) -> AnyElement {
    let p = ramflux_desktop_navigation::palette(cx);
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(design_px(CONTROL_PX))
        .rounded(design_px(p.radius_full))
        .map(|ring| {
            if selected {
                ring.border(design_px(RING_ON_PX)).border_color(p.primary).child(
                    div().size(design_px(DOT_PX)).rounded(design_px(p.radius_full)).bg(p.primary),
                )
            } else {
                ring.bg(p.input).border_1().border_color(p.control)
            }
        })
        .into_any_element()
}

impl RenderOnce for DesignCheckbox {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let checked = self.checked;
        let focus = window.use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle());
        let focus = focus.read(cx).clone();
        let focused = focus.is_focused(window);
        let p = ramflux_desktop_navigation::palette(cx);
        let name = self.accessibility_label.or_else(|| self.label.clone());
        let on_click = self.on_click;
        BaseCheckbox::new(self.id)
            .checked(checked)
            .disabled(self.disabled)
            .track_focus(&focus)
            .when_some(name, |b, name| b.accessibility_label(name))
            .when_some(on_click, |b, on_click| {
                b.on_change(move |state, _, window, cx| {
                    window.prevent_default();
                    on_click(&matches!(state, CheckboxState::Checked), window, cx);
                })
            })
            .child(
                h_flex()
                    .items_center()
                    .gap(space(3.))
                    .rounded(design_px(p.radius_xs))
                    .when(focused, |row| row.focus_ring_style(window, cx))
                    .child(checkbox_indicator(checked, self.disabled, cx))
                    .when_some(self.label, |row, label| {
                        row.child(
                            div()
                                .text_color(if self.disabled { p.control } else { p.text })
                                .child(label),
                        )
                    }),
            )
            // Pointer presses do not move focus (as the Kit checkbox).
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
    }
}
