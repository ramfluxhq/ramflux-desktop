// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! GPUI Kit views for Ramflux Desktop.

use gpui_kit::component::TitleBar;
use gpui_kit::*;

/// Top-level workspace view hosted inside the window `Root`.
pub struct Workspace;

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(TitleBar::new().child(div().child("Ramflux")))
            .child(div().flex_1().flex().items_center().justify_center().child("Ramflux Desktop"))
    }
}
