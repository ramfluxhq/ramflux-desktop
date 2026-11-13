// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! RF adapter over both registered Kit themes. Token generation remains external.

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeConfig};
use gpui_kit::{App, Global, Hsla, Rgba};
use serde::Deserialize;
use serde_json::Value;
use std::rc::Rc;

/// RF-specific surface colors that have no equivalent Kit semantic role.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct Colors {
    /// Primary navigation surface.
    pub rail: Hsla,
    /// Person/group avatar background.
    pub avatar: Hsla,
    /// Person/group avatar foreground.
    pub on_avatar: Hsla,
    /// Agent avatar background.
    pub agent: Hsla,
    /// Agent avatar foreground.
    pub on_agent: Hsla,
}

struct RfTheme {
    light: Colors,
    dark: Colors,
}
impl Global for RfTheme {}

#[derive(Deserialize)]
struct ColorValue {
    components: [f32; 3],
    alpha: f32,
}

fn color(mode: &Value, name: &str) -> Result<Hsla, String> {
    let value = mode
        .get(name)
        .and_then(|v| v.get("$value"))
        .ok_or_else(|| format!("missing RF color {name}"))?;
    let value: ColorValue =
        serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
    let [r, g, b] = value.components;
    Ok(Rgba { r, g, b, a: value.alpha }.into())
}

fn palette(mode: &Value) -> Result<Colors, String> {
    Ok(Colors {
        rail: color(mode, "rail")?,
        avatar: color(mode, "avatar-person")?,
        on_avatar: color(mode, "on-avatar-person")?,
        agent: color(mode, "agent-av")?,
        on_agent: color(mode, "on-agent-av")?,
    })
}

/// Installs both exported themes and RF roles, preserving the selected mode.
///
/// # Errors
/// Returns a malformed/missing token diagnostic before changing theme state.
pub fn install(cx: &mut App) -> Result<(), String> {
    let light: ThemeConfig =
        serde_json::from_str(include_str!("../../../design/generated/light.json"))
            .map_err(|e| e.to_string())?;
    let dark: ThemeConfig =
        serde_json::from_str(include_str!("../../../design/generated/dark.json"))
            .map_err(|e| e.to_string())?;
    let tokens: Value = serde_json::from_str(include_str!("../../../design/tokens/rf-tokens.json"))
        .map_err(|e| e.to_string())?;
    let light_colors = palette(tokens.get("light").ok_or("missing RF light tokens")?)?;
    let dark_colors = palette(tokens.get("dark").ok_or("missing RF dark tokens")?)?;
    let mode = Theme::global(cx).mode;
    cx.set_global(RfTheme { light: light_colors, dark: dark_colors });
    Theme::update(cx, |theme| {
        theme.light_theme = Rc::new(light);
        theme.dark_theme = Rc::new(dark);
    });
    Theme::change(mode, None, cx);
    Ok(())
}

/// Active RF colors; uninitialized fixture hosts use the current Kit theme.
#[must_use]
pub fn colors(cx: &App) -> Colors {
    match cx.try_global::<RfTheme>() {
        Some(theme) => {
            if cx.theme().is_dark() {
                theme.dark
            } else {
                theme.light
            }
        }
        None => Colors {
            rail: cx.theme().sidebar,
            avatar: cx.theme().muted,
            on_avatar: cx.theme().foreground,
            agent: cx.theme().primary,
            on_agent: cx.theme().primary_foreground,
        },
    }
}

/// Design rail width relative to the RF 15px body scale.
pub const RAIL_REM: f32 = crate::geometry::RAIL / 15.0;
/// Default section width from the desktop shell reference, relative to body scale.
pub const SECTION_REM: f32 = 264.0 / 15.0;
/// Conversation row height relative to body scale.
pub const ROW_REM: f32 = 64.0 / 15.0;
