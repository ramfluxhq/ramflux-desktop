// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Removing an app-locked account from this device, from the lock page.
//!
//! Reachable only from the exhausted page (the account's code attempts are
//! used up). The dialog names the account, says what is lost, and needs the
//! "I understand" box before the red button works. The client then sends the
//! locked account's own id as the confirmation; the user never types it.
//!
//! The dialog closes without acting on Escape, a click outside it, the
//! window losing focus and a change of the lock page's account. It keeps no
//! secret: the confirmation is added to the request body only when the
//! request is written.

use gpui_kit::component::{ActiveTheme as _, StyledExt as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Context, FocusHandle, Window, div};
use ramflux_desktop_core::{AccountId, IntentKind, methods};
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};

use super::EntryPage;
use crate::controls::DesignCheckbox;
use crate::i18n::tr;
use crate::modal::{self, Modal};
use crate::style::DesignButton as _;

/// The open confirmation of a removal.
pub(super) struct RemoveDialog {
    /// The locked account the dialog is about.
    account: AccountId,
    /// Its display name, read when the dialog opened (the account is gone
    /// when the removal is reported).
    name: gpui_kit::SharedString,
    /// The "I understand" box.
    acknowledged: bool,
    /// The store's failure count when the request was sent; `Some` while
    /// the answer is awaited.
    sent: Option<u64>,
    /// The last removal of this dialog failed.
    failed: bool,
}

/// What the lock page tells after the recovery's account code answer or a
/// removal (view-local; the waits themselves always come from the daemon).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LockNotice {
    /// The phrase was right; the account code can be entered after the wait.
    PhraseAccepted,
    /// The phrase was right and the account code was not.
    WrongCode,
}

impl EntryPage {
    /// Opens the confirmation for the lock page's exhausted account.
    pub(super) fn open_remove(
        &mut self,
        account: AccountId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.lock_notice = None;
        self.removed_notice = None;
        let name = self.account_name(&account, cx);
        self.remove_dialog =
            Some(RemoveDialog { account, name, acknowledged: false, sent: None, failed: false });
        let focus = self.remove_focus.clone();
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        cx.notify();
    }

    /// Closes the confirmation without removing anything.
    pub(super) fn close_remove(&mut self, cx: &mut Context<Self>) {
        if self.remove_dialog.take().is_some() {
            cx.notify();
        }
    }

    /// Sends the removal: the locked account's id, never a typed value.
    fn confirm_remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.remove_dialog.as_ref() else { return };
        if !dialog.acknowledged || dialog.sent.is_some() {
            return;
        }
        // Only the locked account is removed; any other target closes the
        // dialog without sending.
        if self.unlock_target(cx).as_ref() != Some(&dialog.account) {
            self.remove_dialog = None;
            cx.notify();
            return;
        }
        let account = dialog.account.clone();
        let mark = self.store.read(cx).entry_store().failures_total();
        let result = self.root.update(cx, |root, cx| {
            root.submit_secret(IntentKind::RemoveLocalAccount { account }, window, cx)
        });
        if let Some(dialog) = self.remove_dialog.as_mut() {
            if result.starts_with("submitted") {
                dialog.sent = Some(mark);
                dialog.failed = false;
            } else {
                dialog.failed = true;
            }
        }
        cx.notify();
    }

    /// Follows the store: the dialog ends when its account is gone (the
    /// removal worked), is no longer the lock page's account, or when its
    /// request failed.
    pub(super) fn sync_remove(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.remove_dialog.as_ref() else { return };
        let target = self.unlock_target(cx);
        let store = self.store.read(cx).entry_store();
        if store.account(&dialog.account).is_none() {
            self.removed_notice = self.remove_dialog.take().map(|dialog| dialog.name);
            cx.notify();
            return;
        }
        if target.as_ref() != Some(&dialog.account) {
            self.remove_dialog = None;
            cx.notify();
            return;
        }
        if let Some(mark) = dialog.sent
            && store.failures_since(mark).iter().any(|f| f.method == methods::ACCOUNT_REMOVE_LOCAL)
            && let Some(dialog) = self.remove_dialog.as_mut()
        {
            dialog.sent = None;
            dialog.failed = true;
            cx.notify();
        }
    }

    /// The dialog, when it is open.
    pub(super) fn render_remove_dialog(
        &self,
        focus: &FocusHandle,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let dialog = self.remove_dialog.as_ref()?;
        let name = dialog.name.clone();
        let short = super::parts::short_id(dialog.account.as_str());
        let this = cx.entity().downgrade();
        let close = {
            let this = this.clone();
            move |_: &mut Window, cx: &mut gpui_kit::App| {
                this.update(cx, |page, cx| page.close_remove(cx)).ok();
            }
        };
        let acknowledged = dialog.acknowledged;
        let sending = dialog.sent.is_some();
        let toggle = {
            let this = this.clone();
            move |checked: &bool, _: &mut Window, cx: &mut gpui_kit::App| {
                let checked = *checked;
                this.update(cx, |page, cx| {
                    if let Some(dialog) = page.remove_dialog.as_mut() {
                        dialog.acknowledged = checked;
                    }
                    cx.notify();
                })
                .ok();
            }
        };
        let body = v_flex()
            .id("lock-remove-body")
            .gap(space(3.))
            .child(
                v_flex()
                    .id("lock-remove-account")
                    .child(
                        div()
                            .id("lock-remove-name")
                            .text_size(text_secondary())
                            .font_semibold()
                            .child(name),
                    )
                    .child(
                        div()
                            .id("lock-remove-short-id")
                            .text_size(text_caption())
                            .text_color(cx.theme().muted_foreground)
                            .child(short),
                    ),
            )
            .child(div().text_size(text_secondary()).child(tr("entry.remove.body")))
            .child(
                DesignCheckbox::new("lock-remove-ack")
                    .checked(acknowledged)
                    .disabled(sending)
                    .label(tr("entry.remove.ack"))
                    .on_click(toggle),
            )
            .children(dialog.failed.then(|| {
                div()
                    .id("lock-remove-failed")
                    .role(gpui_kit::Role::Alert)
                    .text_size(text_caption())
                    .text_color(cx.theme().danger)
                    .child(tr("entry.remove.failed"))
            }));
        Some(
            Modal::new("lock-remove-dialog", tr("entry.remove.title"), focus, close.clone())
                .width(modal::width::NARROW)
                .child(body)
                .button(modal::cancel_button("lock-remove-cancel", tr("common.cancel"), cx, close))
                .button(
                    gpui_kit::component::button::Button::new("lock-remove-confirm")
                        .design_danger(cx)
                        .design_label(tr("entry.remove.confirm"))
                        .loading(sending)
                        .design_disabled(!acknowledged || sending, cx)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.confirm_remove(window, cx)),
                        ),
                )
                .render(cx),
        )
    }
}
