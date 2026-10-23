// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Status blocks: progress, streaming log lines, and warning / error /
//! success callouts.

use gpui_kit::component::alert::Alert;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, IntoElement, div};
use ramflux_desktop_core::{BlockId, BlockKind, JsonValue};

use crate::ids::domain_id;

/// Lines of a log stream shown at once (the newest).
pub const LOG_LINES: usize = 8;

/// A progress bar from `value` in 0..=1 with a `label`.
pub fn progress(id: &BlockId, data: &JsonValue, cx: &App) -> Option<AnyElement> {
    let value = data.get("value")?.as_f64()?;
    let label = data.get("label").and_then(JsonValue::as_str).unwrap_or_default().to_owned();
    // Lossy by design: progress is only displayed.
    #[allow(clippy::cast_possible_truncation)]
    let percent = (value.clamp(0.0, 1.0) * 100.0) as f32;
    Some(
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .justify_between()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(label.clone())
                    .child(format!("{percent:.0}%")),
            )
            .child(
                Progress::new(domain_id("progress", id)).value(percent).accessibility_label(label),
            )
            .into_any_element(),
    )
}

/// The newest lines of a log, newline-separated in `text` (streams through
/// `AppendText`).
pub fn log_stream(data: &JsonValue, cx: &App) -> Option<AnyElement> {
    let text = data.get("text")?.as_str()?;
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let skip = lines.len().saturating_sub(LOG_LINES);
    Some(
        v_flex()
            .p_2()
            .rounded(cx.theme().radius)
            .bg(cx.theme().muted)
            .text_xs()
            .font_family(cx.theme().mono_font_family.clone())
            .text_color(cx.theme().foreground)
            .children(lines.into_iter().skip(skip).map(|line| div().child(line)))
            .into_any_element(),
    )
}

/// A warning, error or success callout from `text` and an optional `title`.
pub fn callout(id: &BlockId, kind: BlockKind, data: &JsonValue) -> Option<AnyElement> {
    let text = data.get("text")?.as_str()?.to_owned();
    let title = data.get("title").and_then(JsonValue::as_str).map(str::to_owned);
    let id = domain_id("callout", id);
    let alert = match kind {
        BlockKind::Warning => Alert::warning(id, text),
        BlockKind::Error => Alert::error(id, text),
        _ => Alert::success(id, text),
    };
    Some(alert.when_some(title, Alert::title).into_any_element())
}
