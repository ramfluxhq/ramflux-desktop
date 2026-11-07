// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The text of a message with its formatted spans.
//!
//! Span offsets count Unicode scalar values (never UTF-8 bytes or UTF-16
//! units), so the byte range of a span is found by walking the scalars of the
//! text. Overlapping spans combine their styles. A span has no action: a
//! link is underlined and nothing more.

use std::ops::Range;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, FontStyle, FontWeight, HighlightStyle, IntoElement, SharedString, StrikethroughStyle,
    StyledText, TestSupportExt as _, UnderlineStyle, div, px,
};
use ramflux_desktop_core::{SpanKind, TextSpan};

use crate::i18n::{tr, tr_args};

/// How many scalars of a replied-to message a reply row quotes.
const EXCERPT_SCALARS: usize = 80;

/// The start of `text` for a reply row: its first line, shortened.
pub fn excerpt(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    let mut scalars = line.chars();
    let head: String = scalars.by_ref().take(EXCERPT_SCALARS).collect();
    if scalars.next().is_some() { format!("{head}\u{2026}") } else { head }
}

/// The byte range of `span` in `text`, or `None` when it leaves the text.
pub fn byte_range(text: &str, span: &TextSpan) -> Option<Range<usize>> {
    let starts: Vec<usize> =
        text.char_indices().map(|(at, _)| at).chain(std::iter::once(text.len())).collect();
    let start = *starts.get(usize::try_from(span.offset).ok()?)?;
    let end = *starts.get(usize::try_from(span.offset.checked_add(span.length)?).ok()?)?;
    Some(start..end)
}

fn kind_key(kind: &SpanKind) -> &'static str {
    match kind {
        SpanKind::Bold => "moment.message.span.bold",
        SpanKind::Italic => "moment.message.span.italic",
        SpanKind::Underline => "moment.message.span.underline",
        SpanKind::Strikethrough => "moment.message.span.strikethrough",
        SpanKind::Code => "moment.message.span.code",
        SpanKind::Spoiler => "moment.message.span.spoiler",
        SpanKind::Mention { .. } => "moment.message.span.mention",
        SpanKind::Link { .. } => "moment.message.span.link",
    }
}

fn style_of(kind: &SpanKind, cx: &App) -> HighlightStyle {
    let theme = cx.theme();
    let line = |color| UnderlineStyle { thickness: px(1.), color: Some(color), wavy: false };
    match kind {
        SpanKind::Bold => {
            HighlightStyle { font_weight: Some(FontWeight::BOLD), ..Default::default() }
        }
        SpanKind::Italic => {
            HighlightStyle { font_style: Some(FontStyle::Italic), ..Default::default() }
        }
        SpanKind::Underline => {
            HighlightStyle { underline: Some(line(theme.foreground)), ..Default::default() }
        }
        SpanKind::Strikethrough => HighlightStyle {
            strikethrough: Some(StrikethroughStyle {
                thickness: px(1.),
                color: Some(theme.foreground),
            }),
            ..Default::default()
        },
        SpanKind::Code => {
            HighlightStyle { background_color: Some(theme.muted), ..Default::default() }
        }
        SpanKind::Spoiler => HighlightStyle {
            color: Some(theme.muted),
            background_color: Some(theme.muted),
            ..Default::default()
        },
        SpanKind::Mention { .. } => HighlightStyle {
            color: Some(crate::tone::text(theme.primary, cx)),
            font_weight: Some(FontWeight::MEDIUM),
            ..Default::default()
        },
        SpanKind::Link { .. } => HighlightStyle {
            color: Some(crate::tone::text(theme.primary, cx)),
            underline: Some(line(crate::tone::text(theme.primary, cx))),
            ..Default::default()
        },
    }
}

fn merged(into: &mut HighlightStyle, over: HighlightStyle) {
    into.color = over.color.or(into.color);
    into.font_weight = over.font_weight.or(into.font_weight);
    into.font_style = over.font_style.or(into.font_style);
    into.background_color = over.background_color.or(into.background_color);
    into.underline = over.underline.or(into.underline.take());
    into.strikethrough = over.strikethrough.or(into.strikethrough.take());
}

/// Non-overlapping styled runs: where spans overlap their styles combine.
fn highlights(
    ranges: &[(Range<usize>, &SpanKind)],
    cx: &App,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut cuts: Vec<usize> = ranges.iter().flat_map(|(r, _)| [r.start, r.end]).collect();
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .filter_map(|pair| {
            let run = pair[0]..pair[1];
            let mut style = HighlightStyle::default();
            let mut any = false;
            for (range, kind) in ranges {
                if range.start <= run.start && run.end <= range.end {
                    merged(&mut style, style_of(kind, cx));
                    any = true;
                }
            }
            any.then_some((run, style))
        })
        .collect()
}

/// Renders `text` with `spans`. Each span also gets a marker that names its
/// kind and the exact text it covers, for assistive technology and tests.
pub fn render(text: &SharedString, spans: &[TextSpan], cx: &App) -> impl IntoElement {
    let ranges: Vec<(Range<usize>, &SpanKind)> = spans
        .iter()
        .filter_map(|span| byte_range(text, span).map(|range| (range, &span.kind)))
        .collect();
    let markers: Vec<_> = ranges
        .iter()
        .enumerate()
        .map(|(index, (range, kind))| {
            let label = tr_args(
                "moment.a11y.span",
                &[("kind", &tr(kind_key(kind))), ("text", &text[range.clone()])],
            );
            div()
                .id(SharedString::from(format!("message-span-{index}")))
                .aria_label(label)
                .size_0()
                .test_support()
        })
        .collect();
    div()
        .id("message-text")
        .child(StyledText::new(text.clone()).with_highlights(highlights(&ranges, cx)))
        .children(markers)
}
