// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The design's spacing and type scale, in design pixels.
//!
//! Views size everything from here instead of the Kit's Tailwind utilities
//! (`gap_2()`, `px_3()`, `text_sm()`): those are multiples of the rem, and
//! the Kit rem is 15 px, so they land on 3.75/7.5/11.25 px spacing and
//! 11.25/13.125/16.875 px type, none of which the design uses.
//!
//! - Spacing follows the design's 4 px grid: [`space`]`(n)` is n × 4 px,
//!   the step the utilities were written in (`gap_2()` → `gap(space(2.))`).
//! - Type sizes are the design variables (`rf-size-*`, unthemed).
//! - Any other design length goes through [`design_px`].
//! - Text styles ([`TextStyle`]) put a type size together with its weight
//!   and line height; [`DesignText::design_text`] applies one. The line
//!   height is the interface face's natural one ([`LINE_RATIO`], the
//!   metrics of `rf-font`), which the boards measure (13 px text on a 19 px
//!   line, 14 px button labels on 20 px); GPUI's default is 1.618.
//!
//! The guard `rf-design tokens --check` (also a test of the design crate)
//! fails when UI code uses a raw rem-based spacing or type utility, or
//! divides design pixels by 16.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::{Font, FontFallbacks, FontFeatures, FontStyle, FontWeight, Rems, Styled};

use crate::generated::rf_tokens::RFTheme;

/// Design pixels per rem: the Kit theme's font size (`font.size` of the
/// generated themes, `rf-size-body`).
pub const REM_PX: f32 = 15.0;

/// One step of the design's spacing grid, in design pixels.
pub const STEP_PX: f32 = 4.0;

/// A length of `px` design pixels, in rems (scales with the interface).
#[must_use]
pub fn design_px(px: f32) -> Rems {
    gpui_kit::rems(px / REM_PX)
}

/// `steps` steps of the 4 px grid (`space(2.)` is 8 design px).
#[must_use]
pub fn space(steps: f32) -> Rems {
    design_px(steps * STEP_PX)
}

/// The unthemed type sizes and faces of the design variables.
static SIZES: LazyLock<RFTheme> = LazyLock::new(RFTheme::light);

/// `rf-size-caption` (11 px).
#[must_use]
pub fn text_caption() -> Rems {
    design_px(SIZES.size_caption)
}

/// `rf-size-meta` (12 px).
#[must_use]
pub fn text_meta() -> Rems {
    design_px(SIZES.size_meta)
}

/// `rf-size-secondary` (13 px).
#[must_use]
pub fn text_secondary() -> Rems {
    design_px(SIZES.size_secondary)
}

/// `rf-size-body` (15 px).
#[must_use]
pub fn text_body() -> Rems {
    design_px(SIZES.size_body)
}

/// `rf-size-subheading` (17 px).
#[must_use]
pub fn text_subheading() -> Rems {
    design_px(SIZES.size_subheading)
}

/// The natural line height of the interface face (`rf-font`, Noto Sans SC:
/// ascender 1160 and descender 288 per 1000 units), as a multiple of the
/// type size. The design leaves line heights to the face.
pub const LINE_RATIO: f32 = 1.448;

/// A design text style: type size, weight and line height, in design px.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub size: f32,
    pub weight: FontWeight,
    pub line: f32,
}

impl TextStyle {
    /// `size` at `weight` on the face's natural line, rounded to the pixel.
    #[must_use]
    pub fn natural(size: f32, weight: FontWeight) -> Self {
        Self { size, weight, line: (size * LINE_RATIO).round() }
    }

    /// The same style at another weight.
    #[must_use]
    pub fn weight(self, weight: FontWeight) -> Self {
        Self { weight, ..self }
    }

    /// The same style on another line height (design px).
    #[must_use]
    pub fn line(self, line: f32) -> Self {
        Self { line, ..self }
    }
}

/// The design's text styles, one per type size (`rf-size-*`).
pub mod style {
    use super::{FontWeight, SIZES, TextStyle};

    /// 11 px regular (`rf-size-caption`).
    #[must_use]
    pub fn caption() -> TextStyle {
        TextStyle::natural(SIZES.size_caption, FontWeight::NORMAL)
    }
    /// 12 px regular (`rf-size-meta`).
    #[must_use]
    pub fn meta() -> TextStyle {
        TextStyle::natural(SIZES.size_meta, FontWeight::NORMAL)
    }
    /// 13 px regular (`rf-size-secondary`).
    #[must_use]
    pub fn secondary() -> TextStyle {
        TextStyle::natural(SIZES.size_secondary, FontWeight::NORMAL)
    }
    /// 14 px medium (`rf-size-button`): button labels.
    #[must_use]
    pub fn button() -> TextStyle {
        TextStyle::natural(SIZES.size_button, FontWeight::MEDIUM)
    }
    /// 15 px regular (`rf-size-body`).
    #[must_use]
    pub fn body() -> TextStyle {
        TextStyle::natural(SIZES.size_body, FontWeight::NORMAL)
    }
    /// 17 px semibold (`rf-size-subheading`).
    #[must_use]
    pub fn subheading() -> TextStyle {
        TextStyle::natural(SIZES.size_subheading, FontWeight::SEMIBOLD)
    }
    /// 20 px semibold (`rf-size-title`).
    #[must_use]
    pub fn title() -> TextStyle {
        TextStyle::natural(SIZES.size_title, FontWeight::SEMIBOLD)
    }
    /// 24 px semibold (`rf-size-page`).
    #[must_use]
    pub fn page() -> TextStyle {
        TextStyle::natural(SIZES.size_page, FontWeight::SEMIBOLD)
    }
}

/// Whether the interface language is set in the CJK face.
static CJK_INTERFACE: AtomicBool = AtomicBool::new(false);

/// Sets the script of the interface language for [`text_font`]: the host
/// calls it when the locale changes (Chinese, Japanese and Korean are CJK).
pub fn set_cjk_interface(cjk: bool) {
    CJK_INTERFACE.store(cjk, Ordering::Relaxed);
}

/// The face of a design text style at `weight`.
///
/// The boards set CJK copy in `rf-font` (Noto Sans SC, its own Latin
/// glyphs included) and Latin copy in `rf-font-latin` (Hanken Grotesk). The
/// face also sets the line's metrics: CJK text on a Hanken Grotesk line sits
/// about 0.09 em higher than on the boards (ascent 1.0 em against Noto Sans
/// SC's 1.16 em). So in a CJK interface a text style takes `rf-font` first,
/// otherwise `rf-font-latin` with `rf-font` for the glyphs it lacks.
#[must_use]
pub fn text_font(weight: FontWeight) -> Font {
    let (family, fallback) = if CJK_INTERFACE.load(Ordering::Relaxed) {
        (SIZES.font, SIZES.font_latin)
    } else {
        (SIZES.font_latin, SIZES.font)
    };
    Font {
        family: family.into(),
        features: FontFeatures::default(),
        fallbacks: Some(FontFallbacks::from_fonts(vec![fallback.to_owned()])),
        weight,
        style: FontStyle::Normal,
    }
}

/// Applies a [`TextStyle`] to an element.
pub trait DesignText: Styled + Sized {
    /// Face ([`text_font`]), size, weight and line height of `style`.
    #[must_use]
    fn design_text(self, style: TextStyle) -> Self {
        self.font(text_font(style.weight))
            .text_size(design_px(style.size))
            .font_weight(style.weight)
            .line_height(design_px(style.line))
    }
}

impl<T: Styled> DesignText for T {}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(rems: Rems) -> f32 {
        rems.0 * REM_PX
    }

    #[test]
    fn spacing_is_on_the_four_pixel_grid() {
        assert!((px(space(1.)) - 4.0).abs() < 1e-4);
        assert!((px(space(2.)) - 8.0).abs() < 1e-4);
        assert!((px(space(0.5)) - 2.0).abs() < 1e-4);
        assert!((px(space(12.)) - 48.0).abs() < 1e-4);
    }

    #[test]
    fn type_sizes_are_the_design_variables() {
        for (size, want) in [
            (text_caption(), 11.0),
            (text_meta(), 12.0),
            (text_secondary(), 13.0),
            (text_body(), 15.0),
            (text_subheading(), 17.0),
        ] {
            assert!((px(size) - want).abs() < 1e-4, "{} != {want}", px(size));
        }
    }

    #[test]
    fn text_styles_sit_on_the_faces_natural_line() {
        // Measured on the boards: 13 px text on 19 px lines (H04), 14 px
        // button labels on 20 px (00B, 00D).
        assert_eq!(style::secondary().line, 19.0);
        assert_eq!(style::button().line, 20.0);
        assert_eq!(style::button().weight, FontWeight::MEDIUM);
        assert_eq!(style::caption().line, 16.0);
        assert_eq!(style::body().line, 22.0);
        assert_eq!(style::body().line(24.0).line, 24.0);
    }
}
