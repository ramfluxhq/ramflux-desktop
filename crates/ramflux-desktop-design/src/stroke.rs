// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Stroke alignment: the lead's third accepted class.
//!
//! The design draws a 1 px edge as a stroke centred on the element's outline
//! (half outside, half inside); the UI kit draws a border inside the box. The
//! same edge colour and the same outer geometry then still leave a 1 px band
//! of differing pixels along the outline. A region is `stroke` when all of
//! these hold, measured on both images at their own scale:
//!
//! 1. The capture's element has an edge: its outermost ink pixel on every
//!    side differs from the background outside and the fill inside by more
//!    than [`CONTRAST`] in at least one channel.
//! 2. Same outer geometry: the board's ink box (with the outer half of its
//!    stroke) is within 1 design px of the capture's on every side.
//! 3. Same colour token and width: across the edge, on the middle 40 % of
//!    every side, the board's pixels hold as much of the capture's edge
//!    colour as the capture's own (the coverage of the edge colour over the
//!    local background outside and the fill inside, summed across the band,
//!    for every channel with enough contrast), within [`COVERAGE`] of a
//!    stroke.
//! 4. Every pixel that differs beyond the comparison threshold lies in the
//!    band 1 design px either side of the capture's outline, except inside
//!    the element, where any difference must itself be glyph rasterisation
//!    (checked by the caller on the interior).
//!
//! A different colour, a wider or missing edge, a moved or resized element
//! or any difference outside the band fails the rule.

use image::{Rgba, RgbaImage};

use crate::compare::Rect;

/// Contrast an edge needs against both its sides (channel units).
pub const CONTRAST: u8 = 40;

/// Allowed difference of the edge colour's coverage, as a share of one
/// design px of stroke.
pub const COVERAGE: f64 = 0.25;

/// The comparison's channel threshold (see `compare`).
const THRESHOLD: u8 = 32;

/// A measured stroke fit (device px unless noted).
#[derive(Debug, Clone, PartialEq)]
pub struct Fit {
    /// The capture's edge colour.
    pub edge: Rgba<u8>,
    /// Mean coverage of the edge colour across the band, board and capture,
    /// in design px.
    pub board_coverage: f64,
    pub capture_coverage: f64,
    /// The interior (design px) the caller checks for rasterisation, when
    /// pixels differ inside the element.
    pub interior: Option<Rect>,
}

fn channel(c: Rgba<u8>, k: usize) -> f64 {
    f64::from(c[k])
}

fn diff(a: Rgba<u8>, b: Rgba<u8>) -> u8 {
    (0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0)
}

fn get(image: &RgbaImage, x: i64, y: i64) -> Option<Rgba<u8>> {
    let (w, h) = image.dimensions();
    if x < 0 || y < 0 || x >= i64::from(w) || y >= i64::from(h) {
        return None;
    }
    Some(*image.get_pixel(x as u32, y as u32))
}

/// Ink box (device px, exclusive end) of `image` inside `area` against `bg`.
fn ink_box(image: &RgbaImage, area: (i64, i64, i64, i64), bg: Rgba<u8>) -> Option<[i64; 4]> {
    let mut b = [i64::MAX, i64::MAX, i64::MIN, i64::MIN];
    for y in area.1..area.3 {
        for x in area.0..area.2 {
            let p = get(image, x, y)?;
            if diff(p, bg) > CONTRAST {
                b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
            }
        }
    }
    (b[0] < b[2]).then_some(b)
}

/// One sample line across an edge: `at(i)` is the pixel `i` device px from
/// the capture's edge pixel, positive inwards.
struct Line<'a> {
    image: &'a RgbaImage,
    origin: (i64, i64),
    inward: (i64, i64),
}

impl Line<'_> {
    fn at(&self, i: i64) -> Option<Rgba<u8>> {
        get(self.image, self.origin.0 + self.inward.0 * i, self.origin.1 + self.inward.1 * i)
    }
}

/// Coverage of `edge` across the band of one line, per channel (`None` for a
/// channel without enough contrast on both sides): outside pixels over the
/// local background, inside pixels over the local fill. `bg` and `fill` are
/// the capture's, so both images are measured against the same colours.
fn coverage(
    line: &Line<'_>,
    edge: Rgba<u8>,
    bg: Rgba<u8>,
    fill: Rgba<u8>,
    scale: i64,
) -> Option<[Option<f64>; 3]> {
    let mut per_channel = [None; 3];
    for (k, slot) in per_channel.iter_mut().enumerate() {
        let (out, inn) = (channel(edge, k) - channel(bg, k), channel(edge, k) - channel(fill, k));
        if out.abs() < f64::from(CONTRAST) || inn.abs() < f64::from(CONTRAST) {
            continue;
        }
        let mut sum = 0.0;
        for i in -scale..2 * scale {
            let p = channel(line.at(i)?, k);
            let (base, span) = if i < 0 { (channel(bg, k), out) } else { (channel(fill, k), inn) };
            sum += ((p - base) / span).clamp(0.0, 1.0);
        }
        *slot = Some(sum);
    }
    per_channel.iter().any(Option::is_some).then_some(per_channel)
}

/// One side of the capture's box: its first edge pixel, the inward step,
/// the span along the side and whether the side is vertical.
type Side = ((i64, i64), (i64, i64), i64, i64, bool);

/// Measures the stroke fit of `rect` (design px); `None` when the rule does
/// not hold.
pub fn fit(reference: &RgbaImage, capture: &RgbaImage, rect: Rect, scale: u32) -> Option<Fit> {
    let s = i64::from(scale);
    let area = rect.grow(1);
    let area = (
        i64::from(area.x) * s,
        i64::from(area.y) * s,
        i64::from(area.x + area.w) * s,
        i64::from(area.y + area.h) * s,
    );
    // The local background: the capture's pixel just outside the area.
    let bg = get(capture, area.0 - 1, area.1 - 1).or_else(|| get(capture, area.2, area.3))?;
    let rbg = get(reference, area.0 - 1, area.1 - 1).or_else(|| get(reference, area.2, area.3))?;
    if diff(bg, rbg) > 6 {
        return None;
    }
    let c = ink_box(capture, area, bg)?;
    let r = ink_box(reference, area, rbg)?;
    // 2. Same outer geometry within 1 design px.
    if (0..4).any(|i| (c[i] - r[i]).abs() > s) {
        return None;
    }
    let (cw, ch) = (c[2] - c[0], c[3] - c[1]);
    if cw < 6 * s || ch < 6 * s {
        return None;
    }
    // 1 and 3, on the middle 40 % of every side.
    let sides: [Side; 4] = [
        ((c[0], c[1]), (1, 0), c[1], c[3], true),
        ((c[2] - 1, c[1]), (-1, 0), c[1], c[3], true),
        ((c[0], c[1]), (0, 1), c[0], c[2], false),
        ((c[0], c[3] - 1), (0, -1), c[0], c[2], false),
    ];
    let mut edge = None;
    let (mut board_sum, mut capture_sum, mut n) = (0.0, 0.0, 0.0);
    for (origin, inward, from, to, vertical) in sides {
        let len = to - from;
        for t in (from + len * 3 / 10)..(from + len * 7 / 10) {
            let o = if vertical { (origin.0, t) } else { (t, origin.1) };
            let cap = Line { image: capture, origin: o, inward };
            let board = Line { image: reference, origin: o, inward };
            let e = cap.at(0)?;
            if diff(e, cap.at(-3 * s)?) <= CONTRAST || diff(e, cap.at(3 * s)?) <= CONTRAST {
                return None;
            }
            match edge {
                None => edge = Some(e),
                Some(first) if diff(first, e) > 24 => return None,
                Some(_) => {}
            }
            let (bg, fill) = (cap.at(-3 * s)?, cap.at(3 * s)?);
            let cc = coverage(&cap, e, bg, fill, s)?;
            let bc = coverage(&board, e, bg, fill, s)?;
            for (cc, bc) in cc.iter().zip(bc.iter()) {
                if let (Some(cc), Some(bc)) = (cc, bc) {
                    if (cc - bc).abs() > COVERAGE * f64::from(scale) {
                        return None;
                    }
                    capture_sum += cc;
                    board_sum += bc;
                    n += 1.0;
                }
            }
        }
    }
    let edge = edge?;
    if n == 0.0 {
        return None;
    }
    // 4. Differences only in the band, or inside the element.
    let inner = [c[0] + 2 * s, c[1] + 2 * s, c[2] - 2 * s, c[3] - 2 * s];
    let mut interior = false;
    for y in area.1..area.3 {
        for x in area.0..area.2 {
            if diff(get(reference, x, y)?, get(capture, x, y)?) <= THRESHOLD {
                continue;
            }
            let in_band = x >= c[0] - s && x < c[2] + s && y >= c[1] - s && y < c[3] + s;
            let inside = x >= inner[0] && x < inner[2] && y >= inner[1] && y < inner[3];
            if inside {
                interior = true;
            } else if !in_band || (x >= c[0] + s && x < c[2] - s && y >= c[1] + s && y < c[3] - s) {
                return None;
            }
        }
    }
    let to_design = |v: i64| u32::try_from(v / s).unwrap_or(0);
    let interior = interior.then(|| Rect {
        x: to_design(inner[0]),
        y: to_design(inner[1]),
        w: to_design(inner[2] - inner[0]),
        h: to_design(inner[3] - inner[1]),
    });
    Some(Fit {
        edge,
        board_coverage: board_sum / n / f64::from(scale),
        capture_coverage: capture_sum / n / f64::from(scale),
        interior,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BG: Rgba<u8> = Rgba([252, 251, 249, 255]);
    const EDGE: Rgba<u8> = Rgba([142, 142, 142, 255]);
    const FILL: Rgba<u8> = Rgba([255, 255, 255, 255]);

    fn canvas(scale: u32) -> RgbaImage {
        RgbaImage::from_pixel(60 * scale, 40 * scale, BG)
    }

    fn rect(image: &mut RgbaImage, b: [u32; 4], c: Rgba<u8>) {
        for y in b[1]..b[3] {
            for x in b[0]..b[2] {
                image.put_pixel(x, y, c);
            }
        }
    }

    /// A box at 10,10 18x18 design px with an inside 1 px border (capture),
    /// at `scale`.
    fn inside(scale: u32, edge: Rgba<u8>) -> RgbaImage {
        let s = scale;
        let mut i = canvas(s);
        rect(&mut i, [10 * s, 10 * s, 28 * s, 28 * s], edge);
        rect(&mut i, [11 * s, 11 * s, 27 * s, 27 * s], FILL);
        i
    }

    /// The same box with a centred stroke (board) of `width` design px, at
    /// 2x: half outside the box (20..56 device px), half inside.
    fn centred(edge: Rgba<u8>, width: u32) -> RgbaImage {
        let mut i = canvas(2);
        rect(&mut i, [20 - width, 20 - width, 56 + width, 56 + width], edge);
        rect(&mut i, [20 + width, 20 + width, 56 - width, 56 - width], FILL);
        i
    }

    const AREA: Rect = Rect { x: 9, y: 9, w: 20, h: 20 };

    #[test]
    fn a_centred_stroke_of_the_same_token_fits() {
        let fit = fit(&centred(EDGE, 1), &inside(2, EDGE), AREA, 2);
        let fit = fit.expect("stroke alignment");
        assert!((fit.board_coverage - fit.capture_coverage).abs() <= COVERAGE);
        assert_eq!(fit.interior, None);
    }

    #[test]
    fn another_colour_a_wider_stroke_or_a_moved_box_does_not() {
        let other = Rgba([200, 60, 60, 255]);
        assert_eq!(fit(&centred(other, 1), &inside(2, EDGE), AREA, 2), None, "colour");
        assert_eq!(fit(&centred(EDGE, 2), &inside(2, EDGE), AREA, 2), None, "width");
        let mut moved = canvas(2);
        rect(&mut moved, [23, 19, 61, 57], EDGE);
        rect(&mut moved, [25, 21, 59, 55], FILL);
        assert_eq!(fit(&moved, &inside(2, EDGE), AREA, 2), None, "moved by 2 px");
        let mut missing = canvas(2);
        rect(&mut missing, [20, 20, 56, 56], FILL);
        assert_eq!(fit(&missing, &inside(2, EDGE), AREA, 2), None, "no edge on the board");
    }

    #[test]
    fn a_difference_outside_the_band_fails_and_inside_is_left_to_the_caller() {
        let mut board = centred(EDGE, 1);
        rect(&mut board, [36, 36, 40, 40], Rgba([22, 22, 22, 255]));
        let fit = fit(&board, &inside(2, EDGE), AREA, 2).expect("interior mark");
        assert!(fit.interior.is_some());
        let mut outside = centred(EDGE, 1);
        rect(&mut outside, [2, 2, 8, 8], Rgba([22, 22, 22, 255]));
        let wide = Rect { x: 0, y: 0, w: 30, h: 30 };
        assert_eq!(super::fit(&outside, &inside(2, EDGE), wide, 2), None);
    }
}
