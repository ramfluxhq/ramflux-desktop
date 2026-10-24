// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Renaming a group conversation in place: the header's title becomes a
//! field (Rename… in the header menu), Enter or Save submits, Cancel puts
//! the title back.
//!
//! The name is checked with the policy's own check (`check_group_title`)
//! and submitted as [`UiEventKind::RenameRequested`] through the context's
//! sink. The field stays until the store shows the new name, or shows the
//! daemon's reason when the rename fails. The name is this account's; the
//! help line says only the user sees it.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, ClickEvent, Entity, Role, SharedString, Window, div};
use ramflux_desktop_core::{ContextId, IntentError, Store, check_group_title, methods};

use crate::i18n::{self, tr, tr_args};

/// Where a rename is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// Typing the new name.
    Editing,
    /// Submitted `title`; `since` is the store's failure count then.
    Saving {
        /// The name submitted.
        title: String,
        /// Failures recorded before the submission.
        since: u64,
    },
    /// Refused before sending, or by the daemon (translated reason).
    Failed(SharedString),
}

/// An open rename of one context.
pub struct Rename {
    /// The context being renamed.
    pub context: ContextId,
    /// The field.
    pub input: Entity<InputState>,
    /// Progress.
    pub phase: Phase,
}

impl Rename {
    /// Checks the typed name; returns it to submit, or records why not.
    pub fn take(&mut self, store: &Store, cx: &App) -> Option<String> {
        if matches!(self.phase, Phase::Saving { .. }) {
            return None;
        }
        let typed = self.input.read(cx).value().to_string();
        match check_group_title(&typed) {
            Ok(title) => {
                let title = title.to_owned();
                self.phase = Phase::Saving { title: title.clone(), since: store.failures_total() };
                Some(title)
            }
            Err(IntentError::InvalidGroup(problem)) => {
                let key = match problem {
                    ramflux_desktop_core::GroupProblem::TitleTooLong => "group.new.error.long_name",
                    ramflux_desktop_core::GroupProblem::TitleControl => "group.new.error.bad_name",
                    _ => "group.new.error.empty_name",
                };
                self.phase = Phase::Failed(tr(key));
                None
            }
            Err(other) => {
                self.phase = Phase::Failed(SharedString::from(other.to_string()));
                None
            }
        }
    }

    /// Follows the store: whether the rename is done (the new name shows).
    /// A failure is recorded and keeps the field open.
    pub fn follow(&mut self, store: &Store) -> bool {
        let Phase::Saving { title, since } = &self.phase else { return false };
        if store.context(&self.context).is_some_and(|c| &c.title == title) {
            return true;
        }
        if let Some(failure) =
            store.failures_since(*since).iter().rev().find(|f| f.method == methods::CONTEXT_RENAME)
        {
            self.phase = Phase::Failed(tr_args(
                "group.rename.failed",
                &[("reason", &i18n::backend_text(&failure.message))],
            ));
        }
        false
    }
}

/// The inline field with Save and Cancel, and the help or status line.
pub fn render(
    rename: &Rename,
    on_save: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_cancel: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let saving = matches!(rename.phase, Phase::Saving { .. });
    let (line, color) = match &rename.phase {
        Phase::Failed(reason) => (reason.clone(), crate::tone::text(cx.theme().danger, cx)),
        _ => (tr("group.rename.help"), cx.theme().muted_foreground),
    };
    v_flex()
        .id("conversation-rename")
        .flex_1()
        .min_w_0()
        .gap_1()
        .child(
            h_flex()
                .gap_2()
                .child(
                    Input::new(&rename.input)
                        .id("conversation-rename-input")
                        .aria_label(tr("group.rename.label"))
                        .small()
                        .flex_1(),
                )
                .child(
                    Button::new("conversation-rename-save")
                        .small()
                        .primary()
                        .label(tr("group.rename.save"))
                        .disabled(saving)
                        .on_click(on_save),
                )
                .child(
                    Button::new("conversation-rename-cancel")
                        .small()
                        .ghost()
                        .label(tr("group.rename.cancel"))
                        .on_click(on_cancel),
                ),
        )
        .child(
            div()
                .id("conversation-rename-status")
                .role(Role::Status)
                .aria_label(line.clone())
                .test_support()
                .text_xs()
                .text_color(color)
                .child(line),
        )
        .into_any_element()
}
