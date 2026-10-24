// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! A QR code of a short text (the user's own contact card), drawn as one
//! canvas of square modules.
//!
//! Colors are an audited exception to theme tokens: a QR code's color is
//! its data, and scanners need dark modules on a light field with a quiet
//! zone around it in every theme, so the code is always black on white.
//! The element is an image with an accessible name; the text it encodes is
//! shown and copyable next to it.

use gpui_kit::TestSupportExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Bounds, Role, SharedString, canvas, div, fill, point, px, size};
use qrcodegen::{QrCode, QrCodeEcc};

/// Light modules around the code, in modules (the QR specification's
/// minimum).
const QUIET_ZONE: i32 = 4;

/// The dark modules of `text` as rows of booleans (quiet zone excluded),
/// or `None` when the text does not fit a QR code.
pub fn modules(text: &str) -> Option<Vec<Vec<bool>>> {
    let code = QrCode::encode_text(text, QrCodeEcc::Medium).ok()?;
    let side = code.size();
    Some((0..side).map(|y| (0..side).map(|x| code.get_module(x, y)).collect()).collect())
}

/// The QR code of `text` filling a square of the parent's width, named
/// `label` for assistive technology; `None` when the text does not fit.
pub fn render(id: &'static str, text: &str, label: SharedString) -> Option<AnyElement> {
    let rows = modules(text)?;
    let drawn = canvas(
        move |_, _, _| rows,
        move |bounds: Bounds<gpui_kit::Pixels>, rows, window, _| {
            let side = i32::try_from(rows.len()).unwrap_or(0) + 2 * QUIET_ZONE;
            if side <= 0 {
                return;
            }
            let extent = bounds.size.width.min(bounds.size.height);
            // Whole device pixels per module keep the modules crisp.
            let cell = (f32::from(extent) * window.scale_factor() / side as f32).floor().max(1.0)
                / window.scale_factor();
            let total = px(cell * side as f32);
            let origin = point(
                bounds.origin.x + (bounds.size.width - total) / 2.0,
                bounds.origin.y + (bounds.size.height - total) / 2.0,
            );
            window.paint_quad(fill(Bounds::new(origin, size(total, total)), gpui_kit::white()));
            for (y, row) in rows.iter().enumerate() {
                for (x, dark) in row.iter().enumerate() {
                    if !*dark {
                        continue;
                    }
                    let at = point(
                        origin.x + px(cell * (x as f32 + QUIET_ZONE as f32)),
                        origin.y + px(cell * (y as f32 + QUIET_ZONE as f32)),
                    );
                    window.paint_quad(fill(
                        Bounds::new(at, size(px(cell), px(cell))),
                        gpui_kit::black(),
                    ));
                }
            }
        },
    )
    .size_full();
    Some(
        div()
            .id(id)
            .role(Role::Image)
            .aria_label(label)
            .test_support()
            .w_full()
            .h_56()
            .child(drawn)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_card_becomes_a_square_code_and_an_oversized_text_none() {
        let rows = modules("ramflux:contact/abc").expect("fits");
        assert!(rows.len() >= 21 && rows.iter().all(|r| r.len() == rows.len()));
        assert!(rows[0][0], "finder pattern corner is dark");
        assert!(modules(&"x".repeat(5000)).is_none());
    }
}
