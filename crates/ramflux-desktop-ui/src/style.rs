// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The design's buttons on top of Kit's buttons (design components
//! `Button / Primary`, `Secondary`, `Disabled`, `Danger`, `Danger / Quiet`
//! and `Text` on boards 00B and 00D).
//!
//! Every design button is 32 px high with a 12 px inset, the button radius
//! (`rf-radius-button`) and its label in the button size (`rf-size-button`,
//! 14 px). Colours are design variables:
//!
//! | Button | Fill | Label | Edge |
//! |---|---|---|---|
//! | Primary | `rf-primary` | `rf-on-primary` | – |
//! | Secondary | `rf-surface-white` | `rf-text` | 1 px `rf-text` |
//! | Danger | `rf-danger` | `rf-on-danger` | – |
//! | Danger / Quiet | `rf-danger-bg` | `rf-danger` | – |
//! | Quiet | `rf-surface` | `rf-text` | – |
//! | Text | none | `rf-text` | – |
//! | Disabled (any) | `rf-surface` | `rf-control` | – |
//!
//! Pointer feedback uses design variables too: the fill darkens to
//! `rf-accent` on a primary button, and lightens to `rf-selected` on the
//! light ones.
//!
//! Kit sizes a button's label from the button size (13.125 px at the 15 px
//! rem); [`DesignButton::design_label`] puts the label in at 14 px, medium
//! weight (the boards' label ink matches weight 500), instead.
//! Kit's custom variant tints its idle fill, so the solid fill is set on the
//! button and the variant only gives the pointer feedback; the button's own
//! colours also apply while disabled, so a design button is disabled with
//! [`DesignButton::design_disabled`] (or `gate::gated`), which paints the
//! design's disabled colours.

use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::prelude::*;
use gpui_kit::{App, ElementId, Hsla, SharedString, div, px};
use ramflux_desktop_navigation::scale::{DesignText as _, design_px, space, style as text_style};

/// The design's button kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Primary,
    Secondary,
    Danger,
    DangerQuiet,
    Quiet,
    Text,
}

/// The design height of a button.
const HEIGHT_PX: f32 = 32.0;

struct Colours {
    fill: Hsla,
    label: Hsla,
    hover: Hsla,
    edge: Option<Hsla>,
}

fn colours(kind: ButtonKind, cx: &App) -> Colours {
    let p = ramflux_desktop_navigation::palette(cx);
    let transparent = p.bg.opacity(0.);
    match kind {
        ButtonKind::Primary => {
            Colours { fill: p.primary, label: p.on_primary, hover: p.accent, edge: None }
        }
        ButtonKind::Secondary => {
            Colours { fill: p.surface_white, label: p.text, hover: p.selected, edge: Some(p.text) }
        }
        ButtonKind::Danger => {
            Colours { fill: p.danger, label: p.on_danger, hover: p.danger, edge: None }
        }
        ButtonKind::DangerQuiet => {
            Colours { fill: p.danger_bg, label: p.danger, hover: p.danger_bg, edge: None }
        }
        ButtonKind::Quiet => {
            Colours { fill: p.surface, label: p.text, hover: p.selected, edge: None }
        }
        ButtonKind::Text => {
            Colours { fill: transparent, label: p.text, hover: p.surface, edge: None }
        }
    }
}

/// A design button style prepared with the theme, for places that build
/// buttons where no `App` is at hand (inside a closure that borrows it).
#[derive(Clone)]
pub struct DesignStyle {
    variant: ButtonCustomVariant,
    fill: Hsla,
    label: Hsla,
    radius: f32,
    edge: Option<Hsla>,
}

impl DesignStyle {
    pub fn new(kind: ButtonKind, cx: &App) -> Self {
        let p = ramflux_desktop_navigation::palette(cx);
        let c = colours(kind, cx);
        let variant = ButtonCustomVariant::new(cx)
            .color(c.fill)
            .foreground(c.label)
            .hover(c.hover)
            .active(c.hover);
        Self { variant, fill: c.fill, label: c.label, radius: p.radius_button, edge: c.edge }
    }
}

/// The design's button geometry and colours for a Kit [`Button`].
pub trait DesignButton: Sized {
    /// A prepared design style.
    fn design_style(self, style: &DesignStyle) -> Self;
    /// One of the design's buttons.
    fn design(self, kind: ButtonKind, cx: &App) -> Self;
    /// The label at the design's button size (`rf-size-button`).
    fn design_label(self, label: impl Into<SharedString>) -> Self;
    /// Disables the button in the design's disabled colours.
    fn design_disabled(self, disabled: bool, cx: &App) -> Self;

    /// The design's secondary button (outline on the white surface).
    fn design_secondary(self, cx: &App) -> Self {
        self.design(ButtonKind::Secondary, cx)
    }
    /// The design's primary button.
    fn design_primary(self, cx: &App) -> Self {
        self.design(ButtonKind::Primary, cx)
    }
    /// The design's quiet button: the surface fill, no outline.
    fn design_quiet(self, cx: &App) -> Self {
        self.design(ButtonKind::Quiet, cx)
    }
    /// The design's danger button (a solid `rf-danger` fill).
    fn design_danger(self, cx: &App) -> Self {
        self.design(ButtonKind::Danger, cx)
    }
    /// The design's quiet danger button (`rf-danger-bg` fill).
    fn design_danger_quiet(self, cx: &App) -> Self {
        self.design(ButtonKind::DangerQuiet, cx)
    }
    /// The design's text button (no fill).
    fn design_text(self, cx: &App) -> Self {
        self.design(ButtonKind::Text, cx)
    }
    /// The secondary button where no theme is at hand (a table cell): the
    /// design geometry with Kit's outline in the theme's colours.
    fn design_secondary_compact(self) -> Self;
}

impl DesignButton for Button {
    fn design(self, kind: ButtonKind, cx: &App) -> Self {
        self.design_style(&DesignStyle::new(kind, cx))
    }

    fn design_style(self, style: &DesignStyle) -> Self {
        // Kit's custom variant supplies the pointer feedback; its idle fill is
        // a tint, so the solid fill and label colour are set here.
        self.custom(style.variant)
            .bg(style.fill)
            .text_color(style.label)
            .h(design_px(HEIGHT_PX))
            .px(space(3.))
            .rounded(px(style.radius))
            .when_some(style.edge, |b, edge| b.border_1().border_color(edge))
    }

    fn design_secondary_compact(self) -> Self {
        self.outline().h(design_px(HEIGHT_PX)).px(space(3.))
    }

    fn design_label(self, label: impl Into<SharedString>) -> Self {
        let label = label.into();
        self.accessibility_label(label.clone()).child(
            // The button text style: 14 px medium on its 20 px line, centred
            // in the 32 px button (the boards' label ink sits 10–22 px down).
            div().design_text(text_style::button()).child(label),
        )
    }

    fn design_disabled(self, disabled: bool, cx: &App) -> Self {
        let p = ramflux_desktop_navigation::palette(cx);
        self.disabled(disabled)
            .when(disabled, |b| b.bg(p.surface).text_color(p.control).border_color(p.surface))
    }
}

/// A design primary button.
pub fn primary_button(id: impl Into<ElementId>, cx: &App) -> Button {
    Button::new(id).design_primary(cx)
}

/// A design secondary button.
pub fn secondary_button(id: impl Into<ElementId>, cx: &App) -> Button {
    Button::new(id).design_secondary(cx)
}
