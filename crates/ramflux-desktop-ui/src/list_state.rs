// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The placeholder of a list that has nothing to show: loading (the source
//! has not listed the collection yet; a spinner) or empty (it answered with
//! nothing; an explicit sentence that names the next step). A list with
//! contents shows them and no placeholder.
//!
//! The placeholder is a status node, so assistive technology announces
//! "Loading…" or the empty sentence instead of silence.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, ElementId, Role, SharedString, div};
use ramflux_desktop_core::{ListState, Listing, Store};

use crate::i18n::tr;

/// The list state of `listing` in `store` for a list whose shown contents
/// are `empty` or not.
pub fn of(store: &Store, listing: Listing, empty: bool) -> ListState {
    ListState::of(store.is_listed(listing), empty)
}

/// The placeholder for `state`: a spinner with "Loading…", the translated
/// `empty_key` sentence, or nothing when there is content.
pub fn placeholder(
    id: impl Into<ElementId>,
    state: ListState,
    empty_key: &str,
    cx: &App,
) -> Option<AnyElement> {
    let (label, loading): (SharedString, bool) = match state {
        ListState::Ready => return None,
        ListState::Loading => (tr("list.loading"), true),
        ListState::Empty => (tr(empty_key), false),
    };
    Some(
        h_flex()
            .id(id)
            .role(Role::Status)
            .aria_label(label.clone())
            .test_support()
            .gap_2()
            .px_3()
            .py_2()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .when(loading, |row| row.child(Spinner::new().small()))
            .child(div().child(label))
            .into_any_element(),
    )
}
