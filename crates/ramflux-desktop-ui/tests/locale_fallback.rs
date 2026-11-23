// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The locale at run time: fallback to English, plural forms, argument
//! isolation, layout direction, pseudo-locales and locale-aware formatting.
//!
//! Its own test binary with one test: the locale is process-wide, so
//! switching it must not race other tests.

use ramflux_desktop_navigation::direction;
use ramflux_desktop_ui::i18n::{self, set_locale, tr, tr_args, tr_count};

const MEMBERS: &str = "sections.groups.members";

#[test]
fn the_current_locale_drives_text_direction_and_formats() {
    // The source locale and a complete translation.
    set_locale("en");
    assert_eq!(tr("common.cancel").as_ref(), "Cancel");
    assert_eq!(tr("no.such.key").as_ref(), "no.such.key", "never empty, never a locale prefix");
    assert_eq!(tr_count(MEMBERS, 1).as_ref(), "1 member");
    assert_eq!(tr_count(MEMBERS, 2).as_ref(), "2 members");
    assert!(!direction::is_rtl());
    set_locale("zh-CN");
    assert_ne!(tr("common.cancel").as_ref(), "Cancel");
    assert_eq!(tr_count(MEMBERS, 1), tr_count(MEMBERS, 5).replace('5', "1"), "only `other`");

    // Untranslated copy falls back to English, plural rules included, while
    // the locale's own formatting data applies.
    set_locale("ru");
    assert_eq!(i18n::locale(), "ru");
    assert_eq!(tr("common.cancel").as_ref(), "Cancel");
    assert_eq!(tr_count(MEMBERS, 21).as_ref(), "21 members");
    assert_eq!(i18n::number(1_234_567), "1\u{a0}234\u{a0}567");
    set_locale("de");
    assert_eq!(i18n::number(12_345), "12.345");
    assert_eq!(i18n::bytes(482_133), "470,8 KiB");
    assert_eq!(i18n::percent(0.425).as_ref(), "43\u{a0}%");
    set_locale("hi");
    assert_eq!(i18n::number(1_234_567), "12,34,567");
    set_locale("tr");
    assert_eq!(i18n::percent(0.425).as_ref(), "%43");
    set_locale("ja");
    assert_eq!(i18n::list(&["a", "b"]).as_ref(), "a\u{3001}b");

    // Arabic: right to left, arguments isolated, identifiers left to right.
    set_locale("ar");
    assert!(direction::is_rtl(), "Arabic lays out right to left");
    assert_eq!(tr_count(MEMBERS, 3).as_ref(), "\u{2068}3\u{2069} members");
    assert_eq!(
        tr_args("identity.id.tooltip", &[("id", "rf1abc")]).matches('\u{2068}').count(),
        1,
        "an argument is isolated once"
    );
    assert_eq!(i18n::bidi::ltr("1234 5678"), "\u{2066}1234 5678\u{2069}");

    // A system tag resolves to a shipped locale.
    set_locale("pt_PT.UTF-8");
    assert_eq!(i18n::locale(), "pt-BR");
    assert!(!direction::is_rtl());
    set_locale("zh-Hant-HK");
    assert_eq!(i18n::locale(), "zh-TW");

    // Pseudo-locales (developer builds).
    if i18n::locale::pseudo_enabled() {
        set_locale("en-XA");
        assert_eq!(i18n::locale(), "en-XA");
        let long = tr("common.cancel");
        assert!(long.starts_with("[\u{c7}\u{e1}\u{f1}") && long.ends_with(']'), "{long}");
        assert!(!direction::is_rtl());
        assert_eq!(i18n::number(1_234), "1,234", "formats stay English");
        set_locale("ar-XB");
        assert!(direction::is_rtl());
        assert_eq!(tr("common.cancel").as_ref(), "lecnaC");
        assert_eq!(tr_count(MEMBERS, 2).as_ref(), "srebmem \u{2068}2\u{2069}");
    }

    set_locale("en");
    assert!(!direction::is_rtl());
}
