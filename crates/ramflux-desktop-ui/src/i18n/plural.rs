// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! CLDR plural rules for whole-number counts.
//!
//! A plural message has one catalogue entry per category its language uses
//! (`key.one`, `key.few`, …; `key.other` always). The rules below are the
//! CLDR cardinal rules restricted to non-negative integers (`v = 0`), for the
//! languages of the shipped locales. A language without a rule here uses
//! only `other`, which is the CLDR rule for languages without plural forms.

/// A CLDR plural category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    /// `zero` (Arabic).
    Zero,
    /// `one`.
    One,
    /// `two` (Arabic).
    Two,
    /// `few` (Arabic, Russian).
    Few,
    /// `many` (Arabic, Russian; millions in French, Spanish, Italian and
    /// Portuguese).
    Many,
    /// `other`: every language has it.
    Other,
}

impl Category {
    /// Every category, in CLDR order.
    pub const ALL: [Self; 6] =
        [Self::Zero, Self::One, Self::Two, Self::Few, Self::Many, Self::Other];

    /// The category's name, which is also the last segment of its key.
    pub fn name(self) -> &'static str {
        match self {
            Self::Zero => "zero",
            Self::One => "one",
            Self::Two => "two",
            Self::Few => "few",
            Self::Many => "many",
            Self::Other => "other",
        }
    }

    /// The category named `name`, if it is one.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == name)
    }
}

/// The language subtag of a locale tag (`pt` of `pt-BR`).
fn language(tag: &str) -> &str {
    tag.split(['-', '_']).next().unwrap_or(tag)
}

/// A count that is a whole multiple of a million: the CLDR `many` of the
/// Romance languages (`e = 0 and i != 0 and i % 1000000 = 0 and v = 0`).
fn millions(n: u64) -> bool {
    n != 0 && n.is_multiple_of(1_000_000)
}

/// The plural category of `n` in the language of `tag`.
pub fn category(tag: &str, n: u64) -> Category {
    let (n10, n100) = (n % 10, n % 100);
    match language(tag) {
        "en" | "de" | "it" | "tr" if n == 1 => Category::One,
        "it" if millions(n) => Category::Many,
        "es" if n == 1 => Category::One,
        "es" if millions(n) => Category::Many,
        "fr" | "pt" if n <= 1 => Category::One,
        "fr" | "pt" if millions(n) => Category::Many,
        "hi" if n <= 1 => Category::One,
        "ru" if n10 == 1 && n100 != 11 => Category::One,
        "ru" if (2..=4).contains(&n10) && !(12..=14).contains(&n100) => Category::Few,
        "ru" => Category::Many,
        "ar" => match (n, n100) {
            (0, _) => Category::Zero,
            (1, _) => Category::One,
            (2, _) => Category::Two,
            (_, 3..=10) => Category::Few,
            (_, 11..=99) => Category::Many,
            _ => Category::Other,
        },
        _ => Category::Other,
    }
}

/// The categories a plural message must carry in the language of `tag`.
pub fn categories(tag: &str) -> &'static [Category] {
    use Category::{Few, Many, One, Other, Two, Zero};
    match language(tag) {
        "en" | "de" | "tr" | "hi" => &[One, Other],
        "es" | "fr" | "it" | "pt" => &[One, Many, Other],
        "ru" => &[One, Few, Many, Other],
        "ar" => &[Zero, One, Two, Few, Many, Other],
        _ => &[Other],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Category::{Few, Many, One, Other, Two, Zero};

    #[test]
    fn english_and_german_have_one_and_other() {
        for tag in ["en", "de", "tr", "it"] {
            assert_eq!(category(tag, 1), One, "{tag}");
            assert_eq!(category(tag, 0), Other, "{tag}");
            assert_eq!(category(tag, 2), Other, "{tag}");
            assert_eq!(category(tag, 21), Other, "{tag}");
        }
    }

    #[test]
    fn french_portuguese_and_hindi_count_zero_as_one() {
        for tag in ["fr", "pt-BR", "hi"] {
            assert_eq!(category(tag, 0), One, "{tag}");
            assert_eq!(category(tag, 1), One, "{tag}");
            assert_eq!(category(tag, 2), Other, "{tag}");
        }
        assert_eq!(category("es", 0), Other, "Spanish zero is plural");
    }

    #[test]
    fn romance_languages_have_many_for_millions() {
        for tag in ["fr", "es", "it", "pt-BR"] {
            assert_eq!(category(tag, 1_000_000), Many, "{tag}");
            assert_eq!(category(tag, 2_000_000), Many, "{tag}");
            assert_eq!(category(tag, 1_000_001), Other, "{tag}");
        }
    }

    #[test]
    fn russian_follows_the_last_digits() {
        let cases = [
            (1, One),
            (21, One),
            (11, Many),
            (2, Few),
            (4, Few),
            (22, Few),
            (12, Many),
            (14, Many),
        ];
        for (n, expected) in cases {
            assert_eq!(category("ru", n), expected, "{n}");
        }
        assert_eq!(category("ru", 0), Many);
        assert_eq!(category("ru", 5), Many);
        assert_eq!(category("ru", 111), Many);
        assert_eq!(category("ru", 101), One);
    }

    #[test]
    fn arabic_has_six_categories() {
        let cases = [
            (0, Zero),
            (1, One),
            (2, Two),
            (3, Few),
            (10, Few),
            (103, Few),
            (11, Many),
            (99, Many),
            (111, Many),
            (100, Other),
            (102, Other),
            (1_000, Other),
        ];
        for (n, expected) in cases {
            assert_eq!(category("ar", n), expected, "{n}");
        }
        assert_eq!(categories("ar").len(), 6);
    }

    #[test]
    fn languages_without_plurals_use_other_only() {
        for tag in ["zh-CN", "zh-TW", "ja", "ko", "id"] {
            for n in [0, 1, 2, 5, 11, 1_000_000] {
                assert_eq!(category(tag, n), Other, "{tag} {n}");
            }
            assert_eq!(categories(tag), &[Other]);
        }
    }

    #[test]
    fn every_category_a_rule_returns_is_declared() {
        for tag in crate::i18n::locale::SHIPPED.iter().map(|l| l.tag) {
            for n in 0..2_000_000_u64 {
                if n > 1_200 && !n.is_multiple_of(1_000_000) {
                    continue;
                }
                let c = category(tag, n);
                assert!(categories(tag).contains(&c), "{tag}: {n} is {c:?}, not declared");
            }
        }
    }
}
