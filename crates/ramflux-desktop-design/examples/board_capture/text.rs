// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Board and kit copy, read from `fixtures/board_text.toml`: Rust sources
//! carry no CJK text (the guard in `ui/tests/locales.rs`).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use gpui_kit::SharedString;

#[derive(serde::Deserialize)]
struct File {
    text: BTreeMap<String, String>,
}

fn table() -> &'static BTreeMap<String, String> {
    static TABLE: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    TABLE.get_or_init(|| {
        toml::from_str::<File>(include_str!("fixtures/board_text.toml"))
            .map(|f| f.text)
            .unwrap_or_default()
    })
}

/// The copy for `key`; the key itself when it is missing, so a gap shows
/// in the capture instead of failing the run.
pub fn t(key: &str) -> SharedString {
    table().get(key).map_or_else(|| SharedString::from(key.to_owned()), |s| s.clone().into())
}
