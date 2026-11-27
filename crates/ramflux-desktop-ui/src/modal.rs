// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The shared modal dialog container (design boards B11, C01, E05, F02, G24).
//!
//! A modal dialog is a card centred over a dimmed window: the scrim colour
//! over the whole shell, a card of the board's width with the dialog radius
//! and 24 px padding, a title row with a close button, the body, and a
//! button row at the foot. Tab stays inside the card (focus trap), Escape
//! and a click on the scrim close it. The host owns the dialog's state and
//! draws the element where the window is covered (the workspace root); the
//! container owns only the chrome.

use std::rc::Rc;

use crate::actions::{CloseModal, MODAL_CONTEXT};
use crate::i18n::tr;
use crate::style::DesignButton as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{FocusTrapElement as _, IconName, StyledExt as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, ElementId, FocusHandle, IntoElement, MouseButton, SharedString, Window, div,
    rems,
};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::space;

/// The dialog widths of the boards, in design px.
pub mod width {
    /// Add contact, compare safety numbers, create group (B11, C01, F02).
    pub const STANDARD: f32 = 608.0;
    /// Task details (E05).
    pub const WIDE: f32 = 680.0;
    /// Approve linking a web device (G24).
    pub const NARROW: f32 = 520.0;
}

/// Closes the dialog (Escape, the close button, a click on the scrim).
pub type OnClose = Rc<dyn Fn(&mut Window, &mut App)>;

/// A modal dialog: `id` names the overlay (the card is `<id>-card`),
/// `focus` is the focus handle the trap tracks.
pub struct Modal {
    id: ElementId,
    title: SharedString,
    width: f32,
    focus: FocusHandle,
    on_close: OnClose,
    body: Vec<AnyElement>,
    footer: Vec<AnyElement>,
    closable: bool,
}

impl Modal {
    /// A dialog with `title`, closed through `on_close`.
    pub fn new(
        id: impl Into<ElementId>,
        title: SharedString,
        focus: &FocusHandle,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            title,
            width: width::STANDARD,
            focus: focus.clone(),
            on_close: Rc::new(on_close),
            body: Vec::new(),
            footer: Vec::new(),
            closable: true,
        }
    }

    /// The card width in design px (see [`width`]).
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Whether the title row carries a close button (default). A dialog
    /// that asks a question the user must answer (board G24) has only its
    /// buttons; Escape and the scrim still close it.
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }

    /// Adds body content under the title.
    pub fn child(mut self, element: impl IntoElement) -> Self {
        self.body.push(element.into_any_element());
        self
    }

    /// Adds a button to the foot row (laid out at the inline end, in order).
    pub fn button(mut self, button: impl IntoElement) -> Self {
        self.footer.push(button.into_any_element());
        self
    }

    /// The overlay element.
    pub fn render(self, cx: &App) -> AnyElement {
        let palette = ramflux_desktop_navigation::palette(cx);
        let close_scrim = self.on_close.clone();
        let close_key = self.on_close.clone();
        let close_button = self.on_close;
        let card_id = ElementId::Name(format!("{:?}-card", self.id).into());
        let footer = (!self.footer.is_empty()).then(|| {
            h_flex().id("modal-buttons").gap(space(2.)).child(div().flex_1()).children(self.footer)
        });
        div()
            .id(self.id)
            .key_context(MODAL_CONTEXT)
            .on_action(move |_: &CloseModal, window, cx| close_key(window, cx))
            .absolute()
            .inset_0()
            .occlude()
            .bg(palette.scrim)
            .flex()
            .justify_center()
            .items_center()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| close_scrim(window, cx))
            .child(
                v_flex()
                    .id(card_id)
                    .test_support()
                    .role(gpui_kit::Role::Dialog)
                    .aria_label(self.title.clone())
                    .w(rems(self.width / 15.0))
                    .max_w_full()
                    .max_h_full()
                    .overflow_y_scroll()
                    .gap(space(4.))
                    .p(rems(24.0 / 15.0))
                    .bg(palette.overlay)
                    .border_1()
                    .border_color(palette.overlay_border)
                    .rounded(rems(palette.radius_dialog / 15.0))
                    .shadow_lg()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .id("modal-title-row")
                            .h(gpui_kit::px(34.0))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(rems(palette.size_subheading / 15.0))
                                    .font_semibold()
                                    .child(self.title),
                            )
                            .when(self.closable, |row| {
                                row.child(
                                    Button::new("modal-close")
                                        .ghost()
                                        .icon(IconName::Close)
                                        .accessibility_label(tr("common.close"))
                                        .on_click(move |_, window, cx| close_button(window, cx)),
                                )
                            }),
                    )
                    .children(self.body)
                    .children(footer)
                    .focus_trap("modal-trap", &self.focus),
            )
            .into_any_element()
    }
}

/// A secondary (cancel) button of the foot row.
pub fn cancel_button(
    id: impl Into<ElementId>,
    label: SharedString,
    cx: &App,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .design_secondary(cx)
        .design_label(label)
        .on_click(move |_, window, cx| on_click(window, cx))
}
