// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Status colors used as text.
//!
//! The theme's status colors (`success`, `warning`, `danger`) are made for
//! fills, icons and borders. Small text drawn in them on a light background
//! falls far below the 4.5:1 contrast WCAG asks for body text (green 2.3:1,
//! yellow under 2:1). [`text`] keeps the token's hue and saturation and only
//! darkens it, on a light background, until it reaches that ratio; on a dark
//! background the tokens already pass and are returned unchanged.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, Hsla, Rgba};

/// The contrast small text needs against its background (WCAG 2.x AA).
pub const TEXT_CONTRAST: f32 = 4.5;

/// How much lightness each step takes away while searching.
const STEP: f32 = 0.02;

/// Relative luminance of a color, as WCAG defines it.
fn luminance(color: Hsla) -> f32 {
    let rgba = Rgba::from(color);
    let channel = |c: f32| {
        if c <= 0.039_28 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(rgba.r) + 0.7152 * channel(rgba.g) + 0.0722 * channel(rgba.b)
}

/// The WCAG contrast ratio of two opaque colors.
pub fn contrast(a: Hsla, b: Hsla) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// `color` made readable as small text on `background`: darkened step by
/// step until it reaches [`TEXT_CONTRAST`], when the background is light.
pub fn on(color: Hsla, background: Hsla) -> Hsla {
    if background.l < 0.5 {
        return color;
    }
    let mut readable = Hsla { a: 1.0, ..color };
    while contrast(readable, background) < TEXT_CONTRAST && readable.l > 0.0 {
        readable.l = (readable.l - STEP).max(0.0);
    }
    readable
}

/// `color` made readable as small text on the window background.
pub fn text(color: Hsla, cx: &App) -> Hsla {
    on(color, cx.theme().background)
}

#[cfg(test)]
mod tests {
    use gpui_kit::{hsla, rgb};

    use super::*;

    fn white() -> Hsla {
        hsla(0., 0., 1., 1.)
    }

    #[test]
    fn light_backgrounds_get_text_that_reaches_the_ratio() {
        // The default theme's green-500, yellow-500 and red-500.
        for color in [rgb(0x0022_c55e), rgb(0x00ea_b308), rgb(0x00ef_4444)] {
            let color = Hsla::from(color);
            let readable = on(color, white());
            assert!(contrast(readable, white()) >= TEXT_CONTRAST, "{color:?} -> {readable:?}");
            assert!((readable.h - color.h).abs() < 1e-3, "the hue is kept");
            assert!(readable.l <= color.l, "only darker");
        }
    }

    #[test]
    fn readable_colors_and_dark_backgrounds_are_left_alone() {
        let green = Hsla::from(rgb(0x0015_803d));
        assert_eq!(on(green, white()), green, "already 5:1 on white");
        let dark = hsla(0., 0., 0.04, 1.);
        let light_green = Hsla::from(rgb(0x004a_de80));
        assert_eq!(on(light_green, dark), light_green);
    }
}
