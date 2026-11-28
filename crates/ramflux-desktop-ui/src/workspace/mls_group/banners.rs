// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The banners above the timeline of an MLS group (derived designs D-M3-2 and
//! D-M3-3): the catch-up banner and the warning banners.
//!
//! - **Catch-up.** While it runs the banner shows the numbers (the plain text
//!   when the total is 0) and Skip, which opens the skip dialog: the only
//!   path to `skip: true` (A3). While the skip call runs the banner reads
//!   "Skipping…". While the device rejoins (the state, or a `rejoin_needed`
//!   warning) the banner has no action; its text follows the reason.
//! - **Warnings.** At most two banners show, in severity order (critical
//!   first); a "+N" control opens the rest in the group info. `fork_alarm`,
//!   `rejoin_needed` and `head_commit_rejected` cannot be hidden; the other
//!   three can, for the session. `undecryptable` has Show (it brings the
//!   placeholder row into view through the older-page path), and
//!   `head_commit_rejected` has "Undo the change", which opens the same
//!   confirmation as the row entry. Nothing here calls anything by itself
//!   (A2).
//!
//! Every banner of a read-only group is gone: the store forgets the catch-up
//! record and the warnings of a group the account left or lost.

use crate::style::DesignButton as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Entity, IntoElement, Role, SharedString, div};
use ramflux_desktop_core::mls_group::{CatchUpState, MlsPanel, WarningCode};
use ramflux_desktop_core::{Store, UiEventKind};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_secondary};

use crate::app::StoreHandle;
use crate::bubble::EventSink;
use crate::i18n::{tr, tr_count};

/// Banners shown at once.
const SHOWN: usize = 2;

/// How serious a banner is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tone {
    Critical,
    Warning,
    Info,
}

fn tone_of(code: &WarningCode) -> Tone {
    match code.banner_rank() {
        Some(0..=2) => Tone::Critical,
        Some(3 | 4) => Tone::Warning,
        _ => Tone::Info,
    }
}

/// The text of a warning (the banner and the row of the group info say the
/// same). An unknown code reads generic.
pub fn warning_text(code: &WarningCode) -> SharedString {
    match code {
        WarningCode::ForkAlarm { .. } => tr("group.warning.fork_alarm"),
        WarningCode::RejoinNeeded { reason } => tr(reason.key()),
        WarningCode::Undecryptable { count, .. } => tr_count("group.warning.undecryptable", *count),
        WarningCode::AdmissionNeedsOnline => tr("group.warning.admission_needs_online"),
        WarningCode::HostUnusual { .. } => tr("group.warning.host_unusual"),
        WarningCode::HeadCommitRejected { .. } => tr("group.warning.head_commit_rejected"),
        WarningCode::HostIpVisible { .. } | WarningCode::Unknown(_) => tr("group.warning.generic"),
    }
}

fn card(id: &'static str, tone: Tone, cx: &App) -> gpui_kit::Stateful<gpui_kit::Div> {
    let palette = ramflux_desktop_navigation::palette(cx);
    let color = match tone {
        Tone::Critical => crate::tone::text(cx.theme().danger, cx),
        Tone::Warning | Tone::Info => cx.theme().muted_foreground,
    };
    h_flex()
        .id(id)
        .w_full()
        .gap(space(2.))
        .px(gpui_kit::rems(24.0 / 15.0))
        .py(space(2.))
        .border_b_1()
        .border_color(palette.line)
        .text_size(text_secondary())
        .text_color(color)
}

/// The banners of a group, above the timeline; `None` when there are none.
pub fn banners(
    store: &Store,
    handle: Entity<StoreHandle>,
    group_id: &str,
    sink: EventSink,
    cx: &App,
) -> Option<AnyElement> {
    if store.mls_read_only(group_id).is_some() {
        return None;
    }
    let mut rows: Vec<AnyElement> = Vec::new();
    let rejoin = store.mls_warnings(group_id).into_iter().find_map(|w| match w {
        WarningCode::RejoinNeeded { reason } => Some(reason.key()),
        _ => None,
    });
    if let Some(record) = store.mls_catch_up(group_id) {
        match record.state {
            CatchUpState::Running => {
                let text = if record.total > 0 {
                    crate::i18n::tr_args(
                        "group.catchup.running",
                        &[("done", &record.done.to_string()), ("total", &record.total.to_string())],
                    )
                } else {
                    tr("group.catchup.running_indeterminate")
                };
                let skipping = record.skipping.is_some();
                let failure = record
                    .skip_failure
                    .as_deref()
                    .map(|c| tr(ramflux_desktop_core::mls_group::error_key(c)));
                let group = group_id.to_owned();
                let sink = sink.clone();
                rows.push(
                    card("mls-banner-catchup", Tone::Info, cx)
                        .role(Role::Status)
                        .aria_label(text.clone())
                        .child(Icon::new(IconName::RefreshCw).xsmall())
                        .child(v_flex().flex_1().min_w_0().child(div().child(text)).children(
                            failure.map(|text| {
                                div().id("mls-banner-catchup-error").role(Role::Alert).child(text)
                            }),
                        ))
                        .child(if skipping {
                            div()
                                .id("mls-banner-skipping")
                                .child(tr("group.skip.skipping"))
                                .into_any_element()
                        } else {
                            Button::new("mls-banner-skip")
                                .design_secondary(cx)
                                .design_label(tr("group.catchup.skip"))
                                .on_click(move |_, window, cx| {
                                    sink.emit(
                                        UiEventKind::MlsPanelRequested {
                                            panel: MlsPanel::ConfirmSkip {
                                                group_id: group.clone(),
                                            },
                                        },
                                        window,
                                        cx,
                                    );
                                })
                                .into_any_element()
                        })
                        .into_any_element(),
                );
            }
            CatchUpState::Rejoining => {
                let text = tr(rejoin.unwrap_or("group.catchup.rejoining"));
                rows.push(
                    card("mls-banner-rejoining", Tone::Critical, cx)
                        .role(Role::Status)
                        .aria_label(text.clone())
                        .child(Icon::new(IconName::RefreshCw).xsmall())
                        .child(div().flex_1().min_w_0().child(text))
                        .into_any_element(),
                );
            }
            CatchUpState::Idle => {}
        }
    }
    // A `rejoin_needed` warning with no rejoining state still reads as a
    // rejoining banner (its text follows the reason).
    if rows.is_empty()
        && let Some(key) = rejoin
    {
        let text = tr(key);
        rows.push(
            card("mls-banner-rejoining", Tone::Critical, cx)
                .role(Role::Status)
                .aria_label(text.clone())
                .child(Icon::new(IconName::RefreshCw).xsmall())
                .child(div().flex_1().min_w_0().child(text))
                .into_any_element(),
        );
    }
    let warnings: Vec<&WarningCode> = store
        .mls_banner_warnings(group_id)
        .into_iter()
        .filter(|w| !matches!(w, WarningCode::RejoinNeeded { .. }))
        .collect();
    let more = warnings.len().saturating_sub(SHOWN);
    for (ix, code) in warnings.iter().take(SHOWN).enumerate() {
        let tone = tone_of(code);
        let text = warning_text(code);
        let key = code.code().to_owned();
        let group = group_id.to_owned();
        let action: Option<AnyElement> = match code {
            WarningCode::Undecryptable { .. } => {
                let sink = sink.clone();
                let group = group.clone();
                Some(
                    Button::new(gpui_kit::SharedString::from(format!("mls-banner-show-{ix}")))
                        .design_secondary(cx)
                        .design_label(tr("group.warning.undecryptable.show"))
                        .on_click(move |_, window, cx| {
                            sink.emit(
                                UiEventKind::MlsPanelRequested {
                                    panel: MlsPanel::Unreadable { group_id: group.clone() },
                                },
                                window,
                                cx,
                            );
                        })
                        .into_any_element(),
                )
            }
            WarningCode::HeadCommitRejected { log_seq } => {
                let sink = sink.clone();
                let group = group.clone();
                let log_seq = *log_seq;
                Some(
                    Button::new(gpui_kit::SharedString::from(format!("mls-banner-undo-{ix}")))
                        .design_secondary(cx)
                        .design_label(tr("group.warning.head_commit_rejected.action"))
                        .on_click(move |_, window, cx| {
                            sink.emit(
                                UiEventKind::MlsPanelRequested {
                                    panel: MlsPanel::ConfirmVoid {
                                        group_id: group.clone(),
                                        log_seq,
                                    },
                                },
                                window,
                                cx,
                            );
                        })
                        .into_any_element(),
                )
            }
            _ => None,
        };
        let hide = code.hideable().then(|| {
            let handle = handle.clone();
            let group = group.clone();
            let key = key.clone();
            Button::new(gpui_kit::SharedString::from(format!("mls-banner-hide-{ix}")))
                .design_quiet(cx)
                .design_label(tr("group.warning.hide"))
                .on_click(move |_, _, cx| {
                    handle.read(cx).store().mls_hide_banner(&group, &key);
                    cx.refresh_windows();
                })
        });
        rows.push(
            card(
                match ix {
                    0 => "mls-banner-0",
                    _ => "mls-banner-1",
                },
                tone,
                cx,
            )
            .role(if tone == Tone::Critical { Role::Alert } else { Role::Status })
            .aria_label(text.clone())
            .child(
                Icon::new(match tone {
                    Tone::Critical => IconName::TriangleAlert,
                    Tone::Warning | Tone::Info => IconName::Info,
                })
                .xsmall(),
            )
            .child(div().flex_1().min_w_0().child(text))
            .children(action)
            .children(hide)
            .into_any_element(),
        );
    }
    if more > 0 {
        let group = group_id.to_owned();
        let sink = sink.clone();
        rows.push(
            card("mls-banner-more", Tone::Info, cx)
                .child(div().flex_1().child(tr_count("group.warning.more", more as u64)))
                .child(
                    Button::new("mls-banner-more-open")
                        .design_quiet(cx)
                        .design_label(tr("group.info.title"))
                        .on_click(move |_, window, cx| {
                            sink.emit(
                                UiEventKind::MlsPanelRequested {
                                    panel: MlsPanel::Info { group_id: group.clone() },
                                },
                                window,
                                cx,
                            );
                        }),
                )
                .into_any_element(),
        );
    }
    (!rows.is_empty()).then(|| {
        v_flex().id("mls-banners").test_support().w_full().children(rows).into_any_element()
    })
}
