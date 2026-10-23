// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Account: the active account, lock (returns to entry), backup
//! export (a masked backup passphrase handed to the root as a secret with the
//! chosen path), and passphrase rotation (two masked inputs handed to the
//! root as secrets). Masked inputs are cleared the moment they are taken.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, SharedString, Window};
use ramflux_desktop_core::{Account, IntentKind, LockState, SecretInput, UiEventKind};

use super::{SectionContext, SettingsSection, note};
use crate::file_dialog::{self, StartDir};
use crate::i18n::{self, tr};

/// The masked inputs of the account section, owned by the settings page.
pub struct AccountInputs {
    /// Current passphrase (rotation).
    pub current: Entity<InputState>,
    /// New passphrase (rotation).
    pub next: Entity<InputState>,
    /// Passphrase the exported backup is encrypted with.
    pub backup: Entity<InputState>,
}

fn active_account(context: &SectionContext, cx: &App) -> Option<Account> {
    let handle = context.store.read(cx);
    let store = handle.store();
    handle
        .active_account()
        .and_then(|id| store.account(id))
        .or_else(|| store.accounts().find(|a| a.lock_state == LockState::Unlocked))
        .cloned()
}

fn take(input: &Entity<InputState>, window: &mut Window, cx: &mut App) -> SecretInput {
    let secret = SecretInput::new(input.read(cx).value().to_string());
    input.update(cx, |input, cx| input.set_value("", window, cx));
    secret
}

/// The account section.
pub fn page(context: &SectionContext, inputs: &AccountInputs, cx: &App) -> SettingPage {
    let account = active_account(context, cx);
    let sink = context.sink(SettingsSection::Account);
    let export_reason = crate::gate::reason(
        context.store.read(cx).store(),
        ramflux_desktop_core::methods::ACCOUNT_BACKUP_EXPORT,
    );
    let backup_typed = !inputs.backup.read(cx).value().is_empty();
    let summary = {
        let account = account.clone();
        SettingItem::render(move |_, _, cx: &mut App| match &account {
            Some(a) => DescriptionList::new()
                .columns(1)
                .item(tr("settings.account.label"), SharedString::from(a.label.clone()), 1)
                .item(tr("settings.account.kind"), i18n::account_kind(a.kind), 1)
                .item(
                    tr("settings.account.node"),
                    SharedString::from(
                        a.node_id.as_ref().map_or_else(String::new, ToString::to_string),
                    ),
                    1,
                )
                .item(
                    tr("settings.account.device"),
                    SharedString::from(a.device_id.clone().unwrap_or_default()),
                    1,
                )
                .into_any_element(),
            None => note("settings.account.none", cx),
        })
    };
    let actions = {
        let account = account.clone();
        SettingItem::render(move |_, _, _: &mut App| {
            let Some(account) = account.clone() else { return h_flex().into_any_element() };
            let lock_id = account.id.clone();
            Button::new("settings-lock")
                .small()
                .outline()
                .label(tr("settings.account.lock"))
                .on_click({
                    let lock = sink.clone();
                    move |_, window, cx| {
                        lock.emit(
                            UiEventKind::LockPressed { account_id: lock_id.clone() },
                            window,
                            cx,
                        )
                    }
                })
                .into_any_element()
        })
    };
    let backup = {
        let input = inputs.backup.clone();
        let root = context.root.clone();
        let account = account.as_ref().map(|a| a.id.clone());
        SettingItem::render(move |_, _, cx: &mut App| {
            let Some(account_id) = account.clone() else { return h_flex().into_any_element() };
            let (input, root) = (input.clone(), root.clone());
            let suggested = format!("ramflux-{account_id}.backup");
            let reason = export_reason
                .clone()
                .or_else(|| (!backup_typed).then(|| tr("settings.account.backup_help")));
            v_flex()
                .gap_2()
                .child(
                    Input::new(&input)
                        .id("backup-passphrase")
                        .aria_label(tr("settings.account.backup_passphrase"))
                        .mask_toggle(),
                )
                .child(note("settings.account.backup_help", cx))
                .child(
                    h_flex().child(crate::gate::gated(
                        Button::new("settings-export-backup")
                            .small()
                            .outline()
                            .label(tr("settings.account.export"))
                            .on_click(move |_, window, cx| {
                                // Taken (and the input cleared) now; the secret
                                // lives only in the pending dialog callback, which
                                // drops (zeroizes) it when the dialog is cancelled.
                                let secret = take(&input, window, cx);
                                if secret.is_empty() {
                                    return;
                                }
                                let (root, account_id) = (root.clone(), account_id.clone());
                                file_dialog::save_as(
                                    StartDir::Documents,
                                    &suggested,
                                    window,
                                    cx,
                                    move |path, window, cx| {
                                        let kind =
                                            IntentKind::ExportBackup { account_id, path, secret };
                                        root.update(cx, |root, cx| {
                                            root.submit_secret(kind, window, cx);
                                        });
                                    },
                                );
                            }),
                        reason,
                    )),
                )
                .into_any_element()
        })
    };
    let rotation = {
        let current = inputs.current.clone();
        let next = inputs.next.clone();
        let root = context.root.clone();
        let account = account.map(|a| a.id);
        SettingItem::render(move |_, _, _: &mut App| {
            let (current, next, root, account) =
                (current.clone(), next.clone(), root.clone(), account.clone());
            v_flex()
                .gap_2()
                .child(
                    Input::new(&current)
                        .id("rotate-current")
                        .aria_label(tr("settings.account.current"))
                        .mask_toggle(),
                )
                .child(
                    Input::new(&next)
                        .id("rotate-next")
                        .aria_label(tr("settings.account.next"))
                        .mask_toggle(),
                )
                .child(
                    h_flex().child(
                        Button::new("settings-rotate")
                            .small()
                            .primary()
                            .label(tr("settings.account.rotate"))
                            .on_click(move |_, window, cx| {
                                let old = take(&current, window, cx);
                                let new = take(&next, window, cx);
                                let Some(account_id) = account.clone() else { return };
                                let kind = IntentKind::RotatePassphrase {
                                    account_id,
                                    current: old,
                                    next: new,
                                };
                                root.update(cx, |root, cx| {
                                    root.submit_secret(kind, window, cx);
                                });
                            }),
                    ),
                )
                .into_any_element()
        })
    };
    SettingPage::new(tr("settings.section.account"))
        .group(SettingGroup::new().item(summary).item(actions))
        .group(SettingGroup::new().title(tr("settings.account.backup_title")).item(backup))
        .group(SettingGroup::new().title(tr("settings.account.rotate_title")).item(rotation))
}
