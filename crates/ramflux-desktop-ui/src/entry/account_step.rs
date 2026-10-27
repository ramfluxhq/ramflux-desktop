// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The account step: unlock an existing account, or create an account, join
//! an identity as a new device, or restore a backup on a confirmed node.
//!
//! Each form hands its secret to the intent root as a `SecretInput` and
//! clears the masked input at once.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, Entity, IntoElement, SharedString, Window, div};
use ramflux_desktop_core::{
    Account, AccountKind, BackupPhase, IntentKind, LockState, NodeId, SecretInput, Store,
    UiEventKind, methods,
};
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr;

use super::{AccountPath, EntryPage, RestoreKind};
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
        let awaiting = self.awaits_restore_listing(cx);
        let body = if awaiting {
            // The session that took over is empty until its first jobs
            // listing: the restore result as last seen (or a plain line)
            // stays, never an earlier step or an unlock form.
            let held = self.full_restore.read(cx).held_job(cx).is_some();
            v_flex()
                .id("entry-restore-finishing")
                .test_support()
                .gap_3()
                .children(held.then(|| self.full_restore.clone()))
                .child(
                    h_flex()
                        .id("entry-restore-finishing-line")
                        .gap_2()
                        .items_center()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(Spinner::new().small())
                        .child(tr("entry.restore.finishing")),
                )
                .into_any_element()
        } else if self.awaits_session(cx) {
            self.render_session_wait(cx).into_any_element()
        } else if accounts.is_empty() {
            self.render_paths(&store, cx).into_any_element()
        } else {
            // A full restore that just finished (the account it installed
            // is listed now) stays in view until closed: above the unlock
            // when the account is locked; with "Open account" when the
            // daemon reopened it already (the shell holds this page).
            let shown = self.full_restore.read(cx).followed_job(cx);
            let open = shown.as_ref().filter(|job| {
                job.phase == BackupPhase::Done
                    && accounts.iter().any(|a| a.lock_state == LockState::Unlocked)
            });
            let open_account = open.map(|job| {
                let id = job.id.clone();
                h_flex().child(
                    Button::new("entry-open-account")
                        .primary()
                        .label(tr("entry.restore.open_account"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let id = id.clone();
                            this.full_restore.update(cx, |panel, cx| panel.dismiss(id, cx));
                        })),
                )
            });
            v_flex()
                .gap_4()
                .children(shown.is_some().then(|| self.full_restore.clone()))
                .children(open_account)
                .child(self.render_unlock(&accounts, cx))
                .into_any_element()
        };
        v_flex().id("entry-account").gap_3().child(body)
    }

    /// A freshly opened account session that has not answered: a neutral
    /// loading line, and once the bound passed (or a read failed) the
    /// error with the ways out.
    fn render_session_wait(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.session_failed(cx) {
            return h_flex()
                .id("entry-session-loading")
                .test_support()
                .gap_2()
                .items_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(Spinner::new().small())
                .child(tr("entry.account.loading"))
                .into_any_element();
        }
        let account = self.store.read(cx).active_account().cloned();
        // Named by the label the daemon keeps for it (its id when none is
        // known yet): the account's own session has not answered, so the
        // daemon's own listing is read too.
        let name = account.as_ref().map_or_else(String::new, |id| {
            let handle = self.store.read(cx);
            let label = handle
                .daemon_store()
                .into_iter()
                .chain(std::iter::once(handle.store()))
                .find_map(|store| store.account(id).map(|a| a.label.trim().to_owned()))
                .filter(|label| !label.is_empty());
            label.unwrap_or_else(|| id.to_string())
        });
        v_flex()
            .id("entry-session-failed")
            .test_support()
            .gap_3()
            .child({
                let text = tr_args("entry.account.load_failed", &[("name", &name)]);
                div()
                    .id("entry-session-failed-text")
                    .aria_label(text.clone())
                    .test_support()
                    .child(Alert::error("entry-session-failed-alert", text))
            })
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("entry-session-retry")
                            .primary()
                            .label(tr("entry.account.retry"))
                            .on_click(cx.listener(|this, _, _, cx| this.retry_session(cx))),
                    )
                    .children(account.map(|account_id| {
                        let lock = LockLabel::new(&name);
                        Button::new("entry-session-lock")
                            .outline()
                            .label(lock.visible)
                            // Only the visible text is clipped: the control is
                            // still named, and the tooltip shows, the whole label.
                            .accessibility_label(lock.full)
                            .when_some(lock.tooltip, |button, full| button.tooltip(full))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let account_id = account_id.clone();
                                this.sink.emit(UiEventKind::LockPressed { account_id }, window, cx);
                            }))
                    })),
            )
            .into_any_element()
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
        let kinds = [
            (RestoreKind::Full, tr("entry.restore.full")),
            (RestoreKind::Key, tr("entry.restore.key")),
        ];
        let kind = self.restore_kind.unwrap_or_else(|| {
            let store = self.store.read(cx).store();
            if store.unsupported_reason(methods::RESTORE_VERIFY).is_some() {
                RestoreKind::Key
            } else {
                RestoreKind::Full
            }
        });
        let choice = self.segment(
            "entry-restore-kind",
            &kinds,
            kind,
            |this, kind| this.restore_kind = Some(kind),
            cx,
        );
        let muted = cx.theme().muted_foreground;
        if kind == RestoreKind::Full {
            return v_flex()
                .gap_3()
                .child(choice)
                .child(div().text_sm().text_color(muted).child(tr("entry.restore.full_help")))
                .child(
                    div().id("entry-full-restore").test_support().child(self.full_restore.clone()),
                )
                .into_any_element();
        }
        v_flex()
            .gap_3()
            .child(choice)
            .child(div().text_sm().text_color(muted).child(tr("entry.restore.key_help")))
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

/// How many terminal cells of an account label the Lock button shows
/// (a CJK character or an emoji takes two).
const LOCK_LABEL_CELLS: usize = 24;

/// The Lock button's text for an account named `name`.
#[derive(Debug, PartialEq, Eq)]
struct LockLabel {
    /// What the button shows: the label clipped to its width budget.
    visible: SharedString,
    /// The whole label: the control's accessible name.
    full: SharedString,
    /// The whole account name, when the visible text was clipped, so the
    /// way back to it stays one hover away.
    tooltip: Option<SharedString>,
}

impl LockLabel {
    fn new(name: &str) -> Self {
        let (short, clipped) = clip_label(name, LOCK_LABEL_CELLS);
        Self {
            visible: tr_args("entry.account.lock", &[("name", &short)]),
            full: tr_args("entry.account.lock", &[("name", name)]),
            tooltip: clipped.then(|| SharedString::from(name.to_owned())),
        }
    }
}

/// `label` cut to at most `max` terminal cells with a trailing ellipsis, and
/// whether it was cut. The cut falls between grapheme clusters (a combining
/// mark or a joined emoji is never split) and wide characters count double,
/// so a CJK label is not twice as wide as a Latin one. The error text above
/// the button keeps the full label.
fn clip_label(label: &str, max: usize) -> (String, bool) {
    if UnicodeWidthStr::width(label) <= max {
        return (label.to_owned(), false);
    }
    // One cell is kept for the ellipsis.
    let budget = max.saturating_sub(1);
    let mut head = String::new();
    let mut used = 0;
    for cluster in label.graphemes(true) {
        let width = UnicodeWidthStr::width(cluster);
        if used + width > budget {
            break;
        }
        used += width;
        head.push_str(cluster);
    }
    (format!("{}…", head.trim_end()), true)
}

#[cfg(test)]
mod tests {
    use super::{LOCK_LABEL_CELLS, LockLabel, clip_label};
    use unicode_segmentation::UnicodeSegmentation as _;
    use unicode_width::UnicodeWidthStr;

    fn cells(text: &str) -> usize {
        UnicodeWidthStr::width(text)
    }

    /// The label, clipped, with everything the button needs from it.
    fn assert_clipped(name: &str) -> LockLabel {
        let lock = LockLabel::new(name);
        assert!(lock.visible.ends_with('…'), "the visible text is clipped: {}", lock.visible);
        assert!(!lock.visible.contains(name));
        assert!(cells(&lock.visible) <= cells("Lock ") + LOCK_LABEL_CELLS + 4, "{}", lock.visible);
        assert_eq!(lock.tooltip.as_deref(), Some(name), "the tooltip is the whole name");
        assert!(lock.full.ends_with(name), "the accessible name is whole: {}", lock.full);
        assert_ne!(lock.visible, lock.full);
        lock
    }

    #[test]
    fn a_short_label_is_kept_whole_with_no_tooltip() {
        assert_eq!(clip_label("Personal", 24), ("Personal".to_owned(), false));
        let lock = LockLabel::new("Personal");
        assert_eq!(lock.visible, lock.full);
        assert_eq!(lock.tooltip, None);
    }

    #[test]
    fn a_long_latin_label_is_clipped_with_a_whole_tooltip_and_accessible_name() {
        let name = "Quarterly planning and archive account of the operations team";
        let (short, clipped) = clip_label(name, 24);
        assert!(clipped);
        assert!((22..=24).contains(&cells(&short)), "{short}");
        assert_clipped(name);
        let accented = "Ünïcödé ".repeat(10);
        assert_clipped(&accented);
    }

    #[test]
    fn a_long_cjk_label_is_clipped_by_width_not_by_character_count() {
        let name = "\u{7b2c}\u{56db}\u{5b63}\u{5ea6}\u{8fd0}\u{8425}\u{56e2}\u{961f}\u{7684}\u{89c4}\u{5212}\u{4e0e}\u{5f52}\u{6863}\u{8d26}\u{6237}\u{7b2c}\u{56db}\u{5b63}\u{5ea6}\u{8fd0}\u{8425}\u{56e2}\u{961f}";
        let (short, clipped) = clip_label(name, 24);
        assert!(clipped);
        assert!(cells(&short) <= 24, "{short} is {} cells", cells(&short));
        // Eleven wide characters and the ellipsis: not 23 characters.
        assert_eq!(short.chars().count(), 12);
        assert_clipped(name);
        // Twelve wide characters fit exactly; one more does not.
        assert!(!clip_label(&"\u{8d26}".repeat(12), 24).1);
        assert!(clip_label(&"\u{8d26}".repeat(13), 24).1);
    }

    #[test]
    fn a_cut_never_splits_a_combining_mark_or_a_joined_emoji() {
        let marks = "e\u{301}".repeat(30);
        let (short, clipped) = clip_label(&marks, 24);
        assert!(clipped);
        let body = short.trim_end_matches('…');
        assert!(body.graphemes(true).all(|g| g == "e\u{301}"), "{short:?}");
        assert_eq!(body.graphemes(true).count(), 23);
        assert_clipped(&marks);

        let family = "👨\u{200d}👩\u{200d}👧\u{200d}👦";
        let name = format!("Team {}", family.repeat(12));
        let (short, clipped) = clip_label(&name, 24);
        assert!(clipped);
        let body = short.trim_end_matches('…').trim_start_matches("Team ");
        assert!(
            body.graphemes(true).all(|g| g == family),
            "only whole emoji sequences remain: {short:?}"
        );
        assert!(cells(&short) <= 24);
        assert_clipped(&name);
    }
}
