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

/// How many spans get an assistive-technology marker.
const MAX_MARKERS: usize = 64;
/// How many scalars of the covered text a marker quotes.
const MARKER_SCALARS: usize = 80;

/// Byte offset of every scalar of `text`, plus its length: built once per
/// render, so a span costs two lookups, not a walk of the text.
fn scalar_starts(text: &str) -> Vec<usize> {
    text.char_indices().map(|(at, _)| at).chain(std::iter::once(text.len())).collect()
}

/// The byte range of `span` in a text with the scalar index `starts`, or
/// `None` when it leaves the text.
fn byte_range(starts: &[usize], span: &TextSpan) -> Option<Range<usize>> {
    let start = *starts.get(usize::try_from(span.offset).ok()?)?;
    let end = *starts.get(usize::try_from(span.offset.checked_add(span.length)?).ok()?)?;
    Some(start..end)
}

/// The style classes a span kind falls in; a class has one style whatever
/// the span's value (a link's target, a mention's participant).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Code,
    Spoiler,
    Mention,
    Link,
}

const CLASSES: [Class; 8] = [
    Class::Bold,
    Class::Italic,
    Class::Underline,
    Class::Strikethrough,
    Class::Code,
    Class::Spoiler,
    Class::Mention,
    Class::Link,
];

fn class_of(kind: &SpanKind) -> Class {
    match kind {
        SpanKind::Bold => Class::Bold,
        SpanKind::Italic => Class::Italic,
        SpanKind::Underline => Class::Underline,
        SpanKind::Strikethrough => Class::Strikethrough,
        SpanKind::Code => Class::Code,
        SpanKind::Spoiler => Class::Spoiler,
        SpanKind::Mention { .. } => Class::Mention,
        SpanKind::Link { .. } => Class::Link,
    }
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

fn style_of(class: Class, foreground: gpui_kit::Hsla, cx: &App) -> HighlightStyle {
    let theme = cx.theme();
    let line = |color| UnderlineStyle { thickness: px(1.), color: Some(color), wavy: false };
    match class {
        Class::Bold => HighlightStyle { font_weight: Some(FontWeight::BOLD), ..Default::default() },
        Class::Italic => {
            HighlightStyle { font_style: Some(FontStyle::Italic), ..Default::default() }
        }
        Class::Underline => {
            HighlightStyle { underline: Some(line(foreground)), ..Default::default() }
        }
        Class::Strikethrough => HighlightStyle {
            strikethrough: Some(StrikethroughStyle { thickness: px(1.), color: Some(foreground) }),
            ..Default::default()
        },
        Class::Code => HighlightStyle {
            color: Some(theme.foreground),
            background_color: Some(theme.muted),
            ..Default::default()
        },
        Class::Spoiler => HighlightStyle {
            color: Some(theme.muted),
            background_color: Some(theme.muted),
            ..Default::default()
        },
        Class::Mention => HighlightStyle {
            color: Some(foreground),
            font_weight: Some(FontWeight::MEDIUM),
            ..Default::default()
        },
        Class::Link => HighlightStyle {
            color: Some(foreground),
            underline: Some(line(foreground)),
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

/// What a message's spans ask the renderer for, without any drawing.
struct Plan {
    /// Non-overlapping runs and the classes (a bit per [`CLASSES`] entry)
    /// active in each: a sweep over the span edges, so the cost follows the
    /// number of spans, not spans times intervals.
    runs: Vec<(Range<usize>, u8)>,
    /// The first [`MAX_MARKERS`] spans: the kind label key and the covered
    /// text, shortened to [`MARKER_SCALARS`] scalars.
    markers: Vec<(&'static str, String)>,
}

fn plan(text: &str, spans: &[TextSpan]) -> Plan {
    let starts = scalar_starts(text);
    let mut edges: Vec<(usize, bool, usize)> = Vec::with_capacity(spans.len() * 2);
    let mut markers = Vec::new();
    for span in spans {
        let Some(range) = byte_range(&starts, span) else { continue };
        let class = CLASSES.iter().position(|c| *c == class_of(&span.kind)).unwrap_or(0);
        edges.push((range.start, true, class));
        edges.push((range.end, false, class));
        if markers.len() < MAX_MARKERS {
            let covered: String = text[range].chars().take(MARKER_SCALARS).collect();
            markers.push((kind_key(&span.kind), covered));
        }
    }
    edges.sort_unstable();
    let mut counts = [0_u32; CLASSES.len()];
    let mut runs = Vec::new();
    let mut at = 0;
    for &(pos, opens, class) in &edges {
        if pos > at {
            let mask = mask_of(&counts);
            if mask != 0 {
                runs.push((at..pos, mask));
            }
        }
        at = pos;
        counts[class] = if opens { counts[class] + 1 } else { counts[class].saturating_sub(1) };
    }
    Plan { runs, markers }
}

fn mask_of(counts: &[u32; CLASSES.len()]) -> u8 {
    counts
        .iter()
        .enumerate()
        .fold(0, |mask, (bit, count)| if *count > 0 { mask | (1 << bit) } else { mask })
}

/// Renders `text` with `spans`. The first few spans also get a marker that
/// names their kind and the text they cover (shortened), for assistive
/// technology and tests; the full text is always in the message itself.
pub fn render(
    text: &SharedString,
    spans: &[TextSpan],
    foreground: gpui_kit::Hsla,
    cx: &App,
) -> impl IntoElement {
    let plan = plan(text, spans);
    let highlights: Vec<(Range<usize>, HighlightStyle)> = plan
        .runs
        .into_iter()
        .map(|(run, mask)| {
            let mut style = HighlightStyle::default();
            for (bit, class) in CLASSES.iter().enumerate() {
                if mask & (1 << bit) != 0 {
                    merged(&mut style, style_of(*class, foreground, cx));
                }
            }
            (run, style)
        })
        .collect();
    let markers: Vec<_> = plan
        .markers
        .iter()
        .enumerate()
        .map(|(index, (key, covered))| {
            let label = tr_args("moment.a11y.span", &[("kind", &tr(key)), ("text", covered)]);
            div()
                .id(SharedString::from(format!("message-span-{index}")))
                .aria_label(label)
                .size_0()
                .test_support()
        })
        .collect();
    div()
        .id("message-text")
        .child(StyledText::new(text.clone()).with_highlights(highlights))
        .children(markers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(offset: u32, length: u32, kind: SpanKind) -> TextSpan {
        TextSpan { offset, length, kind }
    }

    #[test]
    fn worst_case_full_length_spans_stay_within_the_render_budget() {
        // The largest permitted text with 4,096 spans, each covering all of it.
        let text = "x".repeat(262_144);
        let spans: Vec<TextSpan> = (0..4_096)
            .map(|i| match i % 3 {
                0 => span(0, 262_144, SpanKind::Bold),
                1 => span(0, 262_144, SpanKind::Italic),
                _ => span(0, 262_144, SpanKind::Link { url: "https://example.com".to_owned() }),
            })
            .collect();
        let plan = plan(&text, &spans);
        assert_eq!(plan.runs.len(), 1, "identical spans combine into one run");
        assert_eq!(plan.markers.len(), MAX_MARKERS, "markers are bounded");
        let label_bytes: usize = plan.markers.iter().map(|(_, covered)| covered.len()).sum();
        assert!(
            label_bytes <= MAX_MARKERS * MARKER_SCALARS,
            "label storage is bounded: {label_bytes}"
        );
    }

    #[test]
    fn staggered_overlapping_spans_make_at_most_two_runs_per_span() {
        let text = "x".repeat(262_144);
        let spans: Vec<TextSpan> =
            (0..4_096).map(|i| span(i * 8, 100_000, SpanKind::Bold)).collect();
        let plan = plan(&text, &spans);
        assert!(plan.runs.len() <= spans.len() * 2 + 1, "runs follow the span count");
    }

    #[test]
    fn spans_outside_the_text_are_dropped_and_scalars_are_not_bytes() {
        let plan =
            plan("A\u{1F600}\u{e9}", &[span(1, 1, SpanKind::Bold), span(2, 5, SpanKind::Italic)]);
        assert_eq!(
            plan.runs,
            vec![(1..5, 1)],
            "bold covers the emoji only; the long span is dropped"
        );
        assert_eq!(plan.markers.len(), 1);
        assert_eq!(plan.markers[0].1, "\u{1F600}");
    }
}
