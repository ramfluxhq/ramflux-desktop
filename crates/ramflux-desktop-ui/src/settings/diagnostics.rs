// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Diagnostics: a preview of the redacted export (daemon state,
//! node endpoints and health, counts) and the export itself. The preview is
//! built from the store, which never holds keys, tokens, nonces or message
//! plaintext; identifiers that could link people are reduced to counts.
//!
//! The local setup groups ([`super::local_setup`]) follow, with the message
//! sync group ([`super::sync`]: how the account is kept up to date) after the
//! local daemon.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, div};
use ramflux_desktop_core::{Store, UiEventKind};
use ramflux_desktop_navigation::direction::{DirectionalExt as _, h_flex};
use ramflux_desktop_navigation::scale::{space, text_caption};

use super::local_setup::{self, LaunchInputs};
use super::{SectionContext, SettingsSection};
use crate::file_dialog::{self, StartDir};
use crate::i18n::tr;
use crate::style::DesignButton as _;

/// File name suggested for the diagnostics export.
pub const DIAGNOSTICS_FILE_NAME: &str = "ramflux-diagnostics.txt";

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
    if let Some(sync) = store.sync_status() {
        lines.push(format!("sync.mode: {:?}", sync.mode));
        if let Some(cursor) = sync.cursor {
            lines.push(format!("sync.cursor: {cursor}"));
        }
        if let Some(state) = sync.daemon_sync {
            lines.push(format!("sync.daemon: {state:?}"));
        }
        if let Some(unrouted) = sync.unrouted {
            lines.push(format!("sync.unrouted: {unrouted}"));
        }
    }
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

fn mono_box(id: &'static str, text: String, cx: &App) -> gpui_kit::AnyElement {
    div()
        .id(id)
        .p(space(2.))
        .rounded(cx.theme().radius)
        .bg(cx.theme().muted)
        .text_size(text_caption())
        .font_family(cx.theme().mono_font_family.clone())
        // Diagnostics read left to right in every interface direction.
        .text_ltr()
        .children(text.lines().map(|line| div().child(line.to_owned())))
        .test_support()
        .into_any_element()
}

/// The diagnostics section: the export, then the
/// local setup, the local daemon, version and logs.
pub fn page(context: &SectionContext, inputs: &LaunchInputs, cx: &App) -> SettingPage {
    let [setup, daemon, about] = local_setup::groups(context, inputs, cx);
    let text = preview(context.store.read(cx).store());
    let sink = context.sink(SettingsSection::Diagnostics);
    let item = SettingItem::render(move |_, _, cx: &mut App| {
        let sink = sink.clone();
        v_flex()
            .gap(space(2.))
            .child(
                div()
                    .text_size(text_caption())
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("settings.diagnostics.explain")),
            )
            .child(mono_box("diagnostics-preview", text.clone(), cx))
            .child(
                h_flex().child(
                    Button::new("diagnostics-export")
                        .small()
                        .design_secondary(cx)
                        .design_label(tr("settings.diagnostics.export"))
                        .on_click(move |_, window, cx| {
                            let sink = sink.clone();
                            file_dialog::save_as(
                                StartDir::Documents,
                                DIAGNOSTICS_FILE_NAME,
                                window,
                                cx,
                                move |path, window, cx| {
                                    sink.emit(
                                        UiEventKind::DiagnosticsExportRequested { path },
                                        window,
                                        cx,
                                    );
                                },
                            );
                        }),
                ),
            )
            .into_any_element()
    });
    let mut page = SettingPage::new(tr("settings.section.diagnostics"))
        .group(SettingGroup::new().title(tr("settings.diagnostics.export_title")).item(item))
        .group(setup)
        .group(daemon);
    if let Some(sync) = super::sync::group(context, cx) {
        page = page.group(sync);
    }
    let page = page.group(about);
    // Debug builds with the demo scenarios: the developer panel opens from
    // here (its old title-bar button has no place on the boards).
    if cfg!(all(debug_assertions, feature = "demo")) {
        page.group(SettingGroup::new().title(tr("status.dev_panel")).item(SettingItem::render(
            |_, _, cx: &mut App| {
                crate::style::secondary_button("diagnostics-dev-panel", cx)
                    .label(tr("settings.diagnostics.dev_panel"))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(crate::actions::ToggleDevPanel), cx)
                    })
                    .into_any_element()
            },
        )))
    } else {
        page
    }
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
