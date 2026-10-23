// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Diagnostics: a preview of the redacted export (daemon state,
//! node endpoints and health, counts) and the export itself. The preview is
//! built from the store, which never holds keys, tokens, nonces or message
//! plaintext; identifiers that could link people are reduced to counts.
//!
//! A read-only group shows the renderer profile this client
//! declares to its source and the renderer errors found this session, and
//! a note says what the daemon does not deliver yet (messages in
//! conversations this device does not know arrive with the daemon's
//! background sync). The local setup groups ([`super::local_setup`]) follow.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, div};
use ramflux_desktop_core::{RendererProfile, Store, UiEventKind};

use super::local_setup::{self, LaunchInputs};
use super::{SectionContext, SettingsSection};
use crate::app::RendererErrors;
use crate::file_dialog::{self, StartDir};
use crate::i18n::{tr, tr_args};
use crate::moments::surface::registry;

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

/// The renderer profile and error counts, one `key: value` per line.
/// Errors are counted by kind; no block data or ids are shown.
pub fn renderer_summary(profile: &RendererProfile, errors: Option<&RendererErrors>) -> String {
    let blocks: Vec<String> = profile
        .blocks
        .iter()
        .map(|b| {
            let versions: Vec<String> = b.versions.iter().map(|v| format!("v{v}")).collect();
            format!("{:?}({})", b.kind, versions.join("/"))
        })
        .collect();
    let limits = &profile.limits;
    let mut lines = vec![
        format!("device: {:?}, runtime: {}", profile.device_class, profile.runtime),
        format!("input: {:?}, output: {:?}", profile.input, profile.output),
        format!("surface modes: {:?}", profile.surface_modes),
        format!("blocks: {}", blocks.join(", ")),
        format!(
            "limits: blocks={} actions={} text={} data_bytes={}",
            limits.max_blocks, limits.max_actions, limits.max_text_chars, limits.max_data_bytes
        ),
    ];
    if let Some(errors) = errors {
        for (kind, count) in errors.counts() {
            lines.push(format!("renderer.errors.{kind}: {count}"));
        }
    }
    lines.join("\n")
}

fn mono_box(id: &'static str, text: String, cx: &App) -> gpui_kit::AnyElement {
    div()
        .id(id)
        .p_2()
        .rounded(cx.theme().radius)
        .bg(cx.theme().muted)
        .text_xs()
        .font_family(cx.theme().mono_font_family.clone())
        .children(text.lines().map(|line| div().child(line.to_owned())))
        .test_support()
        .into_any_element()
}

fn renderer_group(context: &SectionContext, cx: &App) -> SettingGroup {
    let handle = context.store.read(cx);
    let errors = handle.session().map(|s| s.renderer_errors());
    let total = errors.map_or(0, RendererErrors::total);
    let text = renderer_summary(registry::profile(), errors);
    let item = SettingItem::render(move |_, _, cx: &mut App| {
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("settings.diagnostics.renderer_explain")),
            )
            .child(mono_box("renderer-profile", text.clone(), cx))
            .child(div().id("renderer-error-count").text_xs().child(tr_args(
                "settings.diagnostics.renderer_errors",
                &[("count", &total.to_string())],
            )))
            .into_any_element()
    });
    SettingGroup::new().title(tr("settings.diagnostics.renderer")).item(item)
}

fn sync_group(cx: &App) -> SettingGroup {
    let muted = cx.theme().muted_foreground;
    let item = SettingItem::render(move |_, _, _: &mut App| {
        div()
            .id("diagnostics-sync-note")
            .test_support()
            .text_xs()
            .text_color(muted)
            .child(tr("settings.diagnostics.sync_note"))
            .into_any_element()
    });
    SettingGroup::new().title(tr("settings.diagnostics.sync")).item(item)
}

/// The diagnostics section: the export and the renderer profile, then the
/// local setup, the local daemon, version and logs.
pub fn page(context: &SectionContext, inputs: &LaunchInputs, cx: &App) -> SettingPage {
    let [setup, daemon, about] = local_setup::groups(context, inputs, cx);
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
            .child(mono_box("diagnostics-preview", text.clone(), cx))
            .child(
                h_flex().child(
                    Button::new("diagnostics-export")
                        .small()
                        .outline()
                        .label(tr("settings.diagnostics.export"))
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
    SettingPage::new(tr("settings.section.diagnostics"))
        .group(SettingGroup::new().title(tr("settings.diagnostics.export_title")).item(item))
        .group(renderer_group(context, cx))
        .group(sync_group(cx))
        .group(setup)
        .group(daemon)
        .group(about)
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
