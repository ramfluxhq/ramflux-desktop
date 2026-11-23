// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Pseudo-locales: English text rewritten so layout problems show without a
//! translation.
//!
//! - [`Pseudo::Long`] (`en-XA`): letters get accents, the text grows by about
//!   40 % with filler words, and brackets mark both ends, so clipping,
//!   truncation and text that bypasses the catalogue are visible.
//! - [`Pseudo::RightToLeft`] (`ar-XB`): the text is mirrored (its parts in
//!   reverse order, each literal reversed) and the interface lays out right
//!   to left, so anything that does not mirror stands out to a reader of
//!   English. No bidirectional control characters are added.
//!
//! Placeholders (`%{name}`) are kept as they are, and so are formatting
//! patterns (`format.*`: date patterns, separators, units), which are data,
//! not copy.

use super::locale::Pseudo;

/// Whether the text of `key` is left alone by pseudo-locales.
pub fn exempt(key: &str) -> bool {
    key.starts_with("format.")
}

/// A piece of a message: literal text or a `%{name}` placeholder.
enum Part<'a> {
    Text(&'a str),
    Placeholder(&'a str),
}

fn parts(text: &str) -> Vec<Part<'_>> {
    let mut parts = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("%{") {
        let Some(len) = rest[start..].find('}') else { break };
        if start > 0 {
            parts.push(Part::Text(&rest[..start]));
        }
        parts.push(Part::Placeholder(&rest[start..=start + len]));
        rest = &rest[start + len + 1..];
    }
    if !rest.is_empty() {
        parts.push(Part::Text(rest));
    }
    parts
}

/// The accented form of an ASCII letter.
fn accented(c: char) -> char {
    const LOWER: [char; 26] = [
        'á', 'ƀ', 'ç', 'ð', 'é', 'ƒ', 'ĝ', 'ĥ', 'î', 'ĵ', 'ķ', 'ļ', 'ɱ', 'ñ', 'ö', 'þ', 'ǫ', 'ŕ',
        'š', 'ţ', 'û', 'ṽ', 'ŵ', 'ẋ', 'ý', 'ž',
    ];
    const UPPER: [char; 26] = [
        'Å', 'Ɓ', 'Ç', 'Ð', 'É', 'Ƒ', 'Ĝ', 'Ĥ', 'Î', 'Ĵ', 'Ķ', 'Ļ', 'Ṁ', 'Ñ', 'Ö', 'Þ', 'Ǫ', 'Ŕ',
        'Š', 'Ţ', 'Û', 'Ṽ', 'Ŵ', 'Ẋ', 'Ý', 'Ž',
    ];
    let index = |base: u8| usize::from((c as u8).wrapping_sub(base));
    match c {
        'a'..='z' => LOWER.get(index(b'a')).copied().unwrap_or(c),
        'A'..='Z' => UPPER.get(index(b'A')).copied().unwrap_or(c),
        _ => c,
    }
}

/// Filler that lengthens a text, cycled as needed.
const FILLER: &str = "one two three four five six seven eight nine ten";

fn long(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2 + 4);
    let mut visible = 0;
    out.push('[');
    for part in parts(text) {
        match part {
            Part::Text(t) => {
                visible += t.chars().count();
                out.extend(t.chars().map(accented));
            }
            Part::Placeholder(p) => out.push_str(p),
        }
    }
    // About 40 % more, rounded up, counting the brackets.
    let extra = (visible * 2).div_ceil(5).saturating_sub(2);
    if extra > 0 {
        out.push(' ');
        out.extend(FILLER.chars().cycle().take(extra.saturating_sub(1)));
    }
    out.push(']');
    out
}

fn mirrored(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for part in parts(text).into_iter().rev() {
        match part {
            Part::Text(t) => out.extend(t.chars().rev()),
            Part::Placeholder(p) => out.push_str(p),
        }
    }
    out
}

/// `text` (English, for `key`) as `pseudo` shows it.
pub fn transform(pseudo: Pseudo, key: &str, text: &str) -> String {
    if exempt(key) {
        return text.to_owned();
    }
    match pseudo {
        Pseudo::Long => long(text),
        Pseudo::RightToLeft => mirrored(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_long_pseudo_locale_accents_lengthens_and_brackets() {
        let text = transform(Pseudo::Long, "settings.title", "Settings");
        assert!(text.starts_with("[Šéţţîñĝš"), "{text}");
        assert!(text.ends_with(']'));
        let source = "Your messages are end-to-end encrypted on this device";
        let grown = transform(Pseudo::Long, "x.y", source);
        let ratio = grown.chars().count() as f64 / source.chars().count() as f64;
        assert!((1.35..=1.5).contains(&ratio), "about 40 % longer: {ratio}");
    }

    #[test]
    fn placeholders_survive_both_pseudo_locales() {
        let long = transform(Pseudo::Long, "x.y", "%{count} matches for %{name}");
        assert!(long.contains("%{count}") && long.contains("%{name}"), "{long}");
        let rtl = transform(Pseudo::RightToLeft, "x.y", "%{count} matches");
        assert_eq!(rtl, "sehctam %{count}");
        assert_eq!(transform(Pseudo::RightToLeft, "x.y", "%{a} \u{b7} %{b}"), "%{b} \u{b7} %{a}");
    }

    #[test]
    fn formatting_patterns_are_data_not_copy() {
        assert_eq!(transform(Pseudo::Long, "format.time", "%-I:%M %p"), "%-I:%M %p");
        assert_eq!(transform(Pseudo::RightToLeft, "format.number.group", ","), ",");
    }

    #[test]
    fn mirrored_text_carries_no_bidi_controls() {
        let text = transform(Pseudo::RightToLeft, "x.y", "Open settings");
        assert!(!text.chars().any(super::super::bidi::is_control), "{text}");
    }
}
