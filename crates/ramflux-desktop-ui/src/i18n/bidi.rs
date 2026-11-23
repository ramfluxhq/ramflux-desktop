// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Bidirectional text: isolating untrusted text, keeping identifiers left
//! to right, and the direction of a message's content.
//!
//! Rules (the same as the phones'):
//!
//! 1. Text from peers or the network (names, group names, file names,
//!    message text, previews, node origins) never reorders the interface
//!    text around it: it is placed inside an isolate ([`isolate`]), and its
//!    embedding and override controls (U+202A..U+202E) are removed. Its own
//!    isolates are balanced so it cannot close ours.
//! 2. Names and file names lose every directional control ([`strip_controls`]);
//!    message bodies keep the marks (LRM, RLM, ALM) a writer may need.
//! 3. Identifiers (codes, safety numbers, fingerprints, ids, hashes, URLs,
//!    account ids, recovery words) always read left to right ([`ltr`]).
//! 4. A message reads in its own direction ([`content_direction`]), whatever
//!    the interface's.

use std::borrow::Cow;

use ramflux_desktop_navigation::direction::LayoutDirection;

/// First strong isolate: the isolated text takes its own direction.
pub const FSI: char = '\u{2068}';
/// Left-to-right isolate.
pub const LRI: char = '\u{2066}';
/// Right-to-left isolate.
pub const RLI: char = '\u{2067}';
/// Pop directional isolate: ends an isolate.
pub const PDI: char = '\u{2069}';

/// Whether `c` is a bidirectional formatting character: a mark, an
/// embedding, an override or an isolate.
pub fn is_control(c: char) -> bool {
    matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Whether `c` is an embedding or override control (LRE, RLE, PDF, LRO,
/// RLO), which untrusted text never keeps.
fn is_embedding(c: char) -> bool {
    matches!(c, '\u{202a}'..='\u{202e}')
}

/// Whether `c` is a strong right-to-left character (Hebrew, Arabic, Syriac,
/// Thaana, N'Ko and their presentation forms; digits and signs excluded).
pub fn is_strong_rtl(c: char) -> bool {
    let arabic_number = matches!(c, '\u{0600}'..='\u{0605}' | '\u{0660}'..='\u{0669}' | '\u{066b}' | '\u{066c}' | '\u{06dd}' | '\u{06f0}'..='\u{06f9}');
    !arabic_number
        && matches!(c,
            '\u{0590}'..='\u{08ff}'
            | '\u{fb1d}'..='\u{fdff}'
            | '\u{fe70}'..='\u{fefe}'
            | '\u{10800}'..='\u{10fff}'
            | '\u{1e800}'..='\u{1efff}')
        && !matches!(c, '\u{0591}'..='\u{05bd}' | '\u{064b}'..='\u{065f}' | '\u{0670}')
}

/// Whether `c` is a strong left-to-right character.
fn is_strong_ltr(c: char) -> bool {
    c.is_alphabetic() && !is_strong_rtl(c) && !('\u{0590}'..='\u{08ff}').contains(&c)
}

/// Whether `text` contains a right-to-left letter.
pub fn has_rtl(text: &str) -> bool {
    text.chars().any(is_strong_rtl)
}

/// `text` without any directional control: for names and file names.
pub fn strip_controls(text: &str) -> Cow<'_, str> {
    if text.chars().any(is_control) {
        Cow::Owned(text.chars().filter(|c| !is_control(*c)).collect())
    } else {
        Cow::Borrowed(text)
    }
}

/// `text` without embeddings and overrides, with every isolate it opens
/// closed and every unmatched close removed, so it cannot leak a direction
/// into, or close an isolate of, the text around it.
pub fn balanced(text: &str) -> Cow<'_, str> {
    let mut depth = 0_usize;
    let mut clean = true;
    for c in text.chars() {
        match c {
            c if is_embedding(c) => clean = false,
            FSI | LRI | RLI => depth += 1,
            PDI if depth == 0 => clean = false,
            PDI => depth -= 1,
            _ => {}
        }
    }
    if clean && depth == 0 {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 3 * depth);
    let mut open = 0_usize;
    for c in text.chars() {
        match c {
            c if is_embedding(c) => {}
            FSI | LRI | RLI => {
                open += 1;
                out.push(c);
            }
            PDI if open == 0 => {}
            PDI => {
                open -= 1;
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out.extend(std::iter::repeat_n(PDI, open));
    Cow::Owned(out)
}

/// Untrusted `text` made safe to place inside interface text: balanced (see
/// [`balanced`]) and wrapped in a first-strong isolate.
pub fn isolate(text: &str) -> String {
    format!("{FSI}{}{PDI}", balanced(text))
}

/// An identifier (a code, a safety number, a fingerprint, an id, a hash, a
/// URL) that reads left to right in every locale: its own controls removed,
/// wrapped in a left-to-right isolate. Show it in a left-aligned element
/// that does not mirror.
pub fn ltr(text: &str) -> String {
    format!("{LRI}{}{PDI}", strip_controls(text))
}

/// Whether a value substituted into a translated message must be isolated:
/// always in a right-to-left interface, otherwise only when the value holds
/// right-to-left text or directional controls (so left-to-right text in a
/// left-to-right interface is unchanged).
pub fn argument(value: &str, interface: LayoutDirection) -> Cow<'_, str> {
    let clean = balanced(value);
    if is_isolated(&clean) {
        return clean;
    }
    let changed = matches!(clean, Cow::Owned(_));
    if changed || interface.is_rtl() || has_rtl(&clean) {
        Cow::Owned(format!("{FSI}{clean}{PDI}"))
    } else {
        clean
    }
}

/// Whether balanced `text` is one isolate from its first character to its
/// last (a label that is already isolated).
fn is_isolated(text: &str) -> bool {
    let mut chars = text.chars();
    if !matches!(chars.next(), Some(FSI | LRI | RLI)) {
        return false;
    }
    let mut depth = 1_usize;
    let rest = chars.as_str();
    for (at, c) in rest.char_indices() {
        match c {
            FSI | LRI | RLI => depth += 1,
            PDI => {
                depth -= 1;
                if depth == 0 {
                    return at + c.len_utf8() == rest.len();
                }
            }
            _ => {}
        }
    }
    false
}

/// Whether the word starting at `word` is skipped when finding a message's
/// direction: a mention, a link or an address.
fn skipped(word: &str) -> bool {
    word.starts_with('@')
        || word.starts_with('#')
        || word.contains("://")
        || word.starts_with("www.")
        || (word.contains('@') && word.contains('.'))
}

/// The direction a message reads in: the direction of its first strong
/// letter, skipping mentions, links, addresses and inline code (between
/// backticks), or `None` for text without letters.
pub fn content_direction(text: &str) -> Option<LayoutDirection> {
    /// Only the start of a long text is read: a render must stay cheap.
    const SCAN: usize = 4096;
    let text = text.char_indices().nth(SCAN).map_or(text, |(end, _)| &text[..end]);
    let mut prose = String::with_capacity(text.len());
    let mut in_code = false;
    for c in text.chars() {
        if c == '`' {
            in_code = !in_code;
            prose.push(' ');
        } else if !in_code {
            prose.push(c);
        }
    }
    prose
        .split_whitespace()
        .filter(|word| !skipped(word))
        .find_map(first_strong)
        // Only mentions, links or code: the first strong letter anywhere.
        .or_else(|| first_strong(text))
}

fn first_strong(text: &str) -> Option<LayoutDirection> {
    text.chars().find_map(|c| {
        if is_strong_rtl(c) {
            Some(LayoutDirection::RightToLeft)
        } else if is_strong_ltr(c) {
            Some(LayoutDirection::LeftToRight)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use LayoutDirection::{LeftToRight, RightToLeft};

    #[test]
    fn isolation_wraps_and_cannot_be_escaped() {
        assert_eq!(isolate("Alice"), "\u{2068}Alice\u{2069}");
        // A spoofing override is removed; a stray close cannot end our isolate.
        assert_eq!(isolate("Alice\u{202e}gpj.exe"), "\u{2068}Alicegpj.exe\u{2069}");
        assert_eq!(isolate("x\u{2069}y"), "\u{2068}xy\u{2069}");
        // An isolate the text opens is closed inside ours.
        assert_eq!(isolate("\u{2067}abc"), "\u{2068}\u{2067}abc\u{2069}\u{2069}");
        // Balanced isolates are kept (a name already isolated by a list label).
        assert_eq!(balanced("\u{2068}a\u{2069}"), "\u{2068}a\u{2069}");
    }

    #[test]
    fn names_lose_every_control_and_bodies_keep_marks() {
        assert_eq!(strip_controls("a\u{200f}b\u{2066}c\u{202d}"), "abc");
        assert_eq!(balanced("\u{200f}abc"), "\u{200f}abc", "a mark survives in a body");
    }

    #[test]
    fn identifiers_are_left_to_right_isolates() {
        assert_eq!(ltr("1234 5678"), "\u{2066}1234 5678\u{2069}");
        assert_eq!(ltr("12\u{202e}34"), "\u{2066}1234\u{2069}");
    }

    #[test]
    fn arguments_change_only_when_direction_matters() {
        assert_eq!(argument("Alice", LeftToRight), "Alice", "LTR text in an LTR UI is unchanged");
        assert_eq!(argument("3", LeftToRight), "3");
        assert_eq!(
            argument("\u{0645}\u{0631}\u{062d}\u{0628}\u{0627}", LeftToRight).chars().next(),
            Some(FSI)
        );
        assert_eq!(argument("Alice", RightToLeft), "\u{2068}Alice\u{2069}");
        assert_eq!(argument("a\u{202e}b", LeftToRight), "\u{2068}ab\u{2069}");
        let label = "\u{2068}\u{0645}\u{0631}\u{2069}";
        assert_eq!(argument(label, LeftToRight), label, "an isolated label is not wrapped again");
        assert_eq!(argument(label, RightToLeft), label);
        let two = "\u{2068}a\u{2069} \u{2068}b\u{2069}";
        assert_eq!(argument(two, RightToLeft).chars().next(), Some(FSI));
        assert_ne!(argument(two, RightToLeft), two, "two isolates side by side are wrapped");
    }

    #[test]
    fn a_message_reads_in_the_direction_of_its_first_letter() {
        assert_eq!(content_direction("Hello"), Some(LeftToRight));
        assert_eq!(
            content_direction("\u{0645}\u{0631}\u{062d}\u{0628}\u{0627} world"),
            Some(RightToLeft)
        );
        assert_eq!(content_direction("123 \u{05e9}\u{05dc}\u{05d5}\u{05dd}"), Some(RightToLeft));
        assert_eq!(content_direction("12345"), None);
        assert_eq!(content_direction(""), None);
        // Mentions, links and code do not decide.
        assert_eq!(
            content_direction("@bob \u{0645}\u{0631}\u{062d}\u{0628}\u{0627}"),
            Some(RightToLeft)
        );
        assert_eq!(
            content_direction("https://example.com \u{0645}\u{0631}\u{062d}\u{0628}\u{0627}"),
            Some(RightToLeft)
        );
        assert_eq!(
            content_direction("`let x` \u{0645}\u{0631}\u{062d}\u{0628}\u{0627}"),
            Some(RightToLeft)
        );
        assert_eq!(content_direction("@bob"), Some(LeftToRight), "only a mention: its letters");
        // Arabic-Indic digits are numbers, not letters.
        assert_eq!(content_direction("\u{0661}\u{0662} ok"), Some(LeftToRight));
    }

    #[test]
    fn cjk_and_cyrillic_are_left_to_right() {
        assert_eq!(content_direction("\u{4f60}\u{597d}"), Some(LeftToRight));
        assert_eq!(
            content_direction("\u{041f}\u{0440}\u{0438}\u{0432}\u{0435}\u{0442}"),
            Some(LeftToRight)
        );
        assert!(!has_rtl("\u{0661}\u{0662}"));
        assert!(has_rtl("a\u{0627}"));
    }
}
