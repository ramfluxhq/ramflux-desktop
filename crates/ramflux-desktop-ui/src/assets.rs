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
]);

/// GPUI Kit's default icons and [`ExtraIcons`].
#[derive(Clone, Copy, Debug, Default)]
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn call_icons_and_default_icons_load() {
        for icon in [
            gpui_kit::assets::IconName::PhoneIncoming,
            gpui_kit::assets::IconName::Search,
            // The group rename line.
            gpui_kit::assets::IconName::Users,
        ] {
            let path = icon.path();
            assert!(AppAssets.load(&path).ok().flatten().is_some(), "{path}");
        }
    }
}
