// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Diagnostics: a preview of the redacted export (daemon state,
//! node endpoints and health, counts) and the export itself. The preview is
//! built from the store, which never holds keys, tokens, nonces or message
//! plaintext; identifiers that could link people are reduced to counts.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, div};
use ramflux_desktop_core::{Store, UiEventKind};

use super::{SectionContext, SettingsSection};
use crate::i18n::tr;

/// Diagnostics path offered until a native file dialog is wired.
pub const DIAGNOSTICS_PATH: &str = "~/Documents/ramflux-diagnostics.txt";

/// The redacted export preview, one `key: value` per line.
pub fn preview(store: &Store) -> String {
    let mut lines = vec![
        format!("source: {:?}", store.source_status()),
        format!("projection.seq: {}", store.last_seq()),
        format!("projection.rejected: {}", store.rejections().len()),
        format!("accounts: {}", store.accounts().count()),
        format!("devices: {}", store.devices().filter(|d| !d.revoked).count()),
        format!("contexts: {}", store.contexts().count()),
        format!("moments: {}", store.moments().count()),
    ];
    for node in store.nodes() {
        lines.push(format!(
            "node {}: health={:?} pin={:?} endpoints={} key=[redacted]",
            node.id,
            node.health,
            node.pin_source,
            node.endpoints.join(",")
        ));
    }
    lines.push("secrets: [redacted]".to_owned());
    lines.join("\n")
}

/// The diagnostics section.
pub fn page(context: &SectionContext, cx: &App) -> SettingPage {
    let text = preview(context.store.read(cx).store());
    let sink = context.sink(SettingsSection::Diagnostics);
    let item = SettingItem::render(move |_, _, cx: &mut App| {
        let sink = sink.clone();
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("settings.diagnostics.explain")),
            )
            .child(
                div()
                    .id("diagnostics-preview")
                    .p_2()
                    .rounded(cx.theme().radius)
                    .bg(cx.theme().muted)
                    .text_xs()
                    .font_family(cx.theme().mono_font_family.clone())
                    .children(text.lines().map(|line| div().child(line.to_owned())))
                    .test_support(),
            )
            .child(
                h_flex().child(
                    Button::new("diagnostics-export")
                        .small()
                        .outline()
                        .label(tr("settings.diagnostics.export"))
                        .on_click(move |_, window, cx| {
                            sink.emit(
                                UiEventKind::DiagnosticsExportRequested {
                                    path: DIAGNOSTICS_PATH.to_owned(),
                                },
                                window,
                                cx,
                            );
                        }),
                ),
            )
            .into_any_element()
    });
    SettingPage::new(tr("settings.section.diagnostics")).group(SettingGroup::new().item(item))
}

#[cfg(test)]
mod tests {
    use ramflux_desktop_test::{ScenarioPlayer, load_fixture};

    use super::*;

    #[test]
    fn the_preview_redacts_keys_and_names() {
        let mut player = ScenarioPlayer::new(&load_fixture("s09_multi_account_day").expect("s09"));
        player.advance(1_000);
        let text = preview(player.store());
        assert!(text.contains("node n-corp"));
        assert!(!text.contains("SHA256"), "fingerprints are not exported");
        assert!(!text.contains("Alex"), "names are not exported");
        assert!(text.contains("[redacted]"));
    }
}
