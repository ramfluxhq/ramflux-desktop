// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Pieces the ceremony screens (creation and recovery) share: a message
//! node a screen reader and the tests can read, the warning block, muted
//! text, the focus ring and the countdown clock.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, ThemeStyled as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, IntoElement, ParentElement, Role, SharedString, Styled, Window, div,
};

use crate::i18n::tr;

/// A node that announces `text` with `role`, which `child` shows. A `div`
/// with no role is not in the accessibility tree and the alert's own text
/// is not exposed, so this node carries the message.
pub(super) fn announced(
    id: &'static str,
    role: Role,
    text: SharedString,
    child: impl IntoElement,
) -> AnyElement {
    div().id(id).role(role).aria_label(text).test_support().child(child).into_any_element()
}

pub(super) fn warning(id: &'static str, key: &str) -> AnyElement {
    let text = tr(key);
    announced(
        id,
        Role::Status,
        text.clone(),
        Alert::warning(SharedString::from(format!("{id}-alert")), text).small(),
    )
}

pub(super) fn muted(text: SharedString, cx: &App) -> AnyElement {
    div().text_sm().text_color(cx.theme().muted_foreground).child(text).into_any_element()
}

/// Draws the framework's focus ring around `element` while `focused`.
pub(super) fn ringed<E: Styled + ParentElement>(
    element: E,
    focused: bool,
    window: &Window,
    cx: &App,
) -> E {
    if focused { element.focus_ring_style(window, cx) } else { element }
}

pub(super) fn clock(remaining_ms: u64) -> String {
    let seconds = remaining_ms.div_ceil(1_000);
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
