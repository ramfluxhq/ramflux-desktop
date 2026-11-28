// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The conversation chrome of an MLS group: the header (name, member count,
//! Refresh, Info and Members), the banner that replaces the composer while
//! the group is read only (S13), and the strip of the members the next post
//! mentions.
//!
//! The header has none of the old group menu. Refresh asks the group host
//! for news now (`group.refresh`, at most once every five seconds per group,
//! enforced by the policy).

use crate::style::DesignButton as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Entity, IntoElement, Role, SharedString, div};
use ramflux_desktop_core::mls_group::{self as mls, ReadOnly};
use ramflux_desktop_core::{Context, Store, UiEventKind};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_secondary};

use super::{group_name, picked_mentions};
use crate::app::StoreHandle;
use crate::bubble::EventSink;
use crate::i18n::{tr, tr_count};

/// A member picked for the next post's mentions (the chip shows the name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionPick {
    /// The name on the chip.
    pub name: SharedString,
}

/// The header of an MLS group conversation.
pub fn header(
    store: &Store,
    context: &Context,
    sink: EventSink,
    tools: Option<AnyElement>,
    cx: &App,
) -> AnyElement {
    let palette = ramflux_desktop_navigation::palette(cx);
    let group_id = mls::group_of(&context.id).unwrap_or_default().to_owned();
    let summary = store.mls_group(&group_id);
    let read_only = store.mls_read_only(&group_id).is_some();
    let members = summary.map(|g| tr_count("group.info.members", u64::from(g.members)));
    let refresh = {
        let sink = sink.clone();
        let group = group_id.clone();
        Button::new("mls-header-refresh")
            .ghost()
            .small()
            .icon(IconName::RefreshCw)
            .accessibility_label(tr("group.info.refresh"))
            .tooltip(tr("group.info.refresh"))
            .disabled(read_only)
            .on_click(move |_, window, cx| {
                sink.emit(UiEventKind::MlsRefreshPressed { group_id: group.clone() }, window, cx);
            })
    };
    let panel = |id: &'static str, icon: IconName, key: &'static str, panel: mls::MlsPanel| {
        let sink = sink.clone();
        Button::new(id)
            .ghost()
            .small()
            .icon(icon)
            .accessibility_label(tr(key))
            .tooltip(tr(key))
            .on_click(move |_, window, cx| {
                sink.emit(UiEventKind::MlsPanelRequested { panel: panel.clone() }, window, cx);
            })
    };
    h_flex()
        .id("conversation-header")
        .test_support()
        .gap(space(3.))
        .px(gpui_kit::rems(32.0 / 15.0))
        .py(gpui_kit::rems(20.0 / 15.0))
        .min_h(gpui_kit::rems(88.0 / 15.0))
        .border_b_1()
        .border_color(palette.line)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .id("conversation-title")
                        .test_support()
                        .truncate()
                        .text_size(gpui_kit::rems(palette.size_subheading / 15.0))
                        .font_semibold()
                        .text_color(palette.text)
                        .child(group_name(&context.title)),
                )
                .children(members.map(|text| {
                    div()
                        .id("mls-header-members")
                        .test_support()
                        .text_size(gpui_kit::rems(palette.size_secondary / 15.0))
                        .text_color(cx.theme().muted_foreground)
                        .child(text)
                })),
        )
        .child(refresh)
        .child(panel(
            "mls-header-members-button",
            IconName::Users,
            "group.info.members_entry",
            mls::MlsPanel::Members { group_id: group_id.clone() },
        ))
        .child(panel(
            "mls-header-info",
            IconName::Info,
            "group.info.title",
            mls::MlsPanel::Info { group_id },
        ))
        .children(tools)
        .into_any_element()
}

/// The banner that replaces the composer while the group takes no posts.
pub fn read_only_banner(store: &Store, group_id: &str, cx: &App) -> Option<AnyElement> {
    let reason = store.mls_read_only(group_id)?;
    let key = match reason {
        ReadOnly::Left => "group.leave.done",
        ReadOnly::Removed => "group.state.removed",
        ReadOnly::Banned => "group.state.banned",
    };
    let palette = ramflux_desktop_navigation::palette(cx);
    Some(
        div()
            .id("mls-read-only")
            .role(Role::Status)
            .test_support()
            .w_full()
            .px(gpui_kit::rems(24.0 / 15.0))
            .py(space(3.))
            .border_t_1()
            .border_color(palette.line)
            .text_size(text_secondary())
            .text_color(cx.theme().muted_foreground)
            .child(tr(key))
            .into_any_element(),
    )
}

/// The strip above the composer: the members the next post mentions, and the
/// button that opens the picker. `None` while the group takes no posts.
pub fn mention_strip(
    store: &Store,
    handle: Entity<StoreHandle>,
    group_id: &str,
    sink: EventSink,
    cx: &App,
) -> Option<AnyElement> {
    if store.mls_read_only(group_id).is_some() {
        return None;
    }
    let picked = picked_mentions(store, group_id);
    let group = group_id.to_owned();
    let open = {
        let sink = sink.clone();
        let group = group.clone();
        Button::new("mls-mention-open")
            .ghost()
            .small()
            .icon(IconName::AtSign)
            .accessibility_label(tr("group.members.mention"))
            .tooltip(tr("group.members.mention"))
            .on_click(move |_, window, cx| {
                sink.emit(
                    UiEventKind::MlsPanelRequested {
                        panel: mls::MlsPanel::Mentions { group_id: group.clone() },
                    },
                    window,
                    cx,
                );
            })
    };
    Some(
        h_flex()
            .id("mls-mention-strip")
            .test_support()
            .w_full()
            .px(gpui_kit::rems(24.0 / 15.0))
            .gap(space(1.))
            .flex_wrap()
            .child(open)
            .children(picked.into_iter().map(|(anchor, pick)| {
                let group = group.clone();
                let handle = handle.clone();
                Button::new(SharedString::from(format!("mls-mention-chip-{anchor}")))
                    .design_quiet(cx)
                    .design_label(pick.name.clone())
                    .tooltip(pick.name.clone())
                    .on_click(move |_, _, cx| {
                        handle.read(cx).store().mls_toggle_mention(&group, &anchor, &pick.name);
                        cx.refresh_windows();
                    })
            }))
            .into_any_element(),
    )
}
