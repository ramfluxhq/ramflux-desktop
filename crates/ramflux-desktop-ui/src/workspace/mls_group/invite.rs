// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The invite panel (derived design D-M2-1, from the contact card sheet).
//!
//! Any member may create invites (r4); the list and revoke cover the invites
//! this account created. The panel shows the secret note, then the newest
//! link (monospace, middle-truncated, left to right) with "Copy link", the
//! duration and use presets (1 or 7 days; 1, 10 or 100 uses) with "New
//! link", and the list of invites with their "Revoked" and "Expired" badges.
//!
//! A link is a secret. It is written to the clipboard on the tap of "Copy
//! link" only; the desktop has no clipboard expiry or sensitivity flag, so
//! the entry stays until something else is copied. It is never shown in a
//! log, a notice or another view. Revoke asks for a confirmation, and a row
//! shows "Revoked" only after the reply (S6). The desktop shows no QR code in
//! this batch.

use crate::style::DesignButton as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Selectable as _, Sizable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    ClipboardItem, Context, Entity, FocusHandle, IntoElement, Render, Role, SharedString,
    Subscription, Window, div,
};
use ramflux_desktop_core::mls_group::{self as mls, Invite};
use ramflux_desktop_core::{INVITE_TTLS, INVITE_USES, Scope, Timestamp, UiEventKind, methods};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};

use super::when;
use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args, tr_count};

/// Longest link text shown before the middle is cut.
const SHOWN_CHARS: usize = 32;

/// The link with its middle cut when it is long, kept left to right.
pub fn truncated(link: &str) -> String {
    let chars: Vec<char> = link.chars().collect();
    let text = if chars.len() > SHOWN_CHARS {
        let head: String = chars[..SHOWN_CHARS / 2].iter().collect();
        let tail: String = chars[chars.len() - (SHOWN_CHARS / 2 - 1)..].iter().collect();
        format!("{head}\u{2026}{tail}")
    } else {
        link.to_owned()
    };
    i18n::bidi::ltr(&text)
}

/// The panel and its view-local state.
pub struct MlsInvites {
    store: Entity<StoreHandle>,
    sink: EventSink,
    group: Option<String>,
    ttl: usize,
    uses: usize,
    /// The invite whose link was copied last ("Link copied").
    copied: Option<String>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl MlsInvites {
    /// A closed panel.
    pub fn new(
        store: Entity<StoreHandle>,
        sink: EventSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.subscribe(&store, |this, _, event: &StoreEvent, cx| {
                if matches!(event, StoreEvent::Changed(_)) && this.group.is_some() {
                    cx.notify();
                }
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.close(cx);
                }
            }),
        ];
        Self {
            store,
            sink: sink.scoped(Scope::Panel("mls_invites".to_owned())),
            group: None,
            ttl: 1,
            uses: 2,
            copied: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Opens the panel of `group_id`, reading its invites once.
    pub fn open(&mut self, group_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.group = Some(group_id.clone());
        self.copied = None;
        window.focus(&self.focus, cx);
        // Right after a create the one invite is known; otherwise the list
        // of this account's invites is read once.
        if self.store.read(cx).store().mls_invites(&group_id).is_empty() {
            self.sink.emit(UiEventKind::MlsInviteListRequested { group_id }, window, cx);
        }
        cx.notify();
    }

    /// Closes the panel (the copied mark and the links in view go with it).
    pub fn close(&mut self, cx: &mut Context<Self>) {
        if self.group.take().is_some() {
            self.copied = None;
            cx.notify();
        }
    }

    /// Whether the panel is open.
    pub fn is_open(&self) -> bool {
        self.group.is_some()
    }

    fn copy(&mut self, invite: &Invite, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(invite.link.expose().to_owned()));
        self.copied = Some(invite.invite_id.clone());
        cx.notify();
    }

    fn new_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(group_id) = self.group.clone() else { return };
        self.sink.emit(
            UiEventKind::MlsInviteCreatePressed {
                group_id,
                max_uses: INVITE_USES[self.uses],
                ttl_s: INVITE_TTLS[self.ttl],
            },
            window,
            cx,
        );
    }

    fn ask_revoke(&mut self, invite_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(group_id) = self.group.clone() else { return };
        self.sink.emit(
            UiEventKind::MlsPanelRequested {
                panel: mls::MlsPanel::ConfirmRevoke { group_id, invite_id },
            },
            window,
            cx,
        );
    }
}

/// A preset button of a row.
fn preset(
    id: SharedString,
    label: SharedString,
    selected: bool,
    on_click: impl Fn(&mut Window, &mut gpui_kit::App) + 'static,
) -> Button {
    Button::new(id)
        .ghost()
        .small()
        .selected(selected)
        .toggled(selected)
        .label(label)
        .on_click(move |_, window, cx| on_click(window, cx))
}

impl Render for MlsInvites {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(group) = self.group.clone() else {
            return div().id("mls-invites").into_any_element();
        };
        let handle = self.store.read(cx);
        let store = handle.store();
        let now = handle.session().map(|s| s.now()).unwrap_or_default();
        let invites: Vec<Invite> = store.mls_invites(&group).to_vec();
        let failure = store
            .mls_failure(methods::MLS_GROUP_INVITE_CREATE, Some(&group))
            .or_else(|| store.mls_failure(methods::MLS_GROUP_INVITE_REVOKE, Some(&group)))
            .map(|code| tr(mls::error_key(code)));
        let muted = cx.theme().muted_foreground;
        let danger = crate::tone::text(cx.theme().danger, cx);
        let newest = invites.iter().find(|i| !i.revoked && i.expires_at > now).cloned();
        let this = cx.weak_entity();

        let note = div()
            .id("mls-invite-note")
            .test_support()
            .text_size(text_secondary())
            .child(tr("group.invite.secret_note"));
        let link_block = newest.map(|invite| {
            let copied = self.copied.as_ref() == Some(&invite.invite_id);
            let for_copy = invite.clone();
            v_flex()
                .gap(space(1.))
                .child(
                    div()
                        .id("mls-invite-link")
                        .test_support()
                        .font_family("monospace")
                        .text_size(text_secondary())
                        .child(truncated(invite.link.expose())),
                )
                .child(
                    h_flex()
                        .gap(space(2.))
                        .child(
                            Button::new("mls-invite-copy")
                                .design_secondary(cx)
                                .design_label(tr("group.invite.copy"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.copy(&for_copy, cx);
                                })),
                        )
                        .children(copied.then(|| {
                            div()
                                .id("mls-invite-copied")
                                .role(Role::Status)
                                .test_support()
                                .text_size(text_caption())
                                .text_color(muted)
                                .child(tr("group.invite.copied"))
                        })),
                )
        });
        let ttl_row = h_flex().gap(space(1.)).flex_wrap().children(
            INVITE_TTLS.iter().enumerate().map(|(ix, seconds)| {
                let days = seconds / 86_400;
                preset(
                    SharedString::from(format!("mls-invite-ttl-{ix}")),
                    tr_count("group.invite.duration.day", days),
                    self.ttl == ix,
                    {
                        let this = this.clone();
                        move |_, cx| {
                            this.update(cx, |panel, cx| {
                                panel.ttl = ix;
                                cx.notify();
                            })
                            .ok();
                        }
                    },
                )
            }),
        );
        let uses_row = h_flex().gap(space(1.)).flex_wrap().children(
            INVITE_USES.iter().enumerate().map(|(ix, uses)| {
                preset(
                    SharedString::from(format!("mls-invite-uses-{ix}")),
                    tr_count("group.invite.max_uses", u64::from(*uses)),
                    self.uses == ix,
                    {
                        let this = this.clone();
                        move |_, cx| {
                            this.update(cx, |panel, cx| {
                                panel.uses = ix;
                                cx.notify();
                            })
                            .ok();
                        }
                    },
                )
            }),
        );
        let create = v_flex()
            .gap(space(2.))
            .child(
                div()
                    .text_size(text_caption())
                    .text_color(muted)
                    .child(tr("group.invite.duration")),
            )
            .child(ttl_row)
            .child(div().text_size(text_caption()).text_color(muted).child(tr("group.invite.uses")))
            .child(uses_row)
            .child(
                h_flex().child(
                    Button::new("mls-invite-new")
                        .design_primary(cx)
                        .design_label(tr("group.invite.new"))
                        .on_click(cx.listener(|this, _, window, cx| this.new_link(window, cx))),
                ),
            );
        let rows = invites.iter().map(|invite| {
            let expired = invite.expires_at <= now && !invite.revoked;
            let badge = if invite.revoked {
                Some(tr("group.invite.revoked"))
            } else if expired {
                Some(tr("group.invite.expired"))
            } else {
                None
            };
            let detail = i18n::join_all(&[
                tr_args("group.invite.expires", &[("time", &when(invite.expires_at))]).to_string(),
                match invite.uses_left {
                    Some(left) => tr_args(
                        "group.invite.uses_left",
                        &[("left", &left.to_string()), ("total", &invite.max_uses.to_string())],
                    )
                    .to_string(),
                    None => {
                        tr_count("group.invite.max_uses", u64::from(invite.max_uses)).to_string()
                    }
                },
            ]);
            let id = invite.invite_id.clone();
            h_flex()
                .id(SharedString::from(format!("mls-invite-row-{}", invite.invite_id)))
                .test_support()
                .gap(space(2.))
                .py(space(1.))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .font_family("monospace")
                                .text_size(text_caption())
                                .child(truncated(invite.link.expose())),
                        )
                        .child(div().text_size(text_caption()).text_color(muted).child(detail)),
                )
                .children(
                    badge.map(|text| div().text_size(text_caption()).text_color(muted).child(text)),
                )
                .children((!invite.revoked && !expired).then(|| {
                    Button::new(SharedString::from(format!(
                        "mls-invite-revoke-{}",
                        invite.invite_id
                    )))
                    .ghost()
                    .small()
                    .label(tr("group.invite.revoke"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.ask_revoke(id.clone(), window, cx);
                    }))
                }))
        });
        let list = (!invites.is_empty()).then(|| {
            v_flex()
                .gap(space(1.))
                .child(
                    div()
                        .text_size(text_caption())
                        .text_color(muted)
                        .child(tr("group.invite.list")),
                )
                .children(rows)
        });
        let _ = (window, Timestamp(0));
        let body = v_flex()
            .id("mls-invite-body")
            .test_support()
            .gap(space(4.))
            .child(note)
            .children(link_block)
            .child(create)
            .children(failure.map(|text| {
                div()
                    .id("mls-invite-error")
                    .role(Role::Alert)
                    .test_support()
                    .text_size(text_secondary())
                    .text_color(danger)
                    .child(text)
            }))
            .children(list);
        crate::modal::Modal::new(
            "mls-invites",
            tr("group.invite.title"),
            &self.focus,
            move |_, cx| {
                this.update(cx, |panel, cx| panel.close(cx)).ok();
            },
        )
        .child(body)
        .render(cx)
    }
}
