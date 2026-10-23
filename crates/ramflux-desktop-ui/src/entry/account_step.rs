// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The account step: unlock an existing account, or create an account, join
//! an identity as a new device, or restore a backup on a confirmed node.
//!
//! Each form hands its secret to the intent root as a `SecretInput` and
//! clears the masked input at once.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, Entity, IntoElement, SharedString, Window, div};
use ramflux_desktop_core::{
    Account, AccountKind, IntentKind, LockState, NodeId, SecretInput, Store,
};

use super::{AccountPath, EntryPage};
use crate::file_dialog;
use crate::i18n::{self, tr, tr_args};
use crate::ids::domain_id;

/// Takes the value of a masked input as a one-shot secret and clears it.
fn take_secret(input: &Entity<InputState>, window: &mut Window, cx: &mut App) -> SecretInput {
    let secret = SecretInput::new(input.read(cx).value().to_string());
    input.update(cx, |input, cx| input.set_value("", window, cx));
    secret
}

fn text(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().trim().to_owned()
}

impl EntryPage {
    fn locked_account(store: &Store) -> Option<&Account> {
        store.accounts().find(|a| a.lock_state == LockState::Locked)
    }

    /// The node new accounts bind to: the chosen one, else the default,
    /// else the first confirmed and enabled node.
    fn target_node(&self, store: &Store) -> Option<NodeId> {
        let usable = |id: &NodeId| store.node(id).is_some_and(|n| n.confirmed && n.enabled);
        self.node
            .clone()
            .filter(usable)
            .or_else(|| {
                store
                    .nodes()
                    .find(|n| n.is_default && n.confirmed && n.enabled)
                    .map(|n| n.id.clone())
            })
            .or_else(|| store.nodes().find(|n| n.confirmed && n.enabled).map(|n| n.id.clone()))
    }

    fn submit_secret(&mut self, kind: IntentKind, window: &mut Window, cx: &mut Context<Self>) {
        self.root.update(cx, |root, cx| root.submit_secret(kind, window, cx));
        cx.notify();
    }

    /// Sends the typed passphrase for the locked account and clears the input.
    pub fn unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let account = Self::locked_account(self.store.read(cx).store()).map(|a| a.id.clone());
        let secret = take_secret(&self.inputs.passphrase, window, cx);
        let Some(account_id) = account.filter(|_| !secret.is_empty()) else { return };
        self.submitted = Some(account_id.clone());
        self.submit_secret(IntentKind::Unlock { account_id, secret }, window, cx);
    }

    /// Creates an account on the target node.
    pub fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let node = self.target_node(self.store.read(cx).store());
        let label = text(&self.inputs.label, cx);
        let secret = take_secret(&self.inputs.new_secret, window, cx);
        let Some(node_id) = node else { return };
        self.submit_secret(
            IntentKind::CreateAccount { kind: self.kind, label, node_id, secret },
            window,
            cx,
        );
    }

    /// Joins an existing identity as a new device.
    pub fn join(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let node = self.target_node(self.store.read(cx).store());
        let device_label = text(&self.inputs.device_label, cx);
        let code = take_secret(&self.inputs.code, window, cx);
        let Some(node_id) = node else { return };
        self.submit_secret(IntentKind::ActivateDevice { node_id, device_label, code }, window, cx);
    }

    /// Restores an account from a backup file.
    pub fn restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = text(&self.inputs.backup_path, cx);
        let secret = take_secret(&self.inputs.backup_secret, window, cx);
        self.submit_secret(IntentKind::RestoreBackup { path, secret }, window, cx);
    }

    pub(super) fn render_account_step(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let _ = window;
        let store = self.store.read(cx).store().clone();
        let accounts: Vec<Account> = store.accounts().cloned().collect();
        let body = if accounts.is_empty() {
            self.render_paths(&store, cx).into_any_element()
        } else {
            self.render_unlock(&accounts, cx).into_any_element()
        };
        v_flex().id("entry-account").gap_3().child(body)
    }

    fn render_unlock(&self, accounts: &[Account], cx: &mut Context<Self>) -> impl IntoElement {
        let unlocking = self.submitted.is_some();
        let muted = cx.theme().muted_foreground;
        let rows = accounts.iter().map(|account| {
            let detail = tr_args(
                "entry.account.detail",
                &[
                    ("kind", &i18n::account_kind(account.kind)),
                    ("node", account.node_id.as_ref().map_or("", |n| n.as_str())),
                    ("device", account.device_id.as_deref().unwrap_or("")),
                ],
            );
            h_flex()
                .id(domain_id("account", &account.id))
                .gap_3()
                .px_3()
                .py_2()
                .border_1()
                .border_color(cx.theme().border)
                .rounded(cx.theme().radius)
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_sm().font_semibold().child(account.label.clone()))
                        .child(div().text_xs().text_color(muted).child(detail)),
                )
                .child(div().text_xs().text_color(muted).child(tr(match account.lock_state {
                    LockState::Locked => "entry.account.locked",
                    LockState::Unlocked => "entry.account.unlocked",
                })))
        });
        v_flex()
            .gap_3()
            .children(rows)
            .child(
                Input::new(&self.inputs.passphrase)
                    .id("passphrase")
                    .aria_label(tr("entry.account.passphrase"))
                    .mask_toggle()
                    .disabled(unlocking),
            )
            .child(
                h_flex().child(
                    Button::new("unlock")
                        .primary()
                        .icon(IconName::Check)
                        .label(tr(if unlocking {
                            "entry.account.unlocking"
                        } else {
                            "entry.account.unlock"
                        }))
                        .loading(unlocking)
                        .disabled(unlocking)
                        .on_click(cx.listener(|this, _, window, cx| this.unlock(window, cx))),
                ),
            )
    }

    fn segment<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        options: &[(T, SharedString)],
        selected: T,
        set: fn(&mut Self, T),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .id(id)
            .gap_1()
            .children(options.iter().enumerate().map(|(ix, (value, label))| {
                let value = *value;
                Button::new(domain_id(id, &ix))
                    .small()
                    .outline()
                    .selected(value == selected)
                    .label(label.clone())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        set(this, value);
                        cx.notify();
                    }))
            }))
            .into_any_element()
    }

    fn render_paths(&self, store: &Store, cx: &mut Context<Self>) -> impl IntoElement {
        let target = self.target_node(store);
        let nodes: Vec<(NodeId, SharedString)> = store
            .nodes()
            .filter(|n| n.confirmed && n.enabled)
            .map(|n| (n.id.clone(), SharedString::from(n.id.to_string())))
            .collect();
        let node_buttons: Vec<Button> = nodes
            .into_iter()
            .map(|(id, label)| {
                let chosen = Some(&id) == target.as_ref();
                Button::new(domain_id("entry-node-choice", &id))
                    .small()
                    .outline()
                    .selected(chosen)
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.node = Some(id.clone());
                        cx.notify();
                    }))
            })
            .collect();
        let paths = [
            (AccountPath::Create, tr("entry.path.create")),
            (AccountPath::Join, tr("entry.path.join")),
            (AccountPath::Restore, tr("entry.path.restore")),
        ];
        let form = match self.path {
            AccountPath::Create => self.render_create(cx),
            AccountPath::Join => self.render_join(cx),
            AccountPath::Restore => self.render_restore(cx),
        };
        v_flex()
            .gap_3()
            .child(self.segment("entry-path", &paths, self.path, |this, path| this.path = path, cx))
            .when(self.path != AccountPath::Restore, |col| {
                col.child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr("entry.node.label")),
                        )
                        .child(
                            h_flex().flex_wrap().gap_1().children(node_buttons).child(
                                Button::new("entry-review-nodes")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Plus)
                                    .label(tr("entry.node.review"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.review_nodes = true;
                                        cx.notify();
                                    })),
                            ),
                        ),
                )
            })
            .child(form)
    }

    fn field(
        label: &str,
        input: &Entity<InputState>,
        id: &'static str,
        masked: bool,
        cx: &App,
    ) -> AnyElement {
        let label = tr(label);
        let input = Input::new(input).id(id).aria_label(label.clone());
        v_flex()
            .gap_1()
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(label))
            .child(if masked { input.mask_toggle() } else { input })
            .into_any_element()
    }

    fn render_create(&self, cx: &mut Context<Self>) -> AnyElement {
        let kinds = [
            (AccountKind::HumanInteractive, i18n::account_kind(AccountKind::HumanInteractive)),
            (AccountKind::HeadlessAi, i18n::account_kind(AccountKind::HeadlessAi)),
        ];
        v_flex()
            .gap_3()
            .child(self.segment("entry-kind", &kinds, self.kind, |this, kind| this.kind = kind, cx))
            .child(Self::field(
                "entry.create.label",
                &self.inputs.label,
                "account-label",
                false,
                cx,
            ))
            .child(Self::field(
                "entry.create.passphrase",
                &self.inputs.new_secret,
                "account-secret",
                true,
                cx,
            ))
            .child(
                h_flex().child(
                    Button::new("account-create")
                        .primary()
                        .label(tr("entry.create.submit"))
                        .on_click(cx.listener(|this, _, window, cx| this.create(window, cx))),
                ),
            )
            .into_any_element()
    }

    fn render_join(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_3()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("entry.join.help")),
            )
            .child(Self::field(
                "entry.join.device",
                &self.inputs.device_label,
                "device-label",
                false,
                cx,
            ))
            .child(Self::field("entry.join.code", &self.inputs.code, "activation-code", true, cx))
            .child({
                let reason = crate::gate::reason(
                    self.store.read(cx).store(),
                    ramflux_desktop_core::methods::DEVICE_ACTIVATE,
                );
                h_flex()
                    .gap_2()
                    .child(crate::gate::gated(
                        Button::new("account-join")
                            .primary()
                            .label(tr("entry.join.submit"))
                            .on_click(cx.listener(|this, _, window, cx| this.join(window, cx))),
                        reason.clone(),
                    ))
                    .when_some(reason, |row, reason| {
                        row.child(
                            div()
                                .id("account-join-unavailable")
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(reason)
                                .test_support(),
                        )
                    })
            })
            .into_any_element()
    }

    /// Picks the backup file with the native open dialog and fills the
    /// path input (view-local; nothing is submitted).
    fn browse_backup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.inputs.backup_path.clone();
        file_dialog::open_file(tr("entry.restore.path"), window, cx, move |path, window, cx| {
            input.update(cx, |input, cx| input.set_value(path, window, cx));
        });
    }

    fn render_restore(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .items_end()
                    .child(div().flex_1().min_w_0().child(Self::field(
                        "entry.restore.path",
                        &self.inputs.backup_path,
                        "backup-path",
                        false,
                        cx,
                    )))
                    .child(
                        Button::new("backup-browse")
                            .outline()
                            .label(tr("entry.restore.browse"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.browse_backup(window, cx);
                            })),
                    ),
            )
            .child(Self::field(
                "entry.restore.passphrase",
                &self.inputs.backup_secret,
                "backup-secret",
                true,
                cx,
            ))
            .child(
                h_flex().child(
                    Button::new("account-restore")
                        .primary()
                        .label(tr("entry.restore.submit"))
                        .on_click(cx.listener(|this, _, window, cx| this.restore(window, cx))),
                ),
            )
            .into_any_element()
    }
}
