// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Account: the active account, lock (returns to entry), backup
//! export (a masked backup passphrase handed to the root as a secret with the
//! chosen path), and passphrase rotation (two masked inputs handed to the
//! root as secrets). Masked inputs are cleared the moment they are taken.

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::{Sizable as _, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, SharedString, TestSupportExt as _, Window, div};
use ramflux_desktop_core::{Account, IntentKind, LockState, SecretInput, UiEventKind};

use super::{SectionContext, SettingsSection, design, note};
use crate::file_dialog::{self, StartDir};
use crate::i18n::{self, tr, tr_args};

/// The masked inputs of the account section, owned by the settings page.
pub struct AccountInputs {
    /// Current passphrase (rotation).
    pub current: Entity<InputState>,
    /// New passphrase (rotation).
    pub next: Entity<InputState>,
    /// Passphrase the exported backup is encrypted with.
    pub backup: Entity<InputState>,
}

pub(super) fn active_account(context: &SectionContext, cx: &App) -> Option<Account> {
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
    // Design G01: the current account (its row with a "Current" chip, the
    // default node and device, Lock), the key backup export and the
    // passphrase change, each a section of the 760 px column.
    let current = {
        let account = account.clone();
        let sink = sink.clone();
        SettingItem::render(move |_, _, cx: &mut App| {
            let section =
                design::section("account-current", tr("settings.account.current_title"), cx);
            let Some(a) = account.clone() else {
                return section.child(note("settings.account.none", cx)).into_any_element();
            };
            let state = tr(match a.lock_state {
                LockState::Unlocked => "settings.account.unlocked",
                _ => "settings.account.locked",
            });
            let node = a
                .node_id
                .as_ref()
                .map_or_else(|| tr("settings.account.no_node"), |id| id.to_string().into());
            let device = a
                .device_id
                .clone()
                .map_or_else(|| tr("settings.account.no_device"), SharedString::from);
            let lock_id = a.id.clone();
            let lock = sink.clone();
            section
                .child(design::row(
                    ramflux_desktop_navigation::row::list_text(&a.label),
                    i18n::join(&i18n::account_kind(a.kind), &state),
                    Some(design::chip(tr("settings.account.current_chip"), cx)),
                    cx,
                ))
                .child(design::row(
                    tr("settings.account.default_node"),
                    tr_args(
                        "settings.account.node_device",
                        &[("node", node.as_ref()), ("device", device.as_ref())],
                    ),
                    None,
                    cx,
                ))
                .child(
                    h_flex().child(
                        crate::style::secondary_button("settings-lock", cx)
                            .label(tr("settings.account.lock"))
                            .on_click(move |_, window, cx| {
                                lock.emit(
                                    UiEventKind::LockPressed { account_id: lock_id.clone() },
                                    window,
                                    cx,
                                )
                            }),
                    ),
                )
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
            design::section("account-backup", tr("settings.account.backup_title"), cx)
                .child(design::help(tr("settings.account.backup_help"), cx))
                .child(super::presentation::field(
                    tr("settings.account.backup_passphrase"),
                    Input::new(&input)
                        .id("backup-passphrase")
                        .aria_label(tr("settings.account.backup_passphrase"))
                        .mask_toggle()
                        .large(),
                    cx,
                ))
                .child(
                    h_flex().child(crate::gate::gated(
                        crate::style::secondary_button("settings-export-backup", cx)
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
        SettingItem::render(move |_, _, cx: &mut App| {
            let (current, next, root, account) =
                (current.clone(), next.clone(), root.clone(), account.clone());
            design::section("account-rotate", tr("settings.account.rotate_title"), cx)
                .child(super::presentation::field(
                    tr("settings.account.current"),
                    Input::new(&current)
                        .id("rotate-current")
                        .aria_label(tr("settings.account.current"))
                        .mask_toggle()
                        .large(),
                    cx,
                ))
                .child(super::presentation::field(
                    tr("settings.account.next"),
                    Input::new(&next)
                        .id("rotate-next")
                        .aria_label(tr("settings.account.next"))
                        .mask_toggle()
                        .large(),
                    cx,
                ))
                .child(
                    h_flex().child(
                        crate::style::primary_button("settings-rotate", cx)
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
    // The open-account switch (G25) has no backend contract yet: one
    // section with the typed reason, its heading said once.
    let privacy = SettingItem::render(|_, _, cx: &mut App| {
        design::section("account-privacy", tr("settings_ui.privacy"), cx)
            .child(
                div()
                    .id("account-privacy-unavailable")
                    .test_support()
                    .role(gpui_kit::Role::Status)
                    .p_3()
                    .rounded(gpui_kit::rems(
                        ramflux_desktop_navigation::palette(cx).radius_md / 15.0,
                    ))
                    .bg(ramflux_desktop_navigation::palette(cx).surface)
                    .child(design::help(
                        super::presentation::contract_reason(
                            ramflux_desktop_sdk::business::ContractGap::AccountPrivacy,
                        ),
                        cx,
                    )),
            )
            .into_any_element()
    });
    SettingPage::new(tr("settings.section.account"))
        .group(SettingGroup::new().item(current).item(backup).item(rotation).item(privacy))
}
