// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The entry page (P0): daemon, then node, then account.
//!
//! The page is driven only by the store: the step comes from
//! [`crate::route::page_for`], node and account details from the store. The
//! unlock form is the one place a secret is typed. The secret leaves the
//! input as a [`SecretInput`] through the [`UnlockHandler`] (never as a UI
//! event) and the input is cleared immediately.

mod node_card;

use std::rc::Rc;

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::stepper::{Stepper, StepperItem};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, IntoElement, Render, SharedString, Subscription, Window, div, rems,
};
use ramflux_desktop_core::{Account, AccountId, LockState, SecretInput, Store};

use crate::app::{StoreEvent, StoreHandle};
use crate::i18n::{tr, tr_args};
use crate::ids::domain_id;
use crate::route::{EntryStep, Page, page_for};

use node_card::NodeCard;

/// Receives an unlock request with its one-shot secret.
pub type UnlockHandler = Rc<dyn Fn(AccountId, SecretInput, &mut Window, &mut App)>;

/// The entry page view.
pub struct EntryPage {
    store: Entity<StoreHandle>,
    passphrase: Entity<InputState>,
    unlocking: Option<AccountId>,
    on_unlock: UnlockHandler,
    _subscriptions: Vec<Subscription>,
}

impl EntryPage {
    /// An entry page reading `store` and sending unlock requests to `on_unlock`.
    pub fn new(
        store: Entity<StoreHandle>,
        on_unlock: UnlockHandler,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let passphrase = cx.new(|cx| {
            InputState::new(window, cx).masked(true).placeholder(tr("entry.account.passphrase"))
        });
        let subscriptions = vec![
            cx.subscribe(&store, |this, store, event: &StoreEvent, cx| {
                if let StoreEvent::Changed(changes) = event
                    && changes.touches_status()
                {
                    let still_locked = this.unlocking.as_ref().is_some_and(|id| {
                        store
                            .read(cx)
                            .store()
                            .account(id)
                            .is_some_and(|a| a.lock_state == LockState::Locked)
                    });
                    if !still_locked {
                        this.unlocking = None;
                    }
                    cx.notify();
                }
            }),
            cx.subscribe_in(&passphrase, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.unlock(window, cx);
                }
            }),
        ];
        Self { store, passphrase, unlocking: None, on_unlock, _subscriptions: subscriptions }
    }

    /// The passphrase input.
    pub fn passphrase(&self) -> &Entity<InputState> {
        &self.passphrase
    }

    fn locked_account(store: &Store) -> Option<&Account> {
        store.accounts().find(|a| a.lock_state == LockState::Locked)
    }

    /// Sends the typed passphrase for the locked account and clears the input.
    pub fn unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(account) = Self::locked_account(self.store.read(cx).store()).map(|a| a.id.clone())
        else {
            return;
        };
        let secret = SecretInput::new(self.passphrase.read(cx).value().to_string());
        self.passphrase.update(cx, |input, cx| input.set_value("", window, cx));
        if secret.is_empty() {
            return;
        }
        self.unlocking = Some(account.clone());
        cx.notify();
        (self.on_unlock)(account, secret, window, cx);
    }

    fn render_daemon(&self, cx: &App) -> impl IntoElement {
        h_flex().id("entry-daemon").gap_3().child(Spinner::new()).child(
            v_flex().gap_1().child(div().text_sm().child(tr("entry.daemon.waiting"))).child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("entry.daemon.help")),
            ),
        )
    }

    fn render_node(&self, store: &Store, cx: &App) -> impl IntoElement {
        let nodes: Vec<_> = store.nodes().cloned().collect();
        v_flex()
            .id("entry-node")
            .gap_3()
            .when(nodes.is_empty(), |step| {
                step.child(
                    h_flex()
                        .gap_3()
                        .child(Spinner::new())
                        .child(div().text_sm().child(tr("entry.node.waiting"))),
                )
            })
            .children(nodes.into_iter().map(NodeCard::new))
            .when(store.nodes().next().is_some(), |step| {
                step.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("entry.node.joining")),
                )
            })
    }

    fn render_account(&self, store: &Store, cx: &mut Context<Self>) -> impl IntoElement {
        let accounts: Vec<Account> = store.accounts().cloned().collect();
        let locked = Self::locked_account(store).map(|a| a.id.clone());
        let unlocking = self.unlocking.is_some();
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let radius = cx.theme().radius;
        let rows = accounts.into_iter().map(move |account| {
            let state = tr(match account.lock_state {
                LockState::Locked => "entry.account.locked",
                LockState::Unlocked => "entry.account.unlocked",
            });
            let detail = tr_args(
                "entry.account.detail",
                &[
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
                .border_color(border)
                .rounded(radius)
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_sm().font_semibold().child(account.label.clone()))
                        .child(div().text_xs().text_color(muted).child(detail)),
                )
                .child(div().text_xs().text_color(muted).child(state))
        });
        v_flex()
            .id("entry-account")
            .gap_3()
            .children(rows)
            .when(locked.is_some(), |step| {
                step.child(
                    v_flex()
                        .gap_2()
                        .child(
                            Input::new(&self.passphrase)
                                .id("passphrase")
                                .aria_label(tr("entry.account.passphrase"))
                                .mask_toggle()
                                .disabled(unlocking),
                        )
                        .child(
                            h_flex().gap_2().child(
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
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.unlock(window, cx)),
                                    ),
                            ),
                        ),
                )
            })
            .child(
                v_flex()
                    .gap_2()
                    .pt_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("account-create")
                                    .outline()
                                    .small()
                                    .label(tr("entry.account.create"))
                                    .disabled(true),
                            )
                            .child(
                                Button::new("account-link")
                                    .outline()
                                    .small()
                                    .label(tr("entry.account.link"))
                                    .disabled(true),
                            )
                            .child(
                                Button::new("account-restore")
                                    .outline()
                                    .small()
                                    .label(tr("entry.account.restore"))
                                    .disabled(true),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(tr("entry.account.pending_contract")),
                    ),
            )
    }
}

impl Render for EntryPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx).store().clone();
        let step = match page_for(&store) {
            Page::Entry(step) => step,
            Page::Workspace => EntryStep::Account,
        };
        let selected = match step {
            EntryStep::Daemon => 0,
            EntryStep::Node => 1,
            EntryStep::Account => 2,
        };
        let content = match step {
            EntryStep::Daemon => self.render_daemon(cx).into_any_element(),
            EntryStep::Node => self.render_node(&store, cx).into_any_element(),
            EntryStep::Account => self.render_account(&store, cx).into_any_element(),
        };
        let title: SharedString = tr("entry.title");
        div().id("entry").size_full().overflow_y_scroll().bg(cx.theme().background).child(
            v_flex()
                .mx_auto()
                .w_full()
                .max_w(rems(36.))
                .gap_6()
                .px_6()
                .py_12()
                .child(
                    v_flex().gap_1().child(div().text_xl().font_semibold().child(title)).child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr("entry.subtitle")),
                    ),
                )
                .child(Stepper::new("entry-steps").selected_index(selected).items([
                    StepperItem::new().child(tr("entry.step.daemon")),
                    StepperItem::new().child(tr("entry.step.node")),
                    StepperItem::new().child(tr("entry.step.account")),
                ]))
                .child(content),
        )
    }
}
