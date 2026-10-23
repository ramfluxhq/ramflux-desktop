// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Controls for actions the attached backend cannot perform.
//!
//! Such a control stays visible but disabled, with the backend's reason as
//! its tooltip, so the user sees what exists and why it is not available
//! yet. Policy refuses the intent as well; this only keeps the UI honest.

use gpui_kit::SharedString;
use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::Button;
use ramflux_desktop_core::Store;

use crate::i18n;

/// Why `method` is unavailable here (translated), if it is.
pub fn reason(store: &Store, method: &str) -> Option<SharedString> {
    i18n::unsupported(store, method)
}

/// `button`, disabled with `reason` as its tooltip when there is one.
pub fn gated(button: Button, reason: Option<SharedString>) -> Button {
    match reason {
        Some(reason) => button.disabled(true).tooltip(reason),
        None => button,
    }
}
