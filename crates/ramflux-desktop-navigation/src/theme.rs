// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The RF theme registry over GPUI Kit's theme. Token generation remains
//! external.
//!
//! Every theme is a token set: one Kit theme file exported by the design
//! compiler to `design/generated/<id>.json`, plus the RF roles of
//! `design/tokens/rf-tokens.json` under the key `<id>` (or, when the token
//! file has no such key, under the theme's mode, `light` or `dark`). The
//! build embeds every exported file, so adding a theme means adding its
//! token file; no code names a theme.
//!
//! [`install`] registers every set. [`select`] makes one active (its mode
//! becomes the Kit theme mode), and [`select_mode`] picks the set that
//! follows an appearance (the set whose id is the mode's name, else the
//! first of that mode). Views read colours through Kit's `cx.theme()`, the
//! RF roles through [`colors`] and the generated palette through
//! [`crate::palette`]; all three follow the active set.

use std::collections::BTreeMap;
use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeConfig, ThemeMode};
use gpui_kit::{App, Global, Hsla, Rgba, SharedString};
use serde::Deserialize;
use serde_json::Value;

use crate::generated::rf_tokens::RFTheme;

include!(concat!(env!("OUT_DIR"), "/themes.rs"));

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

/// One registered theme, as shown in a theme picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeInfo {
    /// Stable id: the token file's name (`light`, `dark`, …).
    pub id: SharedString,
    /// The name the token set gives itself.
    pub name: SharedString,
    /// Light or dark.
    pub mode: ThemeMode,
}

/// One registered token set, resolved.
struct ThemeSet {
    info: ThemeInfo,
    config: Rc<ThemeConfig>,
    colors: Colors,
    palette: RFTheme,
}

/// Every registered set and the active one.
struct Registry {
    sets: Vec<ThemeSet>,
    active: usize,
}

impl Global for Registry {}

impl Registry {
    fn position(&self, id: &str) -> Option<usize> {
        self.sets.iter().position(|set| set.info.id.as_ref() == id)
    }

    /// The set following `mode`: the one named after it, else the first of
    /// that mode.
    fn for_mode(&self, mode: ThemeMode) -> Option<usize> {
        self.position(mode.name())
            .filter(|&i| self.mode_of(i) == Some(mode))
            .or_else(|| self.sets.iter().position(|set| set.info.mode == mode))
    }

    fn mode_of(&self, index: usize) -> Option<ThemeMode> {
        self.sets.get(index).map(|set| set.info.mode)
    }

    /// The set painting now: the active one when its mode is Kit's mode,
    /// else the one following Kit's mode (a caller changed the mode through
    /// Kit directly).
    fn current(&self, cx: &App) -> Option<&ThemeSet> {
        let mode = Theme::global(cx).mode;
        let index = if self.mode_of(self.active) == Some(mode) {
            Some(self.active)
        } else {
            self.for_mode(mode)
        };
        index.and_then(|i| self.sets.get(i))
    }
}

#[derive(Deserialize)]
struct ColorValue {
    components: [f32; 3],
    alpha: f32,
}

/// The token named `name` of a token set, if present.
fn token<'a>(set: &'a Value, name: &str) -> Option<&'a Value> {
    set.get(name).and_then(|v| v.get("$value"))
}

fn token_color(set: &Value, name: &str) -> Result<Option<Hsla>, String> {
    let Some(value) = token(set, name) else { return Ok(None) };
    let value: ColorValue = serde_json::from_value(value.clone())
        .map_err(|error| format!("RF color {name}: {error}"))?;
    let [r, g, b] = value.components;
    Ok(Some(Rgba { r, g, b, a: value.alpha }.into()))
}

fn token_dimension(set: &Value, name: &str) -> Result<Option<f32>, String> {
    let Some(value) = token(set, name) else { return Ok(None) };
    let number = value.get("value").unwrap_or(value).clone();
    serde_json::from_value::<f32>(number)
        .map(Some)
        .map_err(|error| format!("RF dimension {name}: {error}"))
}

fn required_color(set: &Value, name: &str) -> Result<Hsla, String> {
    token_color(set, name)?.ok_or_else(|| format!("missing RF color {name}"))
}

fn roles(set: &Value) -> Result<Colors, String> {
    Ok(Colors {
        rail: required_color(set, "rail")?,
        avatar: required_color(set, "avatar-person")?,
        on_avatar: required_color(set, "on-avatar-person")?,
        agent: required_color(set, "agent-av")?,
        on_agent: required_color(set, "on-agent-av")?,
    })
}

/// Overrides every field of `base` whose token `set` carries. Font families
/// stay the base's (the generated palette holds them as static names).
macro_rules! overlay {
    ($base:ident, $set:ident; colors: $($c:ident = $cn:literal),* ; dimensions: $($d:ident = $dn:literal),* $(,)?) => {{
        $( if let Some(value) = token_color($set, $cn)? { $base.$c = value; } )*
        $( if let Some(value) = token_dimension($set, $dn)? { $base.$d = value; } )*
    }};
}

/// The generated palette of `mode` with every token `set` defines applied.
///
/// # Errors
/// A token present with a malformed value.
pub fn palette_from_tokens(mode: ThemeMode, set: &Value) -> Result<RFTheme, String> {
    let mut palette = if mode.is_dark() { RFTheme::dark() } else { RFTheme::light() };
    overlay!(palette, set;
        colors: accent = "accent", agent_av = "agent-av", avatar_person = "avatar-person",
            badge = "badge", bg = "bg", control = "control", danger = "danger",
            danger_bg = "danger-bg", focus = "focus", incoming = "incoming",
            incoming_meta = "incoming-meta", incoming_quote = "incoming-quote", input = "input",
            input_border = "input-border", line = "line", line_nav = "line-nav",
            line_soft = "line-soft", line_strong = "line-strong", nav = "nav",
            on_accent = "on-accent", on_agent_av = "on-agent-av",
            on_avatar_person = "on-avatar-person", on_badge = "on-badge",
            on_danger = "on-danger", on_primary = "on-primary", on_success = "on-success",
            on_tooltip = "on-tooltip", outgoing = "outgoing", outgoing_meta = "outgoing-meta",
            outgoing_quote = "outgoing-quote", outgoing_text = "outgoing-text",
            overlay = "overlay", overlay_border = "overlay-border", primary = "primary",
            rail = "rail", rail_active = "rail-active", scrim = "scrim",
            secondary = "secondary", selected = "selected", shadow = "shadow",
            success = "success", success_bg = "success-bg", surface = "surface",
            surface_white = "surface-white", text = "text", tooltip = "tooltip",
            warning = "warning", warning_bg = "warning-bg";
        dimensions: bubble_max = "bubble-max", column_max = "column-max",
            composer_min = "composer-min", gap = "gap", menu_item_height = "menu-item-height",
            mobile_bubble_max = "mobile-bubble-max", radius_bubble = "radius-bubble",
            radius_bubble_joined = "radius-bubble-joined", radius_button = "radius-button",
            radius_card = "radius-card", radius_chip = "radius-chip",
            radius_dialog = "radius-dialog", radius_full = "radius-full",
            radius_input = "radius-input", radius_lg = "radius-lg", radius_md = "radius-md",
            radius_menu = "radius-menu", radius_sheet = "radius-sheet", radius_sm = "radius-sm",
            radius_xl = "radius-xl", radius_xs = "radius-xs",
            row_conversation = "row-conversation", size_body = "size-body",
            size_button = "size-button", size_caption = "size-caption",
            size_meta = "size-meta", size_metric = "size-metric",
            size_mobile_body = "size-mobile-body", size_mobile_caption = "size-mobile-caption",
            size_page = "size-page", size_secondary = "size-secondary",
            size_subheading = "size-subheading", size_title = "size-title",
    );
    Ok(palette)
}

/// Resolves one exported token file against the RF token sets.
fn resolve(id: &str, json: &str, tokens: &Value) -> Result<ThemeSet, String> {
    let config: ThemeConfig =
        serde_json::from_str(json).map_err(|error| format!("theme {id}: {error}"))?;
    let mode = config.mode;
    let set = tokens
        .get(id)
        .or_else(|| tokens.get(mode.name()))
        .ok_or_else(|| format!("theme {id}: no RF token set {id} or {}", mode.name()))?;
    Ok(ThemeSet {
        info: ThemeInfo { id: SharedString::from(id.to_owned()), name: config.name.clone(), mode },
        colors: roles(set)?,
        palette: palette_from_tokens(mode, set)?,
        config: Rc::new(config),
    })
}

/// Every embedded token set, resolved, in id order.
fn sets() -> Result<Vec<ThemeSet>, String> {
    let tokens: Value = serde_json::from_str(include_str!("../../../design/tokens/rf-tokens.json"))
        .map_err(|e| e.to_string())?;
    THEME_FILES.iter().map(|(id, json)| resolve(id, json, &tokens)).collect()
}

/// Registers every exported theme and RF roles, keeping the current mode:
/// the active set is the one following it.
///
/// # Errors
/// Returns a malformed/missing token diagnostic before changing theme state.
pub fn install(cx: &mut App) -> Result<(), String> {
    let sets = sets()?;
    let mode = Theme::global(cx).mode;
    let mut registry = Registry { sets, active: 0 };
    registry.active = registry.for_mode(mode).unwrap_or(0);
    cx.set_global(registry);
    // Both Kit slots get a set, so a Kit-level mode change paints a token set.
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        if let Some(config) = config_for(cx, mode) {
            Theme::update(cx, |theme| match mode {
                ThemeMode::Light => theme.light_theme = config,
                ThemeMode::Dark => theme.dark_theme = config,
            });
        }
    }
    let active = cx.global::<Registry>().active;
    activate(active, cx);
    Ok(())
}

fn config_for(cx: &App, mode: ThemeMode) -> Option<Rc<ThemeConfig>> {
    let registry = cx.try_global::<Registry>()?;
    registry.for_mode(mode).and_then(|i| registry.sets.get(i)).map(|set| set.config.clone())
}

/// Makes set `index` active: its file fills Kit's slot for its mode, and
/// that mode becomes current.
fn activate(index: usize, cx: &mut App) {
    let Some(set) = cx.try_global::<Registry>().and_then(|r| r.sets.get(index)) else { return };
    let (config, mode) = (set.config.clone(), set.info.mode);
    cx.global_mut::<Registry>().active = index;
    Theme::update(cx, |theme| match mode {
        ThemeMode::Light => theme.light_theme = config,
        ThemeMode::Dark => theme.dark_theme = config,
    });
    Theme::change(mode, None, cx);
}

/// Every registered theme, in id order (empty before [`install`]).
#[must_use]
pub fn themes(cx: &App) -> Vec<ThemeInfo> {
    cx.try_global::<Registry>()
        .map(|registry| registry.sets.iter().map(|set| set.info.clone()).collect())
        .unwrap_or_default()
}

/// The theme painting now, if the registry is installed.
#[must_use]
pub fn active(cx: &App) -> Option<ThemeInfo> {
    cx.try_global::<Registry>().and_then(|r| r.current(cx)).map(|set| set.info.clone())
}

/// Makes the theme `id` active at once. Returns whether `id` is registered
/// (nothing changes when it is not).
pub fn select(id: &str, cx: &mut App) -> bool {
    let Some(index) = cx.try_global::<Registry>().and_then(|r| r.position(id)) else {
        return false;
    };
    activate(index, cx);
    true
}

/// Makes the theme following `mode` active (for "follow the system").
pub fn select_mode(mode: ThemeMode, cx: &mut App) {
    match cx.try_global::<Registry>().and_then(|r| r.for_mode(mode)) {
        Some(index) => activate(index, cx),
        None => Theme::change(mode, None, cx),
    }
}

/// Active RF colors; uninitialized fixture hosts use the current Kit theme.
#[must_use]
pub fn colors(cx: &App) -> Colors {
    match cx.try_global::<Registry>().and_then(|r| r.current(cx)) {
        Some(set) => set.colors,
        None => Colors {
            rail: cx.theme().sidebar,
            avatar: cx.theme().muted,
            on_avatar: cx.theme().foreground,
            agent: cx.theme().primary,
            on_agent: cx.theme().primary_foreground,
        },
    }
}

/// The active generated palette; uninitialized hosts get the generated
/// palette of Kit's current mode.
#[must_use]
pub fn palette(cx: &App) -> RFTheme {
    match cx.try_global::<Registry>().and_then(|r| r.current(cx)) {
        Some(set) => set.palette,
        None if cx.theme().is_dark() => RFTheme::dark(),
        None => RFTheme::light(),
    }
}

/// Names of every embedded token set (the theme ids), in order.
#[must_use]
pub fn embedded_ids() -> Vec<&'static str> {
    THEME_FILES.iter().map(|(id, _)| *id).collect()
}

/// Checks every embedded token set resolves; the ids with their modes.
///
/// # Errors
/// The first set that does not resolve.
pub fn check_embedded() -> Result<BTreeMap<String, ThemeMode>, String> {
    Ok(sets()?.into_iter().map(|set| (set.info.id.to_string(), set.info.mode)).collect())
}

/// Design rail width relative to the RF 15px body scale.
pub const RAIL_REM: f32 = crate::geometry::RAIL / 15.0;
/// Default section width from the desktop shell reference, relative to body scale.
pub const SECTION_REM: f32 = 264.0 / 15.0;
/// Conversation row height relative to body scale.
pub const ROW_REM: f32 = 64.0 / 15.0;

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> Value {
        serde_json::from_str(include_str!("../../../design/tokens/rf-tokens.json"))
            .expect("test: token file parses")
    }

    fn close(a: Hsla, b: Hsla) -> bool {
        let (a, b) = (Rgba::from(a), Rgba::from(b));
        [(a.r, b.r), (a.g, b.g), (a.b, b.b), (a.a, b.a)].iter().all(|(x, y)| (x - y).abs() < 1e-3)
    }

    #[test]
    fn every_exported_token_set_resolves_and_light_and_dark_ship() {
        let modes = check_embedded().expect("test: every embedded set must resolve");
        assert_eq!(modes.get("light"), Some(&ThemeMode::Light), "the light set ships");
        assert_eq!(modes.get("dark"), Some(&ThemeMode::Dark), "the dark set ships");
        assert_eq!(modes.len(), embedded_ids().len(), "ids are unique");
    }

    #[test]
    fn a_palette_built_from_tokens_matches_the_generated_palette() {
        let tokens = tokens();
        for (mode, generated) in
            [(ThemeMode::Light, RFTheme::light()), (ThemeMode::Dark, RFTheme::dark())]
        {
            let set = tokens.get(mode.name()).expect("test: mode set present");
            let built = palette_from_tokens(mode, set).expect("test: tokens are well formed");
            for (name, a, b) in [
                ("bg", built.bg, generated.bg),
                ("text", built.text, generated.text),
                ("primary", built.primary, generated.primary),
                ("outgoing", built.outgoing, generated.outgoing),
                ("danger", built.danger, generated.danger),
                ("scrim", built.scrim, generated.scrim),
            ] {
                assert!(close(a, b), "{} {name}: tokens and generator disagree", mode.name());
            }
            assert!((built.radius_md - generated.radius_md).abs() < f32::EPSILON, "radius");
            assert!((built.size_body - generated.size_body).abs() < f32::EPSILON, "type size");
        }
    }

    #[test]
    fn a_new_token_set_overrides_only_what_it_defines() {
        let mut set = tokens().get("dark").cloned().expect("test: dark set");
        let bg = serde_json::json!({ "$type": "color", "$value": {
            "colorSpace": "srgb", "components": [1.0, 0.0, 0.0], "alpha": 1.0 } });
        set.as_object_mut().expect("test: object").insert("bg".to_owned(), bg);
        set.as_object_mut().expect("test: object").remove("radius-md");
        let built = palette_from_tokens(ThemeMode::Dark, &set).expect("test: tokens resolve");
        assert!(close(built.bg, Rgba { r: 1.0, g: 0.0, b: 0.0, a: 1.0 }.into()), "bg overridden");
        assert!(
            (built.radius_md - RFTheme::dark().radius_md).abs() < f32::EPSILON,
            "a token the set leaves out keeps the mode's generated value"
        );
    }

    #[test]
    fn a_malformed_token_is_an_error_not_a_default() {
        let set = serde_json::json!({ "bg": { "$value": { "components": "red" } } });
        assert!(palette_from_tokens(ThemeMode::Light, &set).is_err(), "malformed colour");
        let set = serde_json::json!({ "gap": { "$value": { "value": "wide" } } });
        assert!(palette_from_tokens(ThemeMode::Light, &set).is_err(), "malformed dimension");
    }
}
