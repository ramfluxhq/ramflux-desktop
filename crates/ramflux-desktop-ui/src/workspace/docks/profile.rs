// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The Profile panel (the former contacts dock, as the conversation header
//! offers it): about the peer of the open conversation only. It shows that
//! contact's row (safety state, Verify, Message, the actions menu) and, at
//! the bottom, a collapsed "Technical details" disclosure that holds the
//! evidence of the conversation (franking, group epoch, home node). Add
//! contact, the account's own card, requests, the auto-accept setting,
//! invitations and the blocked list live in the Contacts section.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, Entity, IntoElement, Render, Subscription, Window, div};
use ramflux_desktop_core::{ContextId, ContextKind, ParticipantId, selectors};
use ramflux_desktop_navigation::direction::{forward_icon, h_flex};
use ramflux_desktop_navigation::scale::space;

use super::contacts::ContactsDock;
use super::empty_line;
use super::evidence::EvidenceDock;
use crate::app::{StoreEvent, StoreHandle};
use crate::i18n::tr;

/// The Profile panel content.
pub struct ProfileDock {
    store: Entity<StoreHandle>,
    contacts: Entity<ContactsDock>,
    evidence: Entity<EvidenceDock>,
    context_id: Option<ContextId>,
    technical_open: bool,
    _subscriptions: Vec<Subscription>,
}

impl ProfileDock {
    /// A panel over `store`, drawing the peer through `contacts` and the
    /// technical details through `evidence`.
    pub fn new(
        store: Entity<StoreHandle>,
        contacts: Entity<ContactsDock>,
        evidence: Entity<EvidenceDock>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.subscribe(&store, |_, _, event: &StoreEvent, cx| {
            if let StoreEvent::Changed(changes) = event
                && changes.touches_docks()
            {
                cx.notify();
            }
        });
        // The contact's row follows its dock (a dialog, a state change).
        let followed = cx.observe(&contacts, |_, _, cx| cx.notify());
        Self {
            store,
            contacts,
            evidence,
            context_id: None,
            technical_open: false,
            _subscriptions: vec![subscription, followed],
        }
    }

    /// Shows the peer of `context_id`.
    pub fn set_context(&mut self, context_id: Option<ContextId>, cx: &mut Context<Self>) {
        if self.context_id != context_id {
            self.context_id = context_id;
            cx.notify();
        }
    }

    /// The peer of the open direct conversation: the participant who is not
    /// this account.
    fn peer(&self, cx: &Context<Self>) -> Option<ParticipantId> {
        let store = self.store.read(cx).store();
        let context = self.context_id.as_ref().and_then(|id| store.context(id))?;
        if context.kind != ContextKind::Direct {
            return None;
        }
        let me = selectors::self_participant(store);
        selectors::participants(store, &context.id)
            .into_iter()
            .find(|p| Some(&p.id) != me.as_ref())
            .map(|p| p.id.clone())
    }
}

impl Render for ProfileDock {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let peer = self.peer(cx);
        let row = peer.and_then(|peer| self.contacts.update(cx, |dock, cx| dock.peer_section(&peer, cx)));
        let open = self.technical_open;
        let body = v_flex().id("dock-profile").size_full().overflow_y_scroll();
        let Some(row) = row else {
            return body.child(empty_line("dock.profile.none", cx));
        };
        body.child(row).child(
            v_flex()
                .id("profile-technical")
                .test_support()
                .px(space(3.))
                .pt(space(3.))
                .gap(space(2.))
                .child(
                    h_flex().child(
                        Button::new("profile-technical-toggle")
                            .ghost()
                            .small()
                            .icon(if open {
                                gpui_kit::component::IconName::ChevronDown
                            } else {
                                forward_icon()
                            })
                            .label(tr("node.details"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.technical_open = !this.technical_open;
                                cx.notify();
                            })),
                    ),
                )
                .when(open, |column| column.child(div().child(self.evidence.clone()))),
        )
    }
}
