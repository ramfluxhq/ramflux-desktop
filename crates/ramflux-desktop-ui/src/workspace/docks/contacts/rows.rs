// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Rows of the contacts dock: a contact request (from a moment or from an
//! incoming link), a contact with its safety state and actions, and a
//! blocked participant.
//!
//! Every action emits a UI event; the root turns it into an intent. Block
//! and Remove revoke what was shared with the contact and need the hold.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, SharedString, Window, div};
use ramflux_desktop_core::{
    ContactAction, Moment, MomentKind, Participant, ParticipantId, SafetyState, Scope, Store,
    UiEventKind, selectors,
};

use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args};
use crate::ids::domain_id;

/// An open contact request.
pub enum Request {
    /// A `ContactRequest` moment, answered in its context.
    Moment(Box<Moment>, Participant),
    /// An incoming contact link, answered on the participant.
    Link(Participant),
}

/// Open contact requests: moments first, then incoming links that no
/// moment presents.
pub fn requests(store: &Store) -> Vec<Request> {
    let mut list: Vec<Request> = selectors::contact_requests(store)
        .into_iter()
        .filter_map(|m| match &m.kind {
            MomentKind::ContactRequest(p) => store
                .participant(&p.from)
                .map(|from| Request::Moment(Box::new(m.clone()), from.clone())),
            _ => None,
        })
        .collect();
    let presented: Vec<ParticipantId> = list
        .iter()
        .filter_map(|r| match r {
            Request::Moment(_, from) => Some(from.id.clone()),
            Request::Link(_) => None,
        })
        .collect();
    list.extend(
        selectors::incoming_contact_requests(store)
            .into_iter()
            .filter(|p| !presented.contains(&p.id))
            .map(|p| Request::Link(p.clone())),
    );
    list
}

/// Emits `action` on `participant` through `sink`.
fn action_handler(
    sink: &EventSink,
    participant: &ParticipantId,
    action: ContactAction,
) -> impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static {
    let sink = sink.clone();
    let participant_id = participant.clone();
    move |_, window, cx| {
        sink.emit(
            UiEventKind::ContactActionPressed { participant_id: participant_id.clone(), action },
            window,
            cx,
        );
    }
}

/// Label of a contact action.
pub fn action_label(action: ContactAction) -> SharedString {
    tr(match action {
        ContactAction::Accept => "moment.contact.accept",
        ContactAction::Reject => "moment.contact.reject",
        ContactAction::Block => "dock.contacts.block",
        ContactAction::Unblock => "dock.contacts.unblock",
        ContactAction::Remove => "dock.contacts.remove",
    })
}

/// A request row with Accept and Reject.
pub fn request_row(request: &Request, sink: &EventSink, cx: &App) -> AnyElement {
    let (row_id, from, accept, reject): (_, &Participant, AnyElement, AnyElement) = match request {
        Request::Moment(moment, from) => {
            let scoped = sink
                .scoped(Scope::Context(moment.context_id.clone()))
                .scoped(Scope::Moment(moment.id.clone()));
            let reject = scoped.clone();
            (
                domain_id("contacts-request", &moment.id),
                from,
                answer_button("accept", true)
                    .on_click(move |_, window, cx| {
                        scoped.emit(UiEventKind::ContactAnswered { accept: true }, window, cx);
                    })
                    .into_any_element(),
                answer_button("reject", false)
                    .on_click(move |_, window, cx| {
                        reject.emit(UiEventKind::ContactAnswered { accept: false }, window, cx);
                    })
                    .into_any_element(),
            )
        }
        Request::Link(from) => (
            domain_id("contacts-incoming", &from.id),
            from,
            answer_button("accept", true)
                .on_click(action_handler(sink, &from.id, ContactAction::Accept))
                .into_any_element(),
            answer_button("reject", false)
                .on_click(action_handler(sink, &from.id, ContactAction::Reject))
                .into_any_element(),
        ),
    };
    h_flex()
        .id(row_id)
        .test_support()
        .gap_2()
        .px_3()
        .py_1()
        .child(Icon::new(IconName::CircleUser).small().text_color(cx.theme().warning))
        .child(div().flex_1().min_w_0().text_sm().child(from.display.clone()))
        .child(accept)
        .child(reject)
        .into_any_element()
}

fn answer_button(id: &'static str, accept: bool) -> Button {
    let button = Button::new(id).xsmall().label(tr(if accept {
        "moment.contact.accept"
    } else {
        "moment.contact.reject"
    }));
    if accept { button.primary() } else { button.outline() }
}

/// The actions menu of an accepted contact: Block and Remove.
fn actions_menu(contact: &Participant, sink: &EventSink) -> impl IntoElement + use<> {
    let menu_sink = sink.clone();
    let id = contact.id.clone();
    Button::new("contact-actions")
        .ghost()
        .xsmall()
        .icon(IconName::Ellipsis)
        .accessibility_label(tr_args("dock.contacts.actions_for", &[("name", &contact.display)]))
        .dropdown_menu(move |menu, _, _| {
            let item = |action: ContactAction| {
                PopupMenuItem::new(action_label(action))
                    .on_click(action_handler(&menu_sink, &id, action))
            };
            menu.item(item(ContactAction::Block)).separator().item(item(ContactAction::Remove))
        })
}

/// A contact with its safety state, safety number, Verify while unverified,
/// and the actions menu.
pub fn contact_row(
    contact: &Participant,
    sink: &EventSink,
    verify_reason: Option<&SharedString>,
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
        .test_support()
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
                .children(verify)
                .child(actions_menu(contact, sink)),
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

/// A blocked participant with Unblock and Remove.
pub fn blocked_row(contact: &Participant, sink: &EventSink, cx: &App) -> AnyElement {
    h_flex()
        .id(domain_id("contacts-blocked", &contact.id))
        .test_support()
        .gap_2()
        .px_3()
        .py_1()
        .child(Icon::new(IconName::CircleX).small().text_color(cx.theme().muted_foreground))
        .child(div().flex_1().min_w_0().text_sm().child(contact.display.clone()))
        .child(
            Button::new("unblock")
                .xsmall()
                .outline()
                .label(action_label(ContactAction::Unblock))
                .on_click(action_handler(sink, &contact.id, ContactAction::Unblock)),
        )
        .child(
            Button::new("remove")
                .xsmall()
                .ghost()
                .label(action_label(ContactAction::Remove))
                .on_click(action_handler(sink, &contact.id, ContactAction::Remove)),
        )
        .into_any_element()
}
