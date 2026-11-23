// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The design checkbox keeps the behaviour of a checkbox: its role and
//! state, pointer and keyboard toggling, no focus on a pointer press, an
//! inert disabled state.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    Context, IntoElement, ParentElement as _, Render, Role, Styled as _, TestAppContext, Window,
    div, px,
};
use ramflux_desktop_ui::controls::DesignCheckbox;

struct Harness {
    checked: Rc<Cell<bool>>,
    disabled: bool,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let checked = self.checked.clone();
        div().size(px(200.)).child(
            DesignCheckbox::new("box")
                .checked(self.checked.get())
                .disabled(self.disabled)
                .label("Files")
                .on_click(move |next, _, _| checked.set(*next)),
        )
    }
}

fn open(
    cx: &mut TestAppContext,
    disabled: bool,
) -> (Rc<Cell<bool>>, &mut gpui_kit::VisualTestContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        ramflux_desktop_ui::init(cx);
    });
    let checked = Rc::new(Cell::new(false));
    let state = checked.clone();
    let (_, window) = cx.add_window_view(move |_, _| Harness { checked: state, disabled });
    window.run_until_parked();
    (checked, window)
}

#[gpui_kit::test]
fn a_click_toggles_without_taking_focus(cx: &mut TestAppContext) {
    let (checked, cx) = open(cx, false);
    let found = cx.update(|window, _| window.find("box"));
    assert_eq!(found.role(), Some(Role::CheckBox));
    assert_eq!(found.checked(), Some(false));
    cx.update(|window, cx| window.click("box", cx));
    cx.run_until_parked();
    assert!(checked.get(), "a click checks the box");
    let found = cx.update(|window, _| window.find("box"));
    assert_eq!(found.checked(), Some(true));
    assert_ne!(found.focused(), Some(true), "a pointer press does not move focus");
    cx.update(|window, cx| window.click("box", cx));
    cx.run_until_parked();
    assert!(!checked.get(), "a second click clears it");
}

#[gpui_kit::test]
fn the_next_focus_and_space_toggle_it(cx: &mut TestAppContext) {
    let (checked, cx) = open(cx, false);
    cx.update(|window, cx| window.focus_next(cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|window, _| window.find("box").focused()), Some(true));
    cx.update(|window, cx| window.press("space", cx));
    cx.run_until_parked();
    assert!(checked.get(), "space checks the focused box");
}

#[gpui_kit::test]
fn a_disabled_box_is_inert(cx: &mut TestAppContext) {
    let (checked, cx) = open(cx, true);
    cx.update(|window, cx| window.click("box", cx));
    cx.run_until_parked();
    assert!(!checked.get());
    assert_eq!(cx.update(|window, _| window.find("box").disabled()), Some(true));
}
