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
//!
//! The guard `rf-design tokens --check` (also a test of the design crate)
//! fails when UI code uses a raw rem-based spacing or type utility, or
//! divides design pixels by 16.

use std::sync::LazyLock;

use gpui_kit::Rems;

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

/// The unthemed type sizes of the design variables.
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
}
