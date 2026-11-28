// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The confirmation of the actions of an MLS group that cannot be taken
//! back or that admit or remove people: revoking an invite, deleting an own
//! post for everyone, leaving, approving or declining a join request, a role
//! change, removing and banning a member, undoing a change, skipping the
//! catch-up and turning approval off.
//!
//! One tap on the confirm button emits the action's event, once; Escape, the
//! scrim and the cancel button close it with no action. Return does not
//! activate the confirm button: the dialog holds focus itself, and the
//! buttons are pressed by a click or by Space while focused. Nothing is
//! confirmed by itself.
//!
//! A confirmation names the person it is about: the display name, "this
//! member" when there is none (never an id), the AI marker with its note when
//! the SDK said the account is an AI, and the group name. A target that
//! changed meanwhile still reads "this member"; the policy refuses the call.

use crate::style::DesignButton as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, Entity, FocusHandle, IntoElement, Render, SharedString, Subscription, Window, div,
};
use ramflux_desktop_core::mls_group::{MlsPanel, OpaqueId, SettingsField};
use ramflux_desktop_core::{Scope, Store, Timestamp, UiEventKind};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};

use super::{group_name, name_text, when};
use crate::app::StoreHandle;
use crate::bubble::EventSink;
use crate::i18n::{tr, tr_args};

/// Hours a join request lives on the host.
const REQUEST_LIFETIME_MS: i64 = 24 * 3_600_000;

/// The confirmation dialog and what it asks.
pub struct MlsConfirm {
    store: Entity<StoreHandle>,
    sink: EventSink,
    panel: Option<MlsPanel>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

/// How the dialog reads: the title, the body paragraphs, the confirm label,
/// and whether the confirm is the destructive style.
struct Text {
    title: SharedString,
    body: Vec<SharedString>,
    confirm: SharedString,
    cancel: SharedString,
    destructive: bool,
    /// The person the question is about (name, AI marker).
    about: Option<(SharedString, bool)>,
    /// The hint actions of the approval-off question.
    hint: bool,
}

/// The name a confirmation shows: the display name (bidi-isolated), or "this
/// member" when there is none (never an id).
fn who(display: Option<&str>) -> SharedString {
    match display.filter(|d| !d.is_empty()) {
        Some(name) => name_text(Some(name), false),
        None => tr("group.member.this"),
    }
}

/// A person as a confirmation names them.
fn person(store: &Store, group: &str, anchor: &OpaqueId) -> (SharedString, bool) {
    match store.mls_members(group).and_then(|p| p.rows.get(anchor.expose())) {
        Some(row) => {
            (who(row.display.as_deref()), row.ai_flag || store.mls_is_ai(group, anchor.expose()))
        }
        None => (who(None), store.mls_is_ai(group, anchor.expose())),
    }
}

/// A requester as a confirmation names them.
fn requester(store: &Store, group: &str, id: &OpaqueId) -> (SharedString, bool, Option<Timestamp>) {
    match store.mls_requests(group).and_then(|p| p.row(id)) {
        Some(row) => {
            let request = &row.request;
            let ai = request.ai_flag || store.mls_is_ai(group, request.anchor.expose());
            let expires = request
                .expires_at
                .or_else(|| request.requested_at.map(|at| Timestamp(at.0 + REQUEST_LIFETIME_MS)));
            (who(request.display.as_deref()), ai, expires)
        }
        None => (who(None), false, None),
    }
}

impl MlsConfirm {
    /// A closed dialog.
    pub fn new(
        store: Entity<StoreHandle>,
        sink: EventSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            // The window going to the background closes it with no action.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.close(cx);
                }
            }),
        ];
        Self {
            store,
            sink: sink.scoped(Scope::Panel("mls_confirm".to_owned())),
            panel: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Asks the question `panel` names (a confirm panel); anything else is
    /// not a question.
    pub fn open(&mut self, panel: MlsPanel, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            panel,
            MlsPanel::ConfirmDelete { .. }
                | MlsPanel::ConfirmLeave { .. }
                | MlsPanel::ConfirmRevoke { .. }
                | MlsPanel::ConfirmApprove { .. }
                | MlsPanel::ConfirmDecline { .. }
                | MlsPanel::ConfirmRole { .. }
                | MlsPanel::ConfirmRemove { .. }
                | MlsPanel::ConfirmBan { .. }
                | MlsPanel::ConfirmVoid { .. }
                | MlsPanel::ConfirmSkip { .. }
                | MlsPanel::ConfirmApprovalOff { .. }
        ) {
            self.panel = Some(panel);
            window.focus(&self.focus, cx);
            cx.notify();
        }
    }

    /// Closes the dialog with no action.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.panel.take().is_some() {
            cx.notify();
        }
    }

    /// Whether a question is open.
    pub fn is_open(&self) -> bool {
        self.panel.is_some()
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = self.panel.take() else { return };
        let kind = match panel {
            MlsPanel::ConfirmDelete { group_id, post_id } => {
                UiEventKind::MlsDeletePressed { group_id, post_id }
            }
            MlsPanel::ConfirmLeave { group_id } => UiEventKind::MlsLeavePressed { group_id },
            MlsPanel::ConfirmRevoke { group_id, invite_id } => {
                UiEventKind::MlsInviteRevokePressed { group_id, invite_id }
            }
            MlsPanel::ConfirmApprove { group_id, request_id } => {
                UiEventKind::MlsApprovePressed { group_id, request_id }
            }
            MlsPanel::ConfirmDecline { group_id, request_id } => {
                UiEventKind::MlsDeclinePressed { group_id, request_id }
            }
            MlsPanel::ConfirmRole { group_id, anchor, admin } => {
                UiEventKind::MlsRolePressed { group_id, anchor, admin }
            }
            MlsPanel::ConfirmRemove { group_id, anchor } => {
                UiEventKind::MlsRemovePressed { group_id, anchor }
            }
            MlsPanel::ConfirmBan { group_id, anchor } => {
                UiEventKind::MlsBanPressed { group_id, anchor }
            }
            MlsPanel::ConfirmVoid { group_id, log_seq } => {
                UiEventKind::MlsVoidPressed { group_id, log_seq }
            }
            MlsPanel::ConfirmSkip { group_id } => UiEventKind::MlsSkipPressed { group_id },
            MlsPanel::ConfirmApprovalOff { group_id } => {
                UiEventKind::MlsSettingsPressed { group_id, field: SettingsField::Approval(false) }
            }
            _ => return,
        };
        self.sink.emit(kind, window, cx);
        cx.notify();
    }

    /// A hint action: opens a panel (nothing is created or copied) and closes
    /// the question with no action.
    fn hint(&mut self, panel: MlsPanel, window: &mut Window, cx: &mut Context<Self>) {
        self.panel = None;
        self.sink.emit(UiEventKind::MlsPanelRequested { panel }, window, cx);
        cx.notify();
    }

    fn text(&self, panel: &MlsPanel, cx: &gpui_kit::App) -> Text {
        let store = self.store.read(cx).store();
        let group_of = |id: &str| {
            store.mls_group(id).map_or_else(|| tr("group.info.title"), |g| group_name(&g.name))
        };
        let plain = |title: SharedString, body: Option<SharedString>, confirm: SharedString| Text {
            title,
            body: body.into_iter().collect(),
            confirm,
            cancel: tr("common.cancel"),
            destructive: true,
            about: None,
            hint: false,
        };
        match panel {
            MlsPanel::ConfirmDelete { .. } => {
                plain(tr("group.delete.title"), None, tr("group.delete.confirm"))
            }
            MlsPanel::ConfirmLeave { .. } => plain(
                tr("group.leave.title"),
                Some(tr("group.leave.body")),
                tr("group.leave.confirm"),
            ),
            MlsPanel::ConfirmRevoke { .. } => plain(
                tr("group.invite.revoke_confirm.title"),
                Some(tr("group.invite.revoke_confirm.body")),
                tr("group.invite.revoke"),
            ),
            MlsPanel::ConfirmApprove { group_id, request_id } => {
                let (name, ai, _) = requester(store, group_id, request_id);
                Text {
                    title: tr_args(
                        "group.requests.approve_confirm.title",
                        &[("name", &name), ("group", &group_of(group_id))],
                    ),
                    body: vec![tr("group.requests.approve_confirm.body")],
                    confirm: tr("group.requests.approve_confirm.confirm"),
                    cancel: tr("common.cancel"),
                    destructive: false,
                    about: Some((name, ai)),
                    hint: false,
                }
            }
            MlsPanel::ConfirmDecline { group_id, request_id } => {
                let (name, ai, expires) = requester(store, group_id, request_id);
                let now = self.store.read(cx).session().map(|s| s.now()).unwrap_or_default();
                let expires = expires.unwrap_or(Timestamp(now.0 + REQUEST_LIFETIME_MS));
                Text {
                    title: tr_args("group.requests.decline_confirm.title", &[("name", &name)]),
                    body: vec![tr_args(
                        "group.requests.decline_confirm.body",
                        &[("time", &when(expires))],
                    )],
                    confirm: tr("group.requests.decline_confirm.confirm"),
                    cancel: tr("common.cancel"),
                    destructive: false,
                    about: Some((name, ai)),
                    hint: false,
                }
            }
            MlsPanel::ConfirmRole { group_id, anchor, admin } => {
                let (name, ai) = person(store, group_id, anchor);
                let (title_key, body_key, confirm_key) = if *admin {
                    (
                        "group.member.admin_confirm.title",
                        "group.member.admin_confirm.body",
                        "group.member.admin_confirm.confirm",
                    )
                } else {
                    (
                        "group.member.demote_confirm.title",
                        "group.member.demote_confirm.body",
                        "group.member.demote_confirm.confirm",
                    )
                };
                Text {
                    title: tr_args(title_key, &[("name", &name), ("group", &group_of(group_id))]),
                    body: vec![tr(body_key)],
                    confirm: tr(confirm_key),
                    cancel: tr("common.cancel"),
                    destructive: false,
                    about: Some((name, ai)),
                    hint: false,
                }
            }
            MlsPanel::ConfirmRemove { group_id, anchor } => {
                let (name, ai) = person(store, group_id, anchor);
                Text {
                    title: tr_args(
                        "group.member.remove_confirm.title",
                        &[("name", &name), ("group", &group_of(group_id))],
                    ),
                    body: vec![tr("group.member.remove_confirm.body")],
                    confirm: tr("group.member.remove_confirm.confirm"),
                    cancel: tr("common.cancel"),
                    destructive: true,
                    about: Some((name, ai)),
                    hint: false,
                }
            }
            MlsPanel::ConfirmBan { group_id, anchor } => {
                let (name, ai) = person(store, group_id, anchor);
                Text {
                    title: tr_args(
                        "group.member.ban_confirm.title",
                        &[("name", &name), ("group", &group_of(group_id))],
                    ),
                    body: vec![tr("group.member.ban_confirm.body")],
                    confirm: tr("group.member.ban_confirm.confirm"),
                    cancel: tr("common.cancel"),
                    destructive: true,
                    about: Some((name, ai)),
                    hint: false,
                }
            }
            MlsPanel::ConfirmVoid { .. } => {
                plain(tr("group.void.title"), Some(tr("group.void.body")), tr("group.void.confirm"))
            }
            MlsPanel::ConfirmSkip { .. } => Text {
                title: tr("group.skip.title"),
                body: vec![tr("group.skip.body"), tr("group.skip.body_more")],
                confirm: tr("group.skip.confirm"),
                cancel: tr("group.skip.cancel"),
                destructive: true,
                about: None,
                hint: false,
            },
            _ => Text {
                title: tr("group.settings.approval_off_confirm.title"),
                body: vec![
                    tr("group.settings.approval_off_confirm.body"),
                    tr("group.hint.agent.desktop"),
                ],
                confirm: tr("group.settings.approval_off_confirm.confirm"),
                cancel: tr("common.cancel"),
                destructive: true,
                about: None,
                hint: true,
            },
        }
    }
}

impl Render for MlsConfirm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(panel) = self.panel.clone() else {
            return div().id("mls-confirm").into_any_element();
        };
        let text = self.text(&panel, cx);
        let muted = cx.theme().muted_foreground;
        let this = cx.weak_entity();
        // Cancel is the default (non-destructive) button of a destructive
        // question; it never sends anything.
        let cancel = Button::new("mls-confirm-cancel")
            .design_secondary(cx)
            .design_label(text.cancel.clone())
            .on_click(cx.listener(|this, _, _, cx| this.close(cx)));
        let ok = {
            let button = Button::new("mls-confirm-ok");
            let button =
                if text.destructive { button.design_danger(cx) } else { button.design_primary(cx) };
            button
                .design_label(text.confirm.clone())
                .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx)))
        };
        let hint = text.hint.then(|| {
            let group = match &panel {
                MlsPanel::ConfirmApprovalOff { group_id } => group_id.clone(),
                _ => String::new(),
            };
            h_flex()
                .id("mls-confirm-hint")
                .gap(space(2.))
                .child(
                    Button::new("mls-confirm-hint-invite")
                        .design_secondary(cx)
                        .design_label(tr("group.hint.agent.invite"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.hint(MlsPanel::Invites { group_id: group.clone() }, window, cx);
                        })),
                )
                .child(
                    Button::new("mls-confirm-hint-agents")
                        .design_secondary(cx)
                        .design_label(tr("group.hint.agent.open_agents"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.hint(MlsPanel::Agents, window, cx);
                        })),
                )
        });
        let about = text.about.clone().map(|(name, ai)| {
            v_flex()
                .id("mls-confirm-about")
                .test_support()
                .gap(space(1.))
                .child(
                    h_flex()
                        .gap(space(2.))
                        .child(div().text_size(text_secondary()).child(name))
                        .children(ai.then(|| crate::identity::ai_mark("mls-confirm-ai", cx))),
                )
                .children(ai.then(|| {
                    div()
                        .text_size(text_caption())
                        .text_color(muted)
                        .child(tr("group.member.agent_note"))
                }))
        });
        let mut modal =
            crate::modal::Modal::new("mls-confirm", text.title, &self.focus, move |_, cx| {
                this.update(cx, |dialog, cx| dialog.close(cx)).ok();
            })
            .width(crate::modal::width::NARROW)
            .children_of(about);
        for (ix, paragraph) in text.body.into_iter().enumerate() {
            modal = modal.child(
                div()
                    .id(SharedString::from(format!("mls-confirm-body-{ix}")))
                    .test_support()
                    .text_size(text_secondary())
                    .child(paragraph),
            );
        }
        modal.children_of(hint).button(cancel).button(ok).render(cx)
    }
}
