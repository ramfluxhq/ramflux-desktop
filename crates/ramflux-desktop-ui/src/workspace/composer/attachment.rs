// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The file staged in the composer: chosen, checked, then sent.
//!
//! A file chosen in the native picker is checked at once against what one
//! message carries ([`ramflux_desktop_sdk::ATTACHMENT_LIMIT`], the
//! daemon's frame budget). A file that fits waits in the composer until
//! Send; removing it there cancels it (nothing was sent). On Send it
//! travels as [`ramflux_desktop_core::UiEventKind::AttachFile`] through the
//! conversation's sink; from then on its progress shows on its message in
//! the conversation. A refusal before anything was sent (no relay known, the
//! file changed) comes back as a failed `object.send` and is shown here.
//! All of this is view-local state.

use std::path::Path;

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Role, SharedString, div};
use ramflux_desktop_core::{Direction, Store, methods};

use crate::i18n::{self, tr, tr_args};

/// The staged file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Staged {
    /// Fits; waits for Send.
    Ready {
        /// Local path.
        path: String,
        /// File name.
        name: String,
        /// Size in bytes.
        size: u64,
    },
    /// Larger than one message carries; never sent.
    TooLarge {
        /// File name.
        name: String,
        /// Size in bytes.
        size: u64,
    },
    /// The file could not be read.
    Unreadable {
        /// File name.
        name: String,
        /// Why.
        reason: String,
    },
    /// Sent; a refusal recorded after `since` failures is shown, and the
    /// chip leaves once an outbound object with this name is added (its
    /// message shows the rest).
    Sent {
        /// File name.
        name: String,
        /// The store's failure count when it was sent.
        since: u64,
        /// Outbound objects with this name when it was sent.
        listed: usize,
    },
    /// Refused before anything was sent.
    Failed {
        /// File name.
        name: String,
        /// Translated reason.
        reason: SharedString,
    },
}

impl Staged {
    /// Checks the file at `path`.
    pub fn check(path: &str) -> Self {
        let name = Path::new(path)
            .file_name()
            .map_or_else(|| path.to_owned(), |n| n.to_string_lossy().into_owned());
        match std::fs::metadata(path) {
            Ok(meta) if !meta.is_file() => {
                Self::Unreadable { name, reason: tr("composer.attach.not_a_file").to_string() }
            }
            Ok(meta) if meta.len() > ramflux_desktop_sdk::ATTACHMENT_LIMIT as u64 => {
                Self::TooLarge { name, size: meta.len() }
            }
            Ok(meta) => Self::Ready { path: path.to_owned(), name, size: meta.len() },
            Err(error) => Self::Unreadable { name, reason: error.to_string() },
        }
    }

    /// The path to send, when it is ready.
    pub fn ready_path(&self) -> Option<&str> {
        match self {
            Self::Ready { path, .. } => Some(path),
            _ => None,
        }
    }

    /// The staged file once sent.
    pub fn sent(&self, store: &Store) -> Option<Self> {
        let Self::Ready { name, .. } = self else { return None };
        Some(Self::Sent {
            name: name.clone(),
            since: store.failures_total(),
            listed: outbound_named(store, name),
        })
    }

    /// Follows the store after Send: `Some(next)` when the chip changes (a
    /// failed `object.send` is shown; `None` removes the chip once the
    /// message is in the conversation).
    pub fn observe(&self, store: &Store) -> Option<Option<Self>> {
        let Self::Sent { name, since, listed } = self else { return None };
        if let Some(failure) =
            store.failures_since(*since).iter().rev().find(|f| f.method == methods::ATTACH_OBJECT)
        {
            let reason = i18n::backend_text(&failure.message);
            return Some(Some(Self::Failed { name: name.clone(), reason }));
        }
        (outbound_named(store, name) > *listed).then_some(None)
    }
}

fn outbound_named(store: &Store, name: &str) -> usize {
    store
        .objects()
        .filter(|o| o.direction == Direction::Outbound && o.name.as_deref() == Some(name))
        .count()
}

/// The chip above the composer row: name and size (or why not), and a
/// remove button that cancels it.
pub fn chip(
    staged: &Staged,
    on_remove: impl Fn(&gpui_kit::ClickEvent, &mut gpui_kit::Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let (muted, danger) = (theme.muted_foreground, theme.danger);
    let limit = i18n::size(Some(ramflux_desktop_sdk::ATTACHMENT_LIMIT as u64));
    let (name, detail, error): (&str, SharedString, bool) = match staged {
        Staged::Ready { name, size, .. } => (name, i18n::size(Some(*size)).into(), false),
        Staged::TooLarge { name, size } => (
            name,
            tr_args(
                "composer.attach.too_large",
                &[("size", &i18n::size(Some(*size))), ("limit", &limit)],
            ),
            true,
        ),
        Staged::Unreadable { name, reason } => {
            (name, tr_args("composer.attach.unreadable", &[("reason", reason)]), true)
        }
        Staged::Sent { name, .. } => (name, tr("composer.attach.sending"), false),
        Staged::Failed { name, reason } => {
            (name, tr_args("composer.attach.failed", &[("reason", reason)]), true)
        }
    };
    let removable = !matches!(staged, Staged::Sent { .. });
    h_flex()
        .id("composer-attachment")
        .role(Role::Status)
        .aria_label(SharedString::from(format!("{name}: {detail}")))
        .test_support()
        .gap_2()
        .px_2()
        .py_1()
        .border_1()
        .border_color(if error { danger } else { theme.border })
        .rounded(theme.radius)
        .child(Icon::new(IconName::File).small().text_color(muted))
        .child(div().text_sm().min_w_0().truncate().child(name.to_owned()))
        .child(
            div()
                .id("composer-attachment-detail")
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(if error { danger } else { muted })
                .child(detail)
                .test_support(),
        )
        .when(removable, |row| {
            row.child(
                Button::new("composer-attachment-remove")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .accessibility_label(tr("composer.attach.remove"))
                    .tooltip(tr("composer.attach.remove"))
                    .on_click(on_remove),
            )
        })
        .into_any_element()
}
