// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The card's lifecycle (spec 3.4): the status line, the notices (a draft
//! the runtime discarded, a refused change, a superseded version), the
//! resync control, the fallback text, and the surface's own actions (the
//! task-step decision buttons, which the runtime does not draw).

use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, IntoElement, SharedString, Window, div};
use ramflux_desktop_core::{CardRecord, CardStatus, CardTree, SurfaceState, UiEventKind};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption};

use super::CardSnapshot;
use crate::bubble::EventSink;
use crate::i18n::tr;
use crate::ids::domain_id;
use crate::style::DesignButton as _;

/// The status line of a card: its lifecycle chip and the notices that apply.
pub fn status(
    record: &CardRecord,
    tree: &CardTree,
    snap: &CardSnapshot,
    sink: &EventSink,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let _ = window;
    let chip: Option<SharedString> = match record.status {
        CardStatus::PendingSubmit => Some(tr("moment.card.submitting")),
        CardStatus::Consumed | CardStatus::AwaitingConfirmation => {
            Some(tr("moment.card.waiting_confirmation"))
        }
        CardStatus::Stale => Some(tr("moment.card.updating")),
        _ => match tree.phase() {
            ramflux_desktop_core::CardPhase::Generating => Some(tr("moment.card.generating")),
            ramflux_desktop_core::CardPhase::Closed | ramflux_desktop_core::CardPhase::Retired => {
                Some(tr("moment.card.closed"))
            }
            ramflux_desktop_core::CardPhase::Superseded => Some(tr("moment.card.updated")),
            ramflux_desktop_core::CardPhase::Ready => None,
        },
    };
    let out_of_sync = snap.state == Some(SurfaceState::OutOfSync);
    let superseded = tree.phase() == ramflux_desktop_core::CardPhase::Superseded;
    let resync_sink = sink.clone();
    let mut col = gpui_kit::component::v_flex().w_full().gap(space(1.));
    if let Some(chip) = chip {
        col = col.child(
            div().text_size(text_caption()).text_color(cx.theme().muted_foreground).child(chip),
        );
    }
    if superseded {
        col = col.child(div().text_size(text_caption()).child(tr("moment.card.updated_body")));
    }
    if out_of_sync {
        col = col.child(
            h_flex()
                .gap(space(2.))
                .items_center()
                .child(div().text_size(text_caption()).child(tr("moment.card.out_of_sync")))
                .child(
                    Button::new(domain_id("card-resync", &snap.surface_id))
                        .small()
                        .design_secondary(cx)
                        .design_label(tr("moment.card.resync"))
                        .on_click(move |_, window, cx| {
                            resync_sink.emit(UiEventKind::SurfaceUpdateRequested, window, cx);
                        }),
                ),
        );
    }
    if record.draft_discarded {
        col = col.child(
            div()
                .text_size(text_caption())
                .text_color(cx.theme().warning)
                .child(tr("moment.card.draft_discarded")),
        );
    }
    if record.refusal.is_some() {
        col = col.child(
            div()
                .text_size(text_caption())
                .text_color(cx.theme().danger)
                .child(tr("moment.card.refused")),
        );
    }
    col.into_any_element()
}

/// The fallback text: what a card shows when the runtime does not draw it.
pub fn fallback(snap: &CardSnapshot, cx: &App) -> AnyElement {
    div()
        .w_full()
        .text_size(ramflux_desktop_navigation::scale::text_secondary())
        .text_color(cx.theme().muted_foreground)
        .child(snap.fallback.clone())
        .into_any_element()
}

/// A node kind this build does not draw.
pub fn unsupported(cx: &App) -> AnyElement {
    div()
        .text_size(text_caption())
        .text_color(cx.theme().muted_foreground)
        .child(tr("moment.card.unsupported"))
        .into_any_element()
}

/// The surface's own actions (a task step's decision), under the card.
/// They are bound to the version and hash the user saw.
pub fn surface_actions(
    snap: &CardSnapshot,
    sink: &EventSink,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let _ = window;
    if snap.actions.is_empty() {
        return div().into_any_element();
    }
    let enabled = snap.state == Some(SurfaceState::Open);
    let buttons = snap.actions.iter().map(|action| {
        let sink = sink.clone();
        let action_id = action.id.clone();
        let hash = snap.hash.clone();
        let version = snap.version;
        let disabled = !enabled || action.disabled_reason.is_some();
        let button = Button::new(domain_id("surface-action", &action.id))
            .small()
            .design_secondary(cx)
            .design_disabled(disabled, cx)
            .design_label(action.label.clone());
        if disabled {
            return button.into_any_element();
        }
        button
            .on_click(move |_, window, cx| {
                let Some(hash) = hash.clone() else { return };
                sink.emit(
                    UiEventKind::ActionInvoked {
                        action_id: action_id.clone(),
                        values: ramflux_desktop_core::JsonValue::Null,
                        surface_hash: hash,
                        version,
                    },
                    window,
                    cx,
                );
            })
            .into_any_element()
    });
    h_flex().flex_wrap().gap(space(1.)).children(buttons).into_any_element()
}
