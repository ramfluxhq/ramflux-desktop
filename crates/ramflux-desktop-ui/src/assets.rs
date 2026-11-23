// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The window's asset source: GPUI Kit's default component icons plus the
//! few catalog icons the application uses beyond them (calls), embedded
//! one by one instead of the whole catalog.

use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

gpui_kit::assets::icon_assets!(pub ExtraIcons, [
    Phone, PhoneCall, PhoneIncoming, PhoneOutgoing, PhoneMissed, PhoneOff, Mic, MicOff,
    SignalHigh, SignalMedium, SignalLow, SignalZero,
    Cpu, ShieldAlert, Signature, UserPen, Users,
    MessagesSquare, Contact, SquareCheck,
    MessageSquare, UserPlus, ListChecks, ShieldCheck, Reply, Flag, Pencil,
    Paperclip, Clock, CheckCheck, ScrollText, FileCode,
]);

/// GPUI Kit's default icons and [`ExtraIcons`].
#[derive(Clone, Copy, Debug, Default)]
pub struct AppAssets;

/// The brand signature (design `RF / Signature`), drawn as a mask in the
/// text colour.
pub const SIGNATURE: &str = "brand/signature.svg";

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == SIGNATURE {
            return Ok(Some(Cow::Borrowed(include_bytes!("../assets/brand/signature.svg"))));
        }
        match ExtraIcons.load(path)? {
            Some(bytes) => Ok(Some(bytes)),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut all = gpui_kit::assets::Assets.list(path)?;
        all.extend(ExtraIcons.list(path)?);
        Ok(all)
    }
}

/// The interface's Latin face (design token `rf-font-latin`).
pub const LATIN_FAMILY: &str = "Hanken Grotesk";

/// The interface's CJK face (design token `rf-font`).
pub const CJK_FAMILY: &str = "Noto Sans SC";

/// The interface font: Hanken Grotesk with Noto Sans SC for the glyphs it
/// lacks (CJK), at `weight`.
pub fn design_font(weight: gpui_kit::FontWeight) -> gpui_kit::Font {
    font(LATIN_FAMILY, CJK_FAMILY, weight)
}

/// The face of a design text style at `weight`, by the script of the
/// interface language.
///
/// The boards set CJK copy in `rf-font` (Noto Sans SC, its own Latin
/// glyphs included) and Latin copy in `rf-font-latin` (Hanken Grotesk). The
/// face also sets the line's metrics: CJK text on a Hanken Grotesk line sits
/// about 0.09 em higher than on the boards (Hanken's ascent is 1.0 em, Noto
/// Sans SC's 1.16 em). So in a Chinese, Japanese or Korean interface a text
/// style takes Noto Sans SC first; in every other language Hanken Grotesk
/// with Noto Sans SC for the CJK glyphs it lacks. (Measured: the window as
/// a whole in Noto Sans SC moved board text that sits on GPUI's default
/// line, so the face goes with the text styles, which set the line.)
pub fn text_font(weight: gpui_kit::FontWeight) -> gpui_kit::Font {
    if cjk_interface(crate::i18n::current_locale().tag) {
        font(CJK_FAMILY, LATIN_FAMILY, weight)
    } else {
        design_font(weight)
    }
}

fn font(family: &str, fallback: &str, weight: gpui_kit::FontWeight) -> gpui_kit::Font {
    gpui_kit::Font {
        family: family.to_owned().into(),
        features: gpui_kit::FontFeatures::default(),
        fallbacks: Some(gpui_kit::FontFallbacks::from_fonts(vec![fallback.to_owned()])),
        weight,
        style: gpui_kit::FontStyle::Normal,
    }
}

/// A design text style in its face ([`text_font`]).
pub trait DesignType: gpui_kit::Styled + Sized {
    /// The face, size, weight and line height of `style`.
    #[must_use]
    fn design_type(self, style: ramflux_desktop_navigation::scale::TextStyle) -> Self {
        use ramflux_desktop_navigation::scale::DesignText as _;
        self.font(text_font(style.weight)).design_text(style)
    }
}

impl<T: gpui_kit::Styled> DesignType for T {}

/// Whether an interface in `tag` is set in the CJK face.
fn cjk_interface(tag: &str) -> bool {
    let language = tag.split(['-', '_']).next().unwrap_or(tag);
    matches!(language, "zh" | "ja" | "ko")
}

/// Registers the bundled design fonts with the text system: Hanken Grotesk
/// and Noto Sans SC (the official SC subset), each as static 400, 500 and
/// 600 faces instanced from the upstream variable fonts, so the design's
/// medium and semibold weights render (both SIL OFL 1.1, see NOTICE). Call
/// it once before the first window opens; the bytes are linked only into
/// binaries that call it.
///
/// # Errors
/// A font file the platform's text system cannot read.
pub fn load_fonts(cx: &gpui_kit::App) -> Result<()> {
    cx.text_system().add_fonts(vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/HankenGrotesk-Regular.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/HankenGrotesk-Medium.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/HankenGrotesk-SemiBold.ttf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/NotoSansSC-Regular.otf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/NotoSansSC-Medium.otf").as_slice()),
        Cow::Borrowed(include_bytes!("../assets/fonts/NotoSansSC-SemiBold.otf").as_slice()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cjk_languages_take_the_cjk_face() {
        for tag in ["zh-CN", "zh-TW", "ja", "ko-KR"] {
            assert!(cjk_interface(tag), "{tag}");
        }
        for tag in ["en", "en-XA", "ar-XB", "de-DE", "ru"] {
            assert!(!cjk_interface(tag), "{tag}");
        }
    }

    #[test]
    fn call_icons_and_default_icons_load() {
        for icon in [
            gpui_kit::assets::IconName::PhoneIncoming,
            gpui_kit::assets::IconName::Search,
            // The group rename line.
            gpui_kit::assets::IconName::Users,
            gpui_kit::assets::IconName::MessagesSquare,
            gpui_kit::assets::IconName::Contact,
            gpui_kit::assets::IconName::SquareCheck,
        ] {
            let path = icon.path();
            assert!(AppAssets.load(&path).ok().flatten().is_some(), "{path}");
        }
    }
}
