// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The capture matrix and capture file names.
//!
//! A capture is named `<board-key>__<locale>__<theme>__<width>x<height>.png`,
//! where the size is the window size in logical (design) pixels; the image
//! itself is at the window's scale factor. The capture program writes a
//! `captures.json` beside the images ([`CaptureLog`]) that records, per
//! board, what was rendered and why a board could not be captured.

use serde::{Deserialize, Serialize};

/// Window height used for boards drawn taller than a window (1040 and 1120
/// canvases): the capture is 1440x900 and only the top 900 px of the
/// reference are compared (lead decision for the first baseline).
pub const TALL_BOARD_WINDOW_HEIGHT: u32 = 900;

/// Themes of the matrix.
pub const THEMES: [&str; 2] = ["light", "dark"];

/// Locales of the matrix. `en-XA` is a long-string pseudo-locale (about
/// +40 % with accented Latin) and `ar-XB` a right-to-left pseudo-locale; a
/// locale the application does not ship yet is skipped and logged.
pub const LOCALES: [&str; 4] = ["zh-CN", "en", "en-XA", "ar-XB"];

/// A window size of the matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowSize {
    /// The board's own frame width with its height capped at
    /// [`TALL_BOARD_WINDOW_HEIGHT`] when the board is a taller canvas.
    Board,
    /// A fixed logical size.
    Fixed(u32, u32),
}

/// Sizes of the matrix: the board size, the default window, the 1024 and
/// 800 wide compact layouts and the minimum window.
pub const SIZES: [WindowSize; 5] = [
    WindowSize::Board,
    WindowSize::Fixed(1280, 820),
    WindowSize::Fixed(1024, 768),
    WindowSize::Fixed(800, 900),
    WindowSize::Fixed(640, 480),
];

impl WindowSize {
    /// The logical window size for a board frame.
    pub fn resolve(self, frame_width: u32, frame_height: u32) -> (u32, u32) {
        match self {
            Self::Board => (frame_width, frame_height.min(TALL_BOARD_WINDOW_HEIGHT)),
            Self::Fixed(w, h) => (w, h),
        }
    }

    /// Parses `board` or `<w>x<h>`.
    pub fn parse(text: &str) -> Option<Self> {
        if text == "board" {
            return Some(Self::Board);
        }
        let (w, h) = text.split_once('x')?;
        Some(Self::Fixed(w.parse().ok()?, h.parse().ok()?))
    }
}

/// The identity of one capture.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CaptureName {
    pub key: String,
    pub locale: String,
    pub theme: String,
    pub width: u32,
    pub height: u32,
}

impl CaptureName {
    /// The file name, with the `.png` extension.
    pub fn file_name(&self) -> String {
        format!("{}.png", self.stem())
    }

    /// The file name without extension.
    pub fn stem(&self) -> String {
        format!("{}__{}__{}__{}x{}", self.key, self.locale, self.theme, self.width, self.height)
    }

    /// Parses a file name or stem.
    pub fn parse(name: &str) -> Option<Self> {
        let stem = name.strip_suffix(".png").unwrap_or(name);
        let mut parts = stem.split("__");
        let key = parts.next()?.to_owned();
        let locale = parts.next()?.to_owned();
        let theme = parts.next()?.to_owned();
        let size = parts.next()?;
        if parts.next().is_some() || key.is_empty() {
            return None;
        }
        let (w, h) = size.split_once('x')?;
        Some(Self { key, locale, theme, width: w.parse().ok()?, height: h.parse().ok()? })
    }
}

/// What the capture program did for one board and matrix cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureRecord {
    pub name: CaptureName,
    /// Scale factor of the image (device pixels per design pixel).
    pub scale: u32,
    /// `captured` or `skipped`.
    pub outcome: String,
    /// The screen recipe and fixture used, or why the board was skipped.
    pub note: String,
    /// Landmarks (element ids) expected on screen and missing.
    #[serde(default)]
    pub missing_landmarks: Vec<String>,
}

/// The log the capture program writes as `captures.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CaptureLog {
    /// Fonts, clock and locale facts of the run.
    pub environment: Vec<String>,
    pub records: Vec<CaptureRecord>,
}

impl CaptureLog {
    /// The record of `name`, if any.
    pub fn find(&self, name: &CaptureName) -> Option<&CaptureRecord> {
        self.records.iter().find(|r| &r.name == name)
    }

    /// A record for `key` regardless of matrix cell (the first one).
    pub fn for_key(&self, key: &str) -> Option<&CaptureRecord> {
        self.records.iter().find(|r| r.name.key == key)
    }
}

/// The capture a board's reference is compared with: the board's language
/// and theme at the board size.
pub fn board_capture(key: &str, language: &str, theme: &str, frame: (u32, u32)) -> CaptureName {
    let (width, height) = WindowSize::Board.resolve(frame.0, frame.1);
    // `light+dark` boards (H22) are compared in light.
    let theme = if theme == "dark" { "dark" } else { "light" };
    CaptureName {
        key: key.to_owned(),
        locale: language.to_owned(),
        theme: theme.to_owned(),
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        let name = board_capture("A01", "zh-CN", "light", (1440, 1040));
        assert_eq!(name.file_name(), "A01__zh-CN__light__1440x900.png");
        assert_eq!(CaptureName::parse(&name.file_name()), Some(name));
        assert_eq!(CaptureName::parse("A01__zh-CN__light"), None);
        assert_eq!(CaptureName::parse("A01__zh-CN__light__1x2__x"), None);
    }

    #[test]
    fn sizes_resolve_and_parse() {
        assert_eq!(WindowSize::Board.resolve(800, 900), (800, 900));
        assert_eq!(WindowSize::Board.resolve(1440, 1120), (1440, 900));
        assert_eq!(WindowSize::parse("1024x768"), Some(WindowSize::Fixed(1024, 768)));
        assert_eq!(WindowSize::parse("board"), Some(WindowSize::Board));
        assert_eq!(WindowSize::parse("wide"), None);
    }
}
