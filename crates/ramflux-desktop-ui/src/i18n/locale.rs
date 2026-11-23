// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The locales: which ones ship, how a system language tag selects one, the
//! fallback chain of a lookup, and the current locale.
//!
//! Adding a language is a data change: a row in [`SHIPPED`] and a catalogue
//! `locales/<tag>.json`. No code path names a language besides this table
//! and the plural rules.

use std::sync::atomic::{AtomicUsize, Ordering};

use ramflux_desktop_navigation::direction::LayoutDirection;

/// How a pseudo-locale rewrites the English text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pseudo {
    /// Accented letters, about 40 % longer, in brackets: shows hard-coded
    /// text, clipping and truncation.
    Long,
    /// Mirrored text in a right-to-left layout: shows what does not mirror.
    RightToLeft,
}

/// One selectable locale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Locale {
    /// BCP 47 tag; also the catalogue's file name and the stored choice.
    pub tag: &'static str,
    /// The key of the locale's own name (an endonym, the same in every
    /// language).
    pub name_key: &'static str,
    /// The layout direction of the interface in this locale.
    pub direction: LayoutDirection,
    /// The locale GPUI Kit's own strings use (its catalogue knows a few).
    pub kit: &'static str,
    /// Set for a pseudo-locale (developer builds only).
    pub pseudo: Option<Pseudo>,
}

const fn shipped(tag: &'static str, name_key: &'static str, kit: &'static str) -> Locale {
    Locale { tag, name_key, direction: LayoutDirection::LeftToRight, kit, pseudo: None }
}

/// The launch languages, in the order the language picker lists them. The
/// first is the source locale.
pub const SHIPPED: [Locale; 15] = [
    shipped("en", "locale.name.en", "en"),
    shipped("zh-CN", "locale.name.zh_cn", "zh-CN"),
    shipped("zh-TW", "locale.name.zh_tw", "zh-TW"),
    shipped("ja", "locale.name.ja", "en"),
    shipped("ko", "locale.name.ko", "en"),
    shipped("es", "locale.name.es", "en"),
    shipped("fr", "locale.name.fr", "en"),
    shipped("de", "locale.name.de", "en"),
    shipped("pt-BR", "locale.name.pt_br", "en"),
    shipped("ru", "locale.name.ru", "en"),
    Locale {
        tag: "ar",
        name_key: "locale.name.ar",
        direction: LayoutDirection::RightToLeft,
        kit: "en",
        pseudo: None,
    },
    shipped("hi", "locale.name.hi", "en"),
    shipped("id", "locale.name.id", "en"),
    shipped("tr", "locale.name.tr", "en"),
    shipped("it", "locale.name.it", "it"),
];

/// The pseudo-locales, selectable in developer builds only.
pub const PSEUDO: [Locale; 2] = [
    Locale {
        tag: "en-XA",
        name_key: "locale.name.en_xa",
        direction: LayoutDirection::LeftToRight,
        kit: "en",
        pseudo: Some(Pseudo::Long),
    },
    Locale {
        tag: "ar-XB",
        name_key: "locale.name.ar_xb",
        direction: LayoutDirection::RightToLeft,
        kit: "en",
        pseudo: Some(Pseudo::RightToLeft),
    },
];

/// The source locale: every key has its text, every other locale falls back
/// to it.
pub const SOURCE: &str = "en";

/// Whether pseudo-locales can be selected (developer builds).
pub fn pseudo_enabled() -> bool {
    cfg!(debug_assertions)
}

/// Every locale this build lets the user select, in picker order.
pub fn selectable() -> impl Iterator<Item = &'static Locale> {
    SHIPPED.iter().chain(PSEUDO.iter().filter(|_| pseudo_enabled()))
}

/// The selectable locale with exactly this tag.
pub fn find(tag: &str) -> Option<&'static Locale> {
    selectable().find(|locale| locale.tag == tag)
}

fn all() -> impl Iterator<Item = &'static Locale> {
    SHIPPED.iter().chain(PSEUDO.iter())
}

/// A system language tag in canonical form: `en_US.UTF-8@euro` becomes
/// `en-US`.
fn canonical(tag: &str) -> String {
    let tag = tag.split(['.', '@']).next().unwrap_or_default();
    tag.replace('_', "-")
}

/// The shipped locale that serves a system language tag (`zh-Hans-CN`,
/// `pt_PT.UTF-8`, `ar-EG`), by BCP 47 matching: the exact tag, then Chinese
/// by script and region, then the language, then English.
pub fn resolve(tag: &str) -> &'static Locale {
    let tag = canonical(tag);
    let subtags: Vec<&str> = tag.split('-').filter(|s| !s.is_empty()).collect();
    let language = subtags.first().map(|s| s.to_ascii_lowercase()).unwrap_or_default();
    if let Some(exact) = SHIPPED.iter().find(|l| l.tag.eq_ignore_ascii_case(&tag)) {
        return exact;
    }
    let has = |wanted: &[&str]| {
        subtags.iter().skip(1).any(|s| wanted.iter().any(|w| s.eq_ignore_ascii_case(w)))
    };
    let tagged = |wanted: &str| SHIPPED.iter().find(|l| l.tag == wanted);
    if language == "zh" {
        // Traditional script, or a region that writes it.
        let traditional = has(&["Hant", "TW", "HK", "MO"]) && !has(&["Hans"]);
        if let Some(locale) = tagged(if traditional { "zh-TW" } else { "zh-CN" }) {
            return locale;
        }
    }
    SHIPPED
        .iter()
        .find(|l| l.tag.split('-').next().is_some_and(|lang| lang == language))
        .unwrap_or(&SHIPPED[0])
}

/// The tags a lookup in `tag` tries, in order: the locale, its language
/// alone when that is another catalogue, then English. A pseudo-locale is
/// built from English, so its chain is English alone.
pub fn chain(locale: &Locale) -> Vec<&'static str> {
    let mut tags: Vec<&'static str> = Vec::with_capacity(3);
    if locale.pseudo.is_none() {
        if let Some(own) = all().find(|l| l.tag == locale.tag) {
            tags.push(own.tag);
        }
        let language = locale.tag.split('-').next().unwrap_or(locale.tag);
        if let Some(parent) = SHIPPED.iter().find(|l| l.tag == language && l.tag != locale.tag) {
            tags.push(parent.tag);
        }
    }
    if !tags.contains(&SOURCE) {
        tags.push(SOURCE);
    }
    tags
}

/// Index of the current locale in `SHIPPED` followed by `PSEUDO`.
static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// The current locale.
pub fn current() -> &'static Locale {
    all().nth(CURRENT.load(Ordering::Relaxed)).unwrap_or(&SHIPPED[0])
}

/// Makes `tag` current, if it is a selectable locale (else a shipped locale
/// resolved from it), and returns the locale now current.
pub fn set_current(tag: &str) -> &'static Locale {
    let locale = find(tag).unwrap_or_else(|| resolve(tag));
    if let Some(index) = all().position(|l| l.tag == locale.tag) {
        CURRENT.store(index, Ordering::Relaxed);
    }
    locale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_tags_resolve_to_a_shipped_locale() {
        let cases = [
            ("en_US.UTF-8", "en"),
            ("en-GB", "en"),
            ("zh-Hans-CN", "zh-CN"),
            ("zh_CN", "zh-CN"),
            ("zh-SG", "zh-CN"),
            ("zh", "zh-CN"),
            ("zh-Hant", "zh-TW"),
            ("zh-Hant-HK", "zh-TW"),
            ("zh-TW", "zh-TW"),
            ("zh-HK", "zh-TW"),
            ("zh-MO", "zh-TW"),
            ("pt-BR", "pt-BR"),
            ("pt-PT", "pt-BR"),
            ("pt", "pt-BR"),
            ("ar-EG", "ar"),
            ("ar_SA", "ar"),
            ("de-AT", "de"),
            ("es-419", "es"),
            ("fr-CA", "fr"),
            ("ja-JP", "ja"),
            ("ko-KR", "ko"),
            ("ru-RU", "ru"),
            ("hi-IN", "hi"),
            ("id-ID", "id"),
            ("tr-TR", "tr"),
            ("it-IT", "it"),
            ("tlh", "en"),
            ("", "en"),
            ("C", "en"),
        ];
        for (tag, expected) in cases {
            assert_eq!(resolve(tag).tag, expected, "{tag}");
        }
    }

    #[test]
    fn the_chain_ends_in_english() {
        let tag = |t: &str| SHIPPED.iter().find(|l| l.tag == t).copied();
        assert_eq!(chain(&SHIPPED[0]), ["en"]);
        assert_eq!(tag("zh-TW").map(|l| chain(&l)), Some(vec!["zh-TW", "en"]));
        assert_eq!(tag("pt-BR").map(|l| chain(&l)), Some(vec!["pt-BR", "en"]));
        assert_eq!(chain(&PSEUDO[1]), ["en"], "a pseudo-locale is built from English");
        // A regional locale falls back to its language when both ship.
        let regional = Locale { tag: "ar-EG", ..SHIPPED[10] };
        assert_eq!(chain(&regional), ["ar", "en"]);
    }

    #[test]
    fn only_arabic_and_the_rtl_pseudo_locale_read_right_to_left() {
        let rtl: Vec<&str> = all().filter(|l| l.direction.is_rtl()).map(|l| l.tag).collect();
        assert_eq!(rtl, ["ar", "ar-XB"]);
    }

    #[test]
    fn tags_and_name_keys_are_unique() {
        let tags: std::collections::BTreeSet<&str> = all().map(|l| l.tag).collect();
        let names: std::collections::BTreeSet<&str> = all().map(|l| l.name_key).collect();
        assert_eq!(tags.len(), SHIPPED.len() + PSEUDO.len());
        assert_eq!(names.len(), SHIPPED.len() + PSEUDO.len());
        assert_eq!(SHIPPED[0].tag, SOURCE);
    }

    #[test]
    fn pseudo_locales_are_selectable_in_developer_builds_only() {
        let selectable_pseudo = selectable().filter(|l| l.pseudo.is_some()).count();
        assert_eq!(selectable_pseudo, if cfg!(debug_assertions) { 2 } else { 0 });
    }
}
