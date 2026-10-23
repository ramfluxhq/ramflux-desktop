// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Native file dialogs (GPUI path prompts) for every local path the user
//! chooses: backup export and restore, device sync, diagnostics export and
//! saving a downloaded object, the daemon program and its data folder.
//!
//! A dialog is view-local: it only produces a path. The caller turns the
//! chosen path into a UI event (or fills an input); a cancelled dialog does
//! nothing. Paths are absolute, as the platform returns them.

use std::path::{Path, PathBuf};

use gpui_kit::{App, PathPromptOptions, SharedString, Window};

/// Where a save dialog starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartDir {
    /// The user's documents folder.
    Documents,
    /// The user's downloads folder.
    Downloads,
}

impl StartDir {
    /// The folder, falling back to the home folder and then the current one.
    pub fn path(self) -> PathBuf {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let dir = home.join(match self {
            Self::Documents => "Documents",
            Self::Downloads => "Downloads",
        });
        if dir.is_dir() { dir } else { home }
    }
}

/// Shows the platform "save as" dialog starting in `dir` with `suggested`
/// as the file name, then calls `chosen` with the absolute path. Nothing
/// happens when the user cancels.
pub fn save_as(
    dir: StartDir,
    suggested: &str,
    window: &mut Window,
    cx: &mut App,
    chosen: impl FnOnce(String, &mut Window, &mut App) + 'static,
) {
    let receiver = cx.prompt_for_new_path(&dir.path(), Some(suggested));
    window
        .spawn(cx, async move |cx| {
            if let Ok(Ok(Some(path))) = receiver.await {
                cx.update(|window, cx| chosen(display(&path), window, cx)).ok();
            }
        })
        .detach();
}

/// Shows the platform "open" dialog for one existing file, then calls
/// `chosen` with the absolute path. Nothing happens when the user cancels.
pub fn open_file(
    prompt: SharedString,
    window: &mut Window,
    cx: &mut App,
    chosen: impl FnOnce(String, &mut Window, &mut App) + 'static,
) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some(prompt),
    });
    window
        .spawn(cx, async move |cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(path) = paths.into_iter().next()
            {
                cx.update(|window, cx| chosen(display(&path), window, cx)).ok();
            }
        })
        .detach();
}

/// Shows the platform "open" dialog for one folder, then calls `chosen`
/// with the absolute path. Nothing happens when the user cancels.
pub fn open_dir(
    prompt: SharedString,
    window: &mut Window,
    cx: &mut App,
    chosen: impl FnOnce(String, &mut Window, &mut App) + 'static,
) {
    let receiver = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some(prompt),
    });
    window
        .spawn(cx, async move |cx| {
            if let Ok(Ok(Some(paths))) = receiver.await
                && let Some(path) = paths.into_iter().next()
            {
                cx.update(|window, cx| chosen(display(&path), window, cx)).ok();
            }
        })
        .detach();
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
