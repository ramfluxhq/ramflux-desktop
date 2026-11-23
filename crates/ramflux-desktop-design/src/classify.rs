// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Attribution of deviation regions to error classes by measurement.
//!
//! For each region the background of reference and capture is the most
//! common colour on a frame around the region, and ink is every pixel that
//! differs from it by more than 40/255. Comparing the ink boxes, pixel
//! counts and colours gives the class:
//!
//! | Class | Rule | Decision |
//! |---|---|---|
//! | `chrome` | inside the top-left 72x40 corner, ink on the board only (the macOS traffic lights) | accepted (lead) |
//! | `rasterisation` | ink boxes equal within 1 design px, ink amount within 20 %, same colours | accepted (lead) |
//! | `structure` | the region covers more than 15 % of the compared area | fix |
//! | `missing` | ink on the board, none in the capture, nor within 24 px | fix |
//! | `extra` | ink in the capture, none on the board, nor within 24 px | fix |
//! | `token` | ink boxes equal within 1 px but background or ink colour differs | fix |
//! | `spacing` | same ink size within 1 px, moved by more than 1 px | fix |
//! | `type` | same start and height within 1 px, width differs by up to 25 % | fix |
//! | `copy` | same start and height, width differs by more than 25 % | unreviewed (class hint) |
//! | `layout` | anything else | unreviewed (class hint) |
//!
//! Only the two lead decisions are accepted; nothing else is.

use image::{Rgba, RgbaImage};

use crate::compare::Rect;
use crate::deviations::Decision;

/// Reviewer recorded on entries attributed by measurement (`fix`); the two
/// accepted classes carry `lead`, whose decision they apply.
pub const REVIEWER: &str = "rf-design classify";

/// Ink threshold against the local background.
const INK: u8 = 40;

/// What was measured inside one region (design pixels).
#[derive(Debug, Clone, PartialEq)]
pub struct Measure {
    pub reference_ink: Option<(f64, f64, f64, f64)>,
    pub capture_ink: Option<(f64, f64, f64, f64)>,
    pub reference_pixels: u64,
    pub capture_pixels: u64,
    pub reference_background: Rgba<u8>,
    pub capture_background: Rgba<u8>,
    pub reference_colour: Rgba<u8>,
    pub capture_colour: Rgba<u8>,
    /// Ink within [`NEARBY`] design px around the region, per image: an
    /// element that moved further than its region is not missing.
    pub reference_nearby: u64,
    pub capture_nearby: u64,
}

/// How far around a region a moved element is looked for.
const NEARBY: u32 = 24;

fn max_channel(a: Rgba<u8>, b: Rgba<u8>) -> u8 {
    (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0)
}

fn hex(c: Rgba<u8>) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// Device-pixel bounds of `rect` (design px) clamped to the image.
fn bounds(image: &RgbaImage, rect: Rect, scale: u32) -> (u32, u32, u32, u32) {
    let (w, h) = image.dimensions();
    (
        (rect.x * scale).min(w),
        (rect.y * scale).min(h),
        ((rect.x + rect.w) * scale).min(w),
        ((rect.y + rect.h) * scale).min(h),
    )
}

/// The most common colour on the frame just outside `rect`.
fn background(image: &RgbaImage, rect: Rect, scale: u32) -> Rgba<u8> {
    let (x0, y0, x1, y1) = bounds(image, rect.grow(2), scale);
    let mut counts: std::collections::HashMap<[u8; 3], u32> = std::collections::HashMap::new();
    let mut add = |x: u32, y: u32| {
        let p = image.get_pixel(x, y);
        *counts.entry([p[0], p[1], p[2]]).or_default() += 1;
    };
    if x1 == 0 || y1 == 0 {
        return Rgba([255, 255, 255, 255]);
    }
    for x in x0..x1 {
        add(x, y0);
        add(x, y1 - 1);
    }
    for y in y0..y1 {
        add(x0, y);
        add(x1 - 1, y);
    }
    let best = counts.into_iter().max_by_key(|(c, n)| (*n, *c)).map(|(c, _)| c);
    best.map_or(Rgba([255, 255, 255, 255]), |c| Rgba([c[0], c[1], c[2], 255]))
}

type Ink = (Option<(f64, f64, f64, f64)>, u64, Rgba<u8>);

fn ink(image: &RgbaImage, rect: Rect, scale: u32, bg: Rgba<u8>) -> Ink {
    let (x0, y0, x1, y1) = bounds(image, rect, scale);
    let (mut minx, mut miny, mut maxx, mut maxy) = (u32::MAX, u32::MAX, 0, 0);
    let mut count = 0u64;
    let mut sum = [0u64; 3];
    for y in y0..y1 {
        for x in x0..x1 {
            let p = *image.get_pixel(x, y);
            if max_channel(p, bg) > INK {
                count += 1;
                minx = minx.min(x);
                miny = miny.min(y);
                maxx = maxx.max(x + 1);
                maxy = maxy.max(y + 1);
                for c in 0..3 {
                    sum[c] += u64::from(p[c]);
                }
            }
        }
    }
    if count == 0 {
        return (None, 0, bg);
    }
    let s = f64::from(scale);
    let mean = |c: usize| (sum[c] / count) as u8;
    (
        Some((f64::from(minx) / s, f64::from(miny) / s, f64::from(maxx) / s, f64::from(maxy) / s)),
        count,
        Rgba([mean(0), mean(1), mean(2), 255]),
    )
}

/// Measures `rect` (design px) in both images at `scale`.
pub fn measure(reference: &RgbaImage, capture: &RgbaImage, rect: Rect, scale: u32) -> Measure {
    let rb = background(reference, rect, scale);
    let cb = background(capture, rect, scale);
    let (ri, rn, rc) = ink(reference, rect, scale, rb);
    let (ci, cn, cc) = ink(capture, rect, scale, cb);
    let around = rect.grow(NEARBY);
    let (_, rnear, _) = ink(reference, around, scale, background(reference, around, scale));
    let (_, cnear, _) = ink(capture, around, scale, background(capture, around, scale));
    Measure {
        reference_ink: ri,
        capture_ink: ci,
        reference_pixels: rn,
        capture_pixels: cn,
        reference_background: rb,
        capture_background: cb,
        reference_colour: rc,
        capture_colour: cc,
        reference_nearby: rnear,
        capture_nearby: cnear,
    }
}

/// The attribution of one region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribution {
    pub class: &'static str,
    pub decision: Decision,
    pub reason: String,
}

/// Classifies a region of a board whose compared area is `area` design px².
pub fn classify(m: &Measure, rect: Rect, area: u64) -> Attribution {
    let fix = |class, reason: String| Attribution { class, decision: Decision::Fix, reason };
    let hint =
        |class, reason: String| Attribution { class, decision: Decision::Unreviewed, reason };
    let accept =
        |class, reason: String| Attribution { class, decision: Decision::Accepted, reason };
    let colours = format!(
        "board bg {} ink {}, capture bg {} ink {}",
        hex(m.reference_background),
        hex(m.reference_colour),
        hex(m.capture_background),
        hex(m.capture_colour)
    );
    if rect.x + rect.w <= 72 && rect.y + rect.h <= 40 && m.reference_pixels > 0 {
        let capture_corner = m.capture_pixels * 5 < m.reference_pixels.max(1);
        if capture_corner {
            return accept(
                "chrome",
                "macOS window traffic lights drawn on the board, absent from headless captures"
                    .into(),
            );
        }
    }
    if rect.area() * 100 > area * 15 {
        return fix("structure", format!("screen structure differs over {rect} ({colours})"));
    }
    let (Some(r), Some(c)) = (m.reference_ink, m.capture_ink) else {
        // Ink on one side only: missing or extra, unless the other image has
        // as much ink close by (the element moved further than its region).
        return match (m.reference_ink, m.capture_ink) {
            (Some(_), None) if m.capture_nearby * 2 < m.reference_pixels => {
                fix("missing", format!("drawn on the board, absent in the app at {rect}"))
            }
            (None, Some(_)) if m.reference_nearby * 2 < m.capture_pixels => {
                fix("extra", format!("drawn by the app, not on the board at {rect}"))
            }
            (Some(_), None) | (None, Some(_)) => hint(
                "layout",
                format!("element moved further than its region (ink within {NEARBY} px) at {rect}"),
            ),
            _ => fix("token", format!("surface colour differs at {rect} ({colours})")),
        };
    };
    let d = [c.0 - r.0, c.1 - r.1, c.2 - r.2, c.3 - r.3];
    let (rw, rh, cw, ch) = (r.2 - r.0, r.3 - r.1, c.2 - c.0, c.3 - c.1);
    let within = d.iter().all(|v| v.abs() <= 1.0);
    let amount = (m.capture_pixels as f64) / (m.reference_pixels.max(1) as f64);
    let ink_colour = max_channel(m.reference_colour, m.capture_colour);
    let bg_colour = max_channel(m.reference_background, m.capture_background);
    let shift = format!(
        "ink box board {:.1},{:.1} {:.1}x{:.1}, app {:.1},{:.1} {:.1}x{:.1}",
        r.0, r.1, rw, rh, c.0, c.1, cw, ch
    );
    if within && (0.8..=1.25).contains(&amount) && ink_colour <= 24 && bg_colour <= 6 {
        return accept(
            "rasterisation",
            format!("glyph rasterisation only: position and metrics within 1 px ({shift})"),
        );
    }
    if within {
        return fix("token", format!("same geometry, colour differs ({colours}; {shift})"));
    }
    if (cw - rw).abs() <= 1.0 && (ch - rh).abs() <= 1.0 {
        return fix(
            "spacing",
            format!("same element moved by {:+.1},{:+.1} px ({shift})", d[0], d[1]),
        );
    }
    if (ch - rh).abs() <= 1.0 && d[0].abs() <= 1.0 && d[1].abs() <= 1.0 && rw > 0.0 {
        let ratio = (cw - rw).abs() / rw;
        return if ratio > 0.25 {
            hint(
                "copy",
                format!(
                    "same start and height, width {ratio:.0}% off: text differs? ({shift})",
                    ratio = ratio * 100.0
                ),
            )
        } else {
            fix(
                "type",
                format!(
                    "same start and height, width {:+.1} px: type size or weight ({shift})",
                    cw - rw
                ),
            )
        };
    }
    if rh <= 32.0 && (ch - rh).abs() > 1.0 && (ch - rh).abs() <= 6.0 && d[0].abs() <= 2.0 {
        return fix("type", format!("text height {:+.1} px: type scale ({shift})", ch - rh));
    }
    hint("layout", format!("geometry differs ({shift})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas() -> RgbaImage {
        RgbaImage::from_pixel(200, 100, Rgba([252, 251, 249, 255]))
    }

    fn block(image: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, c: Rgba<u8>) {
        for yy in y..y + h {
            for xx in x..x + w {
                image.put_pixel(xx, yy, c);
            }
        }
    }

    const DARK: Rgba<u8> = Rgba([22, 22, 22, 255]);
    const AREA: u64 = 200 * 100 * 10;

    #[test]
    fn a_moved_element_is_spacing_and_a_missing_one_is_missing() {
        let mut a = canvas();
        let mut b = canvas();
        block(&mut a, 50, 40, 20, 10, DARK);
        block(&mut b, 54, 43, 20, 10, DARK);
        let rect = Rect { x: 48, y: 38, w: 30, h: 18 };
        let at = classify(&measure(&a, &b, rect, 1), rect, AREA);
        assert_eq!(at.class, "spacing", "{}", at.reason);
        assert_eq!(at.decision, Decision::Fix);
        let empty = canvas();
        assert_eq!(classify(&measure(&a, &empty, rect, 1), rect, AREA).class, "missing");
        assert_eq!(classify(&measure(&empty, &a, rect, 1), rect, AREA).class, "extra");
    }

    #[test]
    fn an_element_moved_beyond_its_region_is_not_missing() {
        let mut a = canvas();
        let mut b = canvas();
        block(&mut a, 50, 40, 20, 10, DARK);
        block(&mut b, 85, 40, 20, 10, DARK);
        let left = Rect { x: 49, y: 39, w: 22, h: 12 };
        let at = classify(&measure(&a, &b, left, 1), left, AREA);
        assert_eq!((at.class, at.decision), ("layout", Decision::Unreviewed), "{}", at.reason);
    }

    #[test]
    fn rasterisation_and_chrome_are_accepted_colour_is_token() {
        let mut a = canvas();
        let mut b = canvas();
        block(&mut a, 50, 40, 20, 10, DARK);
        block(&mut b, 50, 40, 21, 10, DARK);
        let rect = Rect { x: 48, y: 38, w: 30, h: 18 };
        let at = classify(&measure(&a, &b, rect, 1), rect, AREA);
        assert_eq!((at.class, at.decision), ("rasterisation", Decision::Accepted), "{}", at.reason);
        let mut c = canvas();
        block(&mut c, 50, 40, 20, 10, Rgba([200, 40, 40, 255]));
        assert_eq!(classify(&measure(&a, &c, rect, 1), rect, AREA).class, "token");
        let mut lights = canvas();
        block(&mut lights, 8, 20, 10, 10, Rgba([200, 200, 200, 255]));
        let corner = Rect { x: 6, y: 18, w: 14, h: 14 };
        let at = classify(&measure(&lights, &canvas(), corner, 1), corner, AREA);
        assert_eq!((at.class, at.decision), ("chrome", Decision::Accepted));
    }

    #[test]
    fn text_width_and_height_changes() {
        let mut a = canvas();
        let mut b = canvas();
        block(&mut a, 50, 40, 40, 12, DARK);
        block(&mut b, 50, 40, 46, 12, DARK);
        let rect = Rect { x: 48, y: 38, w: 52, h: 18 };
        assert_eq!(classify(&measure(&a, &b, rect, 1), rect, AREA).class, "type");
        let mut c = canvas();
        block(&mut c, 50, 40, 70, 12, DARK);
        let wide = Rect { x: 48, y: 38, w: 80, h: 18 };
        let at = classify(&measure(&a, &c, wide, 1), wide, AREA);
        assert_eq!((at.class, at.decision), ("copy", Decision::Unreviewed));
        let big = Rect { x: 0, y: 0, w: 150, h: 80 };
        assert_eq!(classify(&measure(&a, &c, big, 1), big, AREA / 10).class, "structure");
    }
}
