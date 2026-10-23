// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The contacts dock: open contact requests (answer in place), then every
//! contact with its safety state, safety number and a Verify action while
//! unverified. Key changes stay visible here until verified.

use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Context, Entity, IntoElement, Render, Subscription, Window, div};
use ramflux_desktop_core::{
    Listing, Moment, MomentKind, Participant, SafetyState, Scope, Store, UiEventKind, selectors,
};

use super::section;
use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args};
use crate::ids::domain_id;

/// The contacts dock content.
pub struct ContactsDock {
    store: Entity<StoreHandle>,
    sink: EventSink,
    _subscription: Subscription,
}

impl ContactsDock {
    /// A dock over `store`, emitting through `sink` (scoped to the dock).
    pub fn new(store: Entity<StoreHandle>, sink: EventSink, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&store, |_, _, event: &StoreEvent, cx| {
            if let StoreEvent::Changed(changes) = event
                && changes.touches_docks()
            {
                cx.notify();
            }
        });
        Self { store, sink, _subscription: subscription }
    }
}

/// Open contact requests: `(moment, requester)`.
fn requests(store: &Store) -> Vec<(Moment, Participant)> {
    selectors::contact_requests(store)
        .into_iter()
        .filter_map(|m| match &m.kind {
            MomentKind::ContactRequest(p) => {
                store.participant(&p.from).map(|from| (m.clone(), from.clone()))
            }
            _ => None,
        })
        .collect()
}

fn request_row(moment: &Moment, from: &Participant, sink: &EventSink, cx: &App) -> AnyElement {
    let scoped = sink
        .scoped(Scope::Context(moment.context_id.clone()))
        .scoped(Scope::Moment(moment.id.clone()));
    let reject = scoped.clone();
    h_flex()
        .id(domain_id("contacts-request", &moment.id))
        .gap_2()
        .px_3()
        .py_1()
        .child(Icon::new(IconName::CircleUser).small().text_color(cx.theme().warning))
        .child(div().flex_1().min_w_0().text_sm().child(from.display.clone()))
        .child(
            Button::new("accept").xsmall().primary().label(tr("moment.contact.accept")).on_click(
                move |_, window, cx| {
                    scoped.emit(UiEventKind::ContactAnswered { accept: true }, window, cx)
                },
            ),
        )
        .child(
            Button::new("reject").xsmall().outline().label(tr("moment.contact.reject")).on_click(
                move |_, window, cx| {
                    reject.emit(UiEventKind::ContactAnswered { accept: false }, window, cx)
                },
            ),
        )
        .into_any_element()
}

fn contact_row(
    contact: &Participant,
    sink: &EventSink,
    verify_reason: Option<&gpui_kit::SharedString>,
    cx: &App,
) -> AnyElement {
    let color = match contact.safety {
        SafetyState::Verified => cx.theme().success,
        SafetyState::Unverified => cx.theme().muted_foreground,
        SafetyState::KeyChanged | SafetyState::Forked => cx.theme().danger,
    };
    let verify = (contact.safety != SafetyState::Verified).then(|| {
        let sink = sink.clone();
        let participant_id = contact.id.clone();
        crate::gate::gated(
            Button::new("verify").xsmall().outline().label(tr("moment.safety.verify")).on_click(
                move |_, window, cx| {
                    sink.emit(
                        UiEventKind::VerifySafetyPressed { participant_id: participant_id.clone() },
                        window,
                        cx,
                    );
                },
            ),
            verify_reason.cloned(),
        )
    });
    v_flex()
        .id(domain_id("contacts", &contact.id))
        .px_3()
        .py_1()
        .gap_1()
        .child(
            h_flex()
                .gap_2()
                .child(
                    div().flex_1().min_w_0().text_sm().font_medium().child(contact.display.clone()),
                )
                .child(div().text_xs().text_color(color).child(i18n::safety(contact.safety)))
                .children(verify),
        )
        .when_some(contact.safety_number.clone(), |col, number| {
            col.child(
                div()
                    .text_xs()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_color(cx.theme().muted_foreground)
                    .child(tr_args("dock.contacts.safety_number", &[("number", &number)])),
            )
        })
        .into_any_element()
}

impl Render for ContactsDock {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx).store();
        let verify_reason =
            crate::gate::reason(store, ramflux_desktop_core::methods::SAFETY_VERIFY);
        let requests = requests(store);
        let contacts: Vec<Participant> = selectors::contacts(store).into_iter().cloned().collect();
        v_flex()
            .id("dock-contacts")
            .size_full()
            .overflow_y_scroll()
            .when(!requests.is_empty(), |col| {
                col.child(section("dock.contacts.requests", cx)).children(
                    requests.iter().map(|(moment, from)| request_row(moment, from, &self.sink, cx)),
                )
            })
            .child(section("dock.contacts.list", cx))
            .children(crate::list_state::placeholder(
                "contacts-list-state",
                crate::list_state::of(store, Listing::Contacts, contacts.is_empty()),
                "dock.contacts.empty",
                cx,
            ))
            .children(
                contacts.iter().map(|c| contact_row(c, &self.sink, verify_reason.as_ref(), cx)),
            )
    }
}
