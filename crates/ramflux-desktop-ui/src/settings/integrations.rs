// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · MCP and A2I (view-only): registered MCP servers with their
//! tools and health, and pending A2I control events. Registration, tool
//! calls and acknowledgements stay in the command line by design.

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, div};
use ramflux_desktop_core::Integrations;

use super::{SectionContext, note};
use crate::i18n::tr;
use crate::ids::domain_id;

/// The integrations section.
pub fn page(context: &SectionContext, cx: &App) -> SettingPage {
    let integrations: Integrations = context.store.read(cx).store().integrations().clone();
    let servers = integrations.mcp_servers.clone();
    let mcp = SettingItem::render(move |_, _, cx: &mut App| {
        if servers.is_empty() {
            return note("settings.integrations.no_servers", cx);
        }
        v_flex()
            .id("settings-mcp")
            .gap_2()
            .children(servers.iter().map(|server| {
                h_flex()
                    .id(domain_id("mcp", &server.name))
                    .gap_2()
                    .child(div().text_sm().font_medium().child(server.name.clone()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(server.tools.join(", ")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if server.healthy {
                                cx.theme().success
                            } else {
                                cx.theme().danger
                            })
                            .child(tr(if server.healthy {
                                "settings.integrations.healthy"
                            } else {
                                "settings.integrations.unhealthy"
                            })),
                    )
            }))
            .into_any_element()
    });
    let pending = integrations.a2i_pending.clone();
    let a2i = SettingItem::render(move |_, _, cx: &mut App| {
        if pending.is_empty() {
            return note("settings.integrations.no_a2i", cx);
        }
        v_flex()
            .id("settings-a2i")
            .gap_1()
            .children(pending.iter().map(|item| {
                h_flex()
                    .gap_2()
                    .text_sm()
                    .child(
                        div()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_xs()
                            .child(item.id.clone()),
                    )
                    .child(div().child(item.summary.clone()))
            }))
            .into_any_element()
    });
    let readonly = SettingItem::render(|_, _, cx: &mut App| note("settings.readonly", cx));
    SettingPage::new(tr("settings.section.integrations"))
        .group(SettingGroup::new().item(readonly))
        .group(SettingGroup::new().title(tr("settings.integrations.mcp")).item(mcp))
        .group(SettingGroup::new().title(tr("settings.integrations.a2i")).item(a2i))
}
