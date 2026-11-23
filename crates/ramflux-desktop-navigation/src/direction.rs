// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The layout direction of the interface and the layout primitives that
//! follow it.
//!
//! There is one source of the direction: the interface locale. The UI crate
//! sets it with [`set_layout_direction`] whenever the locale changes; views
//! never decide the direction themselves. GPUI lays out every flex row left
//! to right and has no start/end edges, so mirroring is done here, in the
//! primitives:
//!
//! - [`h_flex`] is a row that runs from the start edge: left to right, or
//!   right to left (a reversed row) in a right-to-left interface. Pack its
//!   children with [`DirectionalExt::justify_row_start`] and
//!   [`DirectionalExt::justify_row_end`] (flex-relative, so they follow a
//!   reversed row); GPUI's `justify_start`/`justify_end` are physical.
//! - [`h_flex_ltr`] is a row that never mirrors: identifiers, codes, media
//!   controls and anything whose order is not reading order.
//! - [`DirectionalExt`] gives start/end padding, margins, borders, insets and
//!   text alignment.
//! - [`forward_icon`] and [`back_icon`] give the chevron that points forward
//!   or back in reading order.
//!
//! The direction is process-wide, like the locale it comes from.

use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::component::{IconName, StyledExt as _};
use gpui_kit::{DefiniteLength, Div, Length, Styled, TextAlign, div, px};

/// Which way the interface reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayoutDirection {
    /// Left to right.
    #[default]
    LeftToRight,
    /// Right to left (Arabic and the right-to-left pseudo-locale).
    RightToLeft,
}

impl LayoutDirection {
    /// Whether this direction is right to left.
    #[must_use]
    pub fn is_rtl(self) -> bool {
        self == Self::RightToLeft
    }
}

static RIGHT_TO_LEFT: AtomicBool = AtomicBool::new(false);

/// Sets the interface's layout direction (called by the locale switch only).
pub fn set_layout_direction(direction: LayoutDirection) {
    RIGHT_TO_LEFT.store(direction.is_rtl(), Ordering::Relaxed);
}

/// The interface's layout direction.
#[must_use]
pub fn layout_direction() -> LayoutDirection {
    if RIGHT_TO_LEFT.load(Ordering::Relaxed) {
        LayoutDirection::RightToLeft
    } else {
        LayoutDirection::LeftToRight
    }
}

/// Whether the interface reads right to left.
#[must_use]
pub fn is_rtl() -> bool {
    layout_direction().is_rtl()
}

/// A row that runs from the start edge and centers its children on the
/// cross axis: the direction-aware replacement of Kit's `h_flex`.
#[must_use]
pub fn h_flex() -> Div {
    h_flex_in(layout_direction())
}

/// A row that runs in `direction` whatever the interface's direction (a
/// message bubble follows its content, not the interface).
#[must_use]
pub fn h_flex_in(direction: LayoutDirection) -> Div {
    let row = div().h_flex();
    if direction.is_rtl() { row.flex_row_reverse() } else { row }
}

/// A row that always runs left to right: identifiers, codes, numbers and
/// controls whose order is not reading order.
#[must_use]
pub fn h_flex_ltr() -> Div {
    div().h_flex()
}

/// The chevron that points forward in reading order (a disclosure or "next").
#[must_use]
pub fn forward_icon() -> IconName {
    if is_rtl() { IconName::ChevronLeft } else { IconName::ChevronRight }
}

/// The chevron that points back in reading order ("back", "previous").
#[must_use]
pub fn back_icon() -> IconName {
    if is_rtl() { IconName::ChevronRight } else { IconName::ChevronLeft }
}

/// Start and end edges for any styled element; each resolves to left or
/// right by the interface's direction at render time.
pub trait DirectionalExt: Styled + Sized {
    /// Padding on the start edge.
    fn pad_start(mut self, length: impl Into<DefiniteLength>) -> Self {
        let length = Some(length.into());
        if is_rtl() {
            self.style().padding.right = length;
        } else {
            self.style().padding.left = length;
        }
        self
    }

    /// Padding on the end edge.
    fn pad_end(mut self, length: impl Into<DefiniteLength>) -> Self {
        let length = Some(length.into());
        if is_rtl() {
            self.style().padding.left = length;
        } else {
            self.style().padding.right = length;
        }
        self
    }

    /// Margin on the start edge.
    fn margin_start(mut self, length: impl Into<Length>) -> Self {
        let length = Some(length.into());
        if is_rtl() {
            self.style().margin.right = length;
        } else {
            self.style().margin.left = length;
        }
        self
    }

    /// Margin on the end edge.
    fn margin_end(mut self, length: impl Into<Length>) -> Self {
        let length = Some(length.into());
        if is_rtl() {
            self.style().margin.left = length;
        } else {
            self.style().margin.right = length;
        }
        self
    }

    /// A one-pixel border on the start edge.
    fn border_start_1(mut self) -> Self {
        let width = Some(px(1.).into());
        if is_rtl() {
            self.style().border_widths.right = width;
        } else {
            self.style().border_widths.left = width;
        }
        self
    }

    /// A one-pixel border on the end edge (a sidebar's divider).
    fn border_end_1(mut self) -> Self {
        let width = Some(px(1.).into());
        if is_rtl() {
            self.style().border_widths.left = width;
        } else {
            self.style().border_widths.right = width;
        }
        self
    }

    /// Absolute offset from the start edge.
    fn inset_start(mut self, length: impl Into<Length>) -> Self {
        let length = Some(length.into());
        if is_rtl() {
            self.style().inset.right = length;
        } else {
            self.style().inset.left = length;
        }
        self
    }

    /// Absolute offset from the end edge.
    fn inset_end(mut self, length: impl Into<Length>) -> Self {
        let length = Some(length.into());
        if is_rtl() {
            self.style().inset.left = length;
        } else {
            self.style().inset.right = length;
        }
        self
    }

    /// A row that runs from the start edge (for an element that is already
    /// a flex container).
    fn flex_row_start(self) -> Self {
        if is_rtl() { self.flex_row_reverse() } else { self.flex_row() }
    }

    /// Children packed toward the start of the row (follows a reversed row).
    fn justify_row_start(mut self) -> Self {
        self.style().justify_content = Some(gpui_kit::AlignContent::FlexStart);
        self
    }

    /// Children packed toward the end of the row (an own message's side).
    fn justify_row_end(mut self) -> Self {
        self.style().justify_content = Some(gpui_kit::AlignContent::FlexEnd);
        self
    }

    /// Text aligned to the start edge.
    fn text_start(self) -> Self {
        self.text_align(if is_rtl() { TextAlign::Right } else { TextAlign::Left })
    }

    /// Text aligned to the end edge (numbers in a column of figures).
    fn text_end(self) -> Self {
        self.text_align(if is_rtl() { TextAlign::Left } else { TextAlign::Right })
    }

    /// Left-aligned text in every interface direction: identifiers, codes,
    /// numbers and logs, which always read left to right.
    fn text_ltr(self) -> Self {
        self.text_align(TextAlign::Left)
    }

    /// Text aligned to the start of `direction` (a message follows its own
    /// direction, not the interface's).
    fn text_start_in(self, direction: LayoutDirection) -> Self {
        self.text_align(if direction.is_rtl() { TextAlign::Right } else { TextAlign::Left })
    }
}

impl<T: Styled> DirectionalExt for T {}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test owns the process-wide direction, so parallel tests never
    /// see another test's direction.
    #[test]
    fn primitives_mirror_only_in_a_right_to_left_interface() {
        set_layout_direction(LayoutDirection::LeftToRight);
        assert!(!is_rtl());
        assert!(matches!(forward_icon(), IconName::ChevronRight));
        assert!(matches!(back_icon(), IconName::ChevronLeft));
        let mut row = h_flex().pad_start(px(4.)).border_end_1().inset_start(px(2.));
        assert_eq!(row.style().flex_direction, Some(gpui_kit::FlexDirection::Row));
        assert!(row.style().padding.left.is_some() && row.style().padding.right.is_none());
        assert!(row.style().border_widths.right.is_some());
        assert!(row.style().inset.left.is_some());

        set_layout_direction(LayoutDirection::RightToLeft);
        assert!(is_rtl());
        assert!(matches!(forward_icon(), IconName::ChevronLeft));
        assert!(matches!(back_icon(), IconName::ChevronRight));
        let mut row = h_flex().pad_start(px(4.)).border_end_1().inset_start(px(2.));
        assert_eq!(row.style().flex_direction, Some(gpui_kit::FlexDirection::RowReverse));
        assert!(row.style().padding.right.is_some() && row.style().padding.left.is_none());
        assert!(row.style().border_widths.left.is_some());
        assert!(row.style().inset.right.is_some());
        let mut fixed = h_flex_ltr();
        assert_eq!(fixed.style().flex_direction, Some(gpui_kit::FlexDirection::Row));
        let mut message = h_flex_in(LayoutDirection::LeftToRight);
        assert_eq!(message.style().flex_direction, Some(gpui_kit::FlexDirection::Row));

        set_layout_direction(LayoutDirection::LeftToRight);
    }
}
