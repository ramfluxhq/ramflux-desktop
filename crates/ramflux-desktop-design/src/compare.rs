// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Pixel comparison of a capture with its design reference.
//!
//! 1. Both images are brought to the same scale (a 2x capture is compared
//!    with the 2x reference when one exists, otherwise box-averaged to 1x)
//!    and aligned at the top-left corner. A reference taller than the
//!    capture is compared over the capture's height (`top-900`).
//! 2. A pixel differs when its largest channel difference exceeds the
//!    threshold (32/255) and the other image has no pixel within the
//!    threshold in its neighbourhood (one design pixel), in both directions.
//!    One-pixel anti-aliasing and edge shifts therefore pass; larger shifts
//!    do not.
//! 3. Differing pixels are grouped into 4 design-pixel cells, cells into
//!    8-connected regions, and regions closer than 8 design pixels are
//!    merged. A region wider or taller than 2 design pixels is a deviation;
//!    smaller ones are anti-aliasing noise and only count in the ratio.
//! 4. The perceptual score is the mean SSIM of the luminance over 8x8
//!    windows at one pixel per design pixel (1.0 is identical).
//!
//! Coordinates in results are design pixels.

use image::{Rgba, RgbaImage};
use serde::{Deserialize, Serialize};

/// Comparison parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Options {
    /// Largest channel difference that still counts as equal (0..=255).
    pub threshold: u8,
    /// Neighbourhood radius in design pixels for the anti-aliasing tolerance.
    pub neighbourhood: u32,
    /// Cell size in design pixels used to group differing pixels.
    pub cell: u32,
    /// Regions closer than this many design pixels are merged.
    pub merge_gap: u32,
    /// A region larger than this in either direction is a deviation.
    pub min_extent: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self { threshold: 32, neighbourhood: 1, cell: 4, merge_gap: 8, min_extent: 2 }
    }
}

/// A rectangle in design pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn grow(self, by: u32) -> Self {
        let x = self.x.saturating_sub(by);
        let y = self.y.saturating_sub(by);
        Self { x, y, w: self.x + self.w + by - x, h: self.y + self.h + by - y }
    }

    pub fn contains(self, x: u32, y: u32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    pub fn intersects(self, other: Self) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }

    pub fn union(self, other: Self) -> Self {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let r = (self.x + self.w).max(other.x + other.w);
        let b = (self.y + self.h).max(other.y + other.h);
        Self { x, y, w: r - x, h: b - y }
    }

    pub fn area(self) -> u64 {
        u64::from(self.w) * u64::from(self.h)
    }
}

impl std::fmt::Display for Rect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{},{} {}x{}", self.x, self.y, self.w, self.h)
    }
}

/// A group of differing pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Region {
    /// Bounding box in design pixels.
    pub rect: Rect,
    /// Differing pixels inside it, in device pixels of the compared scale.
    pub pixels: u64,
}

/// How the reference was cropped to the capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Crop {
    /// Same size, nothing cropped.
    Full,
    /// The reference is a taller canvas; its top part (the capture's height)
    /// is compared.
    Top,
    /// The sizes disagree otherwise; the overlapping top-left area is
    /// compared and the board fails on size.
    Mismatch,
}

/// The result of comparing one capture with one reference.
#[derive(Debug, Clone)]
pub struct Comparison {
    /// Device pixels per design pixel of the compared images.
    pub scale: u32,
    /// Compared area in device pixels.
    pub width: u32,
    pub height: u32,
    pub crop: Crop,
    /// Pixels that differ beyond the anti-aliasing tolerance.
    pub differing: u64,
    /// Pixels whose channels differ beyond the threshold at all.
    pub strict_differing: u64,
    /// Pixels whose channels differ beyond [`TINT_THRESHOLD`]: surface and
    /// background colours too close to pass the main threshold.
    pub tint_differing: u64,
    pub ssim: f64,
    /// Deviation regions, largest first.
    pub regions: Vec<Region>,
    /// Regions at or under the minimum extent (anti-aliasing noise).
    pub noise_regions: usize,
    /// Per compared pixel: 0 when equal, else the largest channel difference.
    pub mask: Vec<u8>,
}

impl Comparison {
    pub fn compared(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// Share of differing pixels (0..=1).
    pub fn ratio(&self) -> f64 {
        ratio(self.differing, self.compared())
    }

    pub fn strict_ratio(&self) -> f64 {
        ratio(self.strict_differing, self.compared())
    }

    pub fn tint_ratio(&self) -> f64 {
        ratio(self.tint_differing, self.compared())
    }

    /// Differing pixels inside `rect` (design pixels).
    pub fn differing_in(&self, rect: Rect) -> u64 {
        let s = self.scale;
        let x0 = (rect.x * s).min(self.width);
        let y0 = (rect.y * s).min(self.height);
        let x1 = ((rect.x + rect.w) * s).min(self.width);
        let y1 = ((rect.y + rect.h) * s).min(self.height);
        let mut count = 0;
        for y in y0..y1 {
            let row = (y * self.width) as usize;
            count +=
                self.mask[row + x0 as usize..row + x1 as usize].iter().filter(|&&d| d > 0).count()
                    as u64;
        }
        count
    }

    /// Differing pixels inside `region` that also lie in one of `rects`.
    pub fn differing_covered(&self, region: Rect, rects: &[Rect]) -> u64 {
        let s = self.scale;
        let x0 = (region.x * s).min(self.width);
        let y0 = (region.y * s).min(self.height);
        let x1 = ((region.x + region.w) * s).min(self.width);
        let y1 = ((region.y + region.h) * s).min(self.height);
        let mut count = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                if self.mask[(y * self.width + x) as usize] > 0
                    && rects.iter().any(|r| r.contains(x / s, y / s))
                {
                    count += 1;
                }
            }
        }
        count
    }
}

fn ratio(part: u64, whole: u64) -> f64 {
    if whole == 0 { 0.0 } else { part as f64 / whole as f64 }
}

/// Channel difference above which a pixel counts for the tint ratio: wrong
/// surface tokens (for example #F5F4F1 against #FCFBF9) differ by less than
/// the main threshold and would otherwise go unseen.
pub const TINT_THRESHOLD: u8 = 4;

/// Composites a possibly transparent image over white.
pub fn flatten(image: &RgbaImage) -> RgbaImage {
    let mut out = image.clone();
    for pixel in out.pixels_mut() {
        let a = u32::from(pixel[3]);
        if a < 255 {
            for c in 0..3 {
                let v = (u32::from(pixel[c]) * a + 255 * (255 - a) + 127) / 255;
                pixel[c] = v as u8;
            }
            pixel[3] = 255;
        }
    }
    out
}

/// Box-averages an image by an integer factor.
pub fn downsample(image: &RgbaImage, factor: u32) -> RgbaImage {
    if factor <= 1 {
        return image.clone();
    }
    let w = image.width() / factor;
    let h = image.height() / factor;
    let n = factor * factor;
    RgbaImage::from_fn(w, h, |x, y| {
        let mut sum = [0u32; 4];
        for dy in 0..factor {
            for dx in 0..factor {
                let p = image.get_pixel(x * factor + dx, y * factor + dy);
                for c in 0..4 {
                    sum[c] += u32::from(p[c]);
                }
            }
        }
        Rgba([
            ((sum[0] + n / 2) / n) as u8,
            ((sum[1] + n / 2) / n) as u8,
            ((sum[2] + n / 2) / n) as u8,
            ((sum[3] + n / 2) / n) as u8,
        ])
    })
}

fn max_channel(a: &Rgba<u8>, b: &Rgba<u8>) -> u8 {
    (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0)
}

/// Compares `capture` with `reference`, both at `scale` device pixels per
/// design pixel and already flattened.
pub fn compare(
    reference: &RgbaImage,
    capture: &RgbaImage,
    scale: u32,
    options: Options,
) -> Comparison {
    let scale = scale.max(1);
    let width = reference.width().min(capture.width());
    let height = reference.height().min(capture.height());
    let crop = if reference.dimensions() == capture.dimensions() {
        Crop::Full
    } else if reference.width() == capture.width() && reference.height() > capture.height() {
        Crop::Top
    } else {
        Crop::Mismatch
    };
    let radius = (options.neighbourhood * scale) as i64;
    let threshold = options.threshold;
    let mut mask = vec![0u8; (width as usize) * (height as usize)];
    let mut differing = 0u64;
    let mut strict = 0u64;
    let mut tint = 0u64;
    let has_match = |from: &RgbaImage, at: &Rgba<u8>, x: u32, y: u32| -> bool {
        for dy in -radius..=radius {
            let yy = i64::from(y) + dy;
            if yy < 0 || yy >= i64::from(height) {
                continue;
            }
            for dx in -radius..=radius {
                let xx = i64::from(x) + dx;
                if xx < 0 || xx >= i64::from(width) {
                    continue;
                }
                if max_channel(at, from.get_pixel(xx as u32, yy as u32)) <= threshold {
                    return true;
                }
            }
        }
        false
    };
    for y in 0..height {
        for x in 0..width {
            let r = reference.get_pixel(x, y);
            let c = capture.get_pixel(x, y);
            let d = max_channel(r, c);
            if d > TINT_THRESHOLD {
                tint += 1;
            }
            if d <= threshold {
                continue;
            }
            strict += 1;
            if has_match(capture, r, x, y) && has_match(reference, c, x, y) {
                continue;
            }
            differing += 1;
            mask[(y * width + x) as usize] = d;
        }
    }
    let (regions, noise_regions) = regions(&mask, width, height, scale, options);
    let ssim = ssim(reference, capture, width, height, scale);
    Comparison {
        scale,
        width,
        height,
        crop,
        differing,
        strict_differing: strict,
        tint_differing: tint,
        ssim,
        regions,
        noise_regions,
        mask,
    }
}

/// Groups the differing pixels of `mask` into regions (design pixels).
fn regions(
    mask: &[u8],
    width: u32,
    height: u32,
    scale: u32,
    options: Options,
) -> (Vec<Region>, usize) {
    let cell = (options.cell * scale).max(1);
    let gw = width.div_ceil(cell) as usize;
    let gh = height.div_ceil(cell) as usize;
    // Per cell: differing count and tight device-pixel box.
    let mut count = vec![0u64; gw * gh];
    let mut boxes = vec![(u32::MAX, u32::MAX, 0u32, 0u32); gw * gh];
    for y in 0..height {
        for x in 0..width {
            if mask[(y * width + x) as usize] == 0 {
                continue;
            }
            let i = (y / cell) as usize * gw + (x / cell) as usize;
            count[i] += 1;
            let b = &mut boxes[i];
            b.0 = b.0.min(x);
            b.1 = b.1.min(y);
            b.2 = b.2.max(x + 1);
            b.3 = b.3.max(y + 1);
        }
    }
    // 8-connected components of marked cells, in device pixels.
    let mut seen = vec![false; gw * gh];
    let mut found: Vec<(Rect, u64)> = Vec::new();
    for start in 0..gw * gh {
        if count[start] == 0 || seen[start] {
            continue;
        }
        seen[start] = true;
        let mut stack = vec![start];
        let mut bbox = (u32::MAX, u32::MAX, 0u32, 0u32);
        let mut pixels = 0;
        while let Some(i) = stack.pop() {
            pixels += count[i];
            let b = boxes[i];
            bbox = (bbox.0.min(b.0), bbox.1.min(b.1), bbox.2.max(b.2), bbox.3.max(b.3));
            let (cx, cy) = ((i % gw) as i64, (i / gw) as i64);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (cx + dx, cy + dy);
                    if nx < 0 || ny < 0 || nx >= gw as i64 || ny >= gh as i64 {
                        continue;
                    }
                    let j = ny as usize * gw + nx as usize;
                    if count[j] > 0 && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        found.push((Rect { x: bbox.0, y: bbox.1, w: bbox.2 - bbox.0, h: bbox.3 - bbox.1 }, pixels));
    }
    // Merge regions closer than the gap until stable.
    let gap = options.merge_gap * scale;
    loop {
        let mut merged = false;
        let mut i = 0;
        while i < found.len() {
            let mut j = i + 1;
            while j < found.len() {
                if found[i].0.grow(gap).intersects(found[j].0) {
                    let (rect, pixels) = found.swap_remove(j);
                    found[i].0 = found[i].0.union(rect);
                    found[i].1 += pixels;
                    merged = true;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        if !merged {
            break;
        }
    }
    let mut out = Vec::new();
    let mut noise = 0;
    for (rect, pixels) in found {
        let x0 = rect.x / scale;
        let y0 = rect.y / scale;
        let x1 = (rect.x + rect.w).div_ceil(scale);
        let y1 = (rect.y + rect.h).div_ceil(scale);
        let design = Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
        if design.w > options.min_extent || design.h > options.min_extent {
            out.push(Region { rect: design, pixels });
        } else {
            noise += 1;
        }
    }
    out.sort_by(|a, b| b.pixels.cmp(&a.pixels).then(a.rect.y.cmp(&b.rect.y)));
    (out, noise)
}

fn luma(p: &Rgba<u8>) -> f64 {
    0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2])
}

/// Mean SSIM over 8x8 windows of the luminance at one pixel per design pixel.
fn ssim(reference: &RgbaImage, capture: &RgbaImage, width: u32, height: u32, scale: u32) -> f64 {
    let w = (width / scale) as usize;
    let h = (height / scale) as usize;
    if w == 0 || h == 0 {
        return 1.0;
    }
    let sample = |image: &RgbaImage| -> Vec<f64> {
        let mut out = vec![0.0; w * h];
        let n = f64::from(scale * scale);
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0.0;
                for dy in 0..scale {
                    for dx in 0..scale {
                        sum += luma(image.get_pixel(x as u32 * scale + dx, y as u32 * scale + dy));
                    }
                }
                out[y * w + x] = sum / n;
            }
        }
        out
    };
    let a = sample(reference);
    let b = sample(capture);
    let c1 = (0.01f64 * 255.0).powi(2);
    let c2 = (0.03f64 * 255.0).powi(2);
    let mut total = 0.0;
    let mut windows = 0u64;
    for wy in (0..h).step_by(8) {
        for wx in (0..w).step_by(8) {
            let (mut sa, mut sb, mut saa, mut sbb, mut sab, mut n) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
            for y in wy..(wy + 8).min(h) {
                for x in wx..(wx + 8).min(w) {
                    let (va, vb) = (a[y * w + x], b[y * w + x]);
                    sa += va;
                    sb += vb;
                    saa += va * va;
                    sbb += vb * vb;
                    sab += va * vb;
                    n += 1.0;
                }
            }
            let (ma, mb) = (sa / n, sb / n);
            let va = saa / n - ma * ma;
            let vb = sbb / n - mb * mb;
            let cov = sab / n - ma * mb;
            total += ((2.0 * ma * mb + c1) * (2.0 * cov + c2))
                / ((ma * ma + mb * mb + c1) * (va + vb + c2));
            windows += 1;
        }
    }
    total / windows as f64
}

/// Outline colours of the heat map.
pub const KNOWN: Rgba<u8> = Rgba([255, 160, 0, 255]);
pub const NEW: Rgba<u8> = Rgba([220, 0, 180, 255]);
pub const RESOLVED: Rgba<u8> = Rgba([0, 140, 255, 255]);

/// The difference heat map at the compared scale: the capture faded to a
/// light grey, differing pixels from yellow (small) to red (large).
pub fn heat_map(capture: &RgbaImage, comparison: &Comparison) -> RgbaImage {
    let (w, h) = (comparison.width, comparison.height);
    RgbaImage::from_fn(w, h, |x, y| {
        let d = comparison.mask[(y * w + x) as usize];
        if d > 0 {
            let t = f64::from(d) / 255.0;
            Rgba([255, (210.0 * (1.0 - t)) as u8, 0, 255])
        } else {
            let l = luma(capture.get_pixel(x, y));
            let v = (255.0 - (255.0 - l) * 0.25) as u8;
            Rgba([v, v, v, 255])
        }
    })
}

/// Draws a rectangle outline (design pixels) on an image at `scale`.
pub fn outline(image: &mut RgbaImage, rect: Rect, scale: u32, colour: Rgba<u8>) {
    let thickness = scale.max(1) * 2;
    let (w, h) = image.dimensions();
    let x0 = (rect.x * scale).min(w);
    let y0 = (rect.y * scale).min(h);
    let x1 = ((rect.x + rect.w) * scale).min(w);
    let y1 = ((rect.y + rect.h) * scale).min(h);
    for y in y0..y1 {
        for x in x0..x1 {
            let edge = x < x0 + thickness
                || y < y0 + thickness
                || x + thickness >= x1
                || y + thickness >= y1;
            if edge {
                image.put_pixel(x, y, colour);
            }
        }
    }
}

/// Reference | capture | heat map side by side at one pixel per design
/// pixel, cropped to the compared area, with a 16 px grey gutter.
pub fn three_up(
    reference: &RgbaImage,
    capture: &RgbaImage,
    heat: &RgbaImage,
    scale: u32,
) -> RgbaImage {
    let gutter = 16;
    let crop = |image: &RgbaImage| {
        let w = image.width().min(heat.width());
        let h = image.height().min(heat.height());
        let view = image::imageops::crop_imm(image, 0, 0, w, h).to_image();
        downsample(&view, scale)
    };
    let panels = [crop(reference), crop(capture), downsample(heat, scale)];
    let pw = panels.iter().map(RgbaImage::width).max().unwrap_or(0);
    let ph = panels.iter().map(RgbaImage::height).max().unwrap_or(0);
    let mut out = RgbaImage::from_pixel(pw * 3 + gutter * 2, ph, Rgba([96, 96, 96, 255]));
    for (i, panel) in panels.iter().enumerate() {
        image::imageops::replace(&mut out, panel, i64::from(i as u32 * (pw + gutter)), 0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic screen: light background, a dark block and a text-like
    /// row of thin bars, at `scale`.
    fn screen(scale: u32, block_x: u32) -> RgbaImage {
        let (w, h) = (200 * scale, 120 * scale);
        RgbaImage::from_fn(w, h, |x, y| {
            let (dx, dy) = (x / scale, y / scale);
            if (block_x..block_x + 40).contains(&dx) && (20..60).contains(&dy) {
                Rgba([34, 34, 36, 255])
            } else if (100..180).contains(&dy) && dy < 104 && dx % 3 == 0 {
                Rgba([22, 22, 22, 255])
            } else {
                Rgba([252, 251, 249, 255])
            }
        })
    }

    #[test]
    fn identical_images_have_zero_difference() {
        for scale in [1, 2] {
            let a = screen(scale, 30);
            let c = compare(&a, &a.clone(), scale, Options::default());
            assert_eq!(c.differing, 0);
            assert_eq!(c.strict_differing, 0);
            assert_eq!(c.tint_differing, 0);
            assert!(c.regions.is_empty());
            assert!((c.ssim - 1.0).abs() < 1e-9);
            assert_eq!(c.crop, Crop::Full);
        }
    }

    #[test]
    fn a_one_pixel_edge_shift_is_tolerated() {
        let c = compare(&screen(1, 30), &screen(1, 31), 1, Options::default());
        assert_eq!(c.differing, 0, "a one design pixel shift is anti-aliasing tolerance");
        assert!(c.strict_differing > 0);
    }

    #[test]
    fn a_shifted_block_is_detected_with_its_region() {
        for scale in [1, 2] {
            let c = compare(&screen(scale, 30), &screen(scale, 36), scale, Options::default());
            assert!(c.differing > 0);
            // The strip the block left and the strip it moved into.
            assert_eq!(c.regions.len(), 2, "{:?}", c.regions);
            let mut rects: Vec<Rect> = c.regions.iter().map(|r| r.rect).collect();
            rects.sort_by_key(|r| r.x);
            let (left, right) = (rects[0], rects[1]);
            assert!(left.x >= 29 && left.x <= 31 && left.w >= 4 && left.w <= 7, "{left}");
            assert!(right.x + right.w >= 75 && right.x + right.w <= 77, "{right}");
            for r in rects {
                assert!(r.y >= 19 && r.y <= 21 && r.h >= 38 && r.h <= 42, "{r}");
            }
            assert!(c.ssim < 0.99);
        }
    }

    #[test]
    fn a_colour_change_is_detected() {
        let a = screen(1, 30);
        let mut b = a.clone();
        for y in 20..60 {
            for x in 30..70 {
                b.put_pixel(x, y, Rgba([90, 90, 90, 255]));
            }
        }
        let c = compare(&a, &b, 1, Options::default());
        assert_eq!(c.differing, 40 * 40);
        assert_eq!(c.regions.len(), 1);
        assert_eq!(c.regions[0].rect, Rect { x: 30, y: 20, w: 40, h: 40 });
    }

    #[test]
    fn a_taller_reference_is_compared_over_the_capture_height() {
        let tall = RgbaImage::from_pixel(50, 80, Rgba([255, 255, 255, 255]));
        let short = RgbaImage::from_pixel(50, 60, Rgba([255, 255, 255, 255]));
        let c = compare(&tall, &short, 1, Options::default());
        assert_eq!(c.crop, Crop::Top);
        assert_eq!((c.width, c.height), (50, 60));
        let narrow = RgbaImage::from_pixel(40, 60, Rgba([255, 255, 255, 255]));
        assert_eq!(compare(&tall, &narrow, 1, Options::default()).crop, Crop::Mismatch);
    }

    #[test]
    fn a_close_surface_colour_shows_in_the_tint_ratio_only() {
        let a = RgbaImage::from_pixel(20, 10, Rgba([252, 251, 249, 255]));
        let b = RgbaImage::from_pixel(20, 10, Rgba([245, 244, 241, 255]));
        let c = compare(&a, &b, 1, Options::default());
        assert_eq!(c.differing, 0);
        assert!((c.tint_ratio() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn single_noise_pixels_are_not_deviations() {
        let a = screen(1, 30);
        let mut b = a.clone();
        b.put_pixel(150, 10, Rgba([0, 0, 0, 255]));
        let c = compare(&a, &b, 1, Options::default());
        assert_eq!(c.differing, 1);
        assert!(c.regions.is_empty());
        assert_eq!(c.noise_regions, 1);
    }

    #[test]
    fn three_up_is_three_panels_at_design_scale() {
        let a = screen(2, 30);
        let b = screen(2, 36);
        let c = compare(&a, &b, 2, Options::default());
        let heat = heat_map(&b, &c);
        let image = three_up(&a, &b, &heat, 2);
        assert_eq!(image.dimensions(), (200 * 3 + 32, 120));
    }

    #[test]
    fn downsample_and_flatten() {
        let image =
            RgbaImage::from_fn(4, 2, |x, _| Rgba([if x % 2 == 0 { 0 } else { 200 }, 0, 0, 255]));
        let half = downsample(&image, 2);
        assert_eq!(half.dimensions(), (2, 1));
        assert_eq!(half.get_pixel(0, 0)[0], 100);
        let clear = RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 0]));
        assert_eq!(flatten(&clear).get_pixel(0, 0), &Rgba([255, 255, 255, 255]));
    }
}
