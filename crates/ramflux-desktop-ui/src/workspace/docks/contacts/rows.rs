// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Rows of the contacts dock: a contact request (from a moment or from an
//! incoming link), a contact with its identity state and actions, and a
//! blocked participant.
//!
//! Every action emits a UI event; the root turns it into an intent. Block
//! and Remove revoke what was shared with the contact and need the confirmation.
//! Message (on an accepted contact) is navigation: it shows the direct
//! conversation with the contact, or a draft of a new one; it is disabled,
//! with the reason, while the contact is not reachable yet.
//!
//! A contact row says how far the contact is trusted, as a status the
//! SDK reports: "Verified", nothing for an identity first seen on this
//! device, and a warning with Review while an identity change waits for
//! the user (the identity change sheet accepts it). It also says whether this
//! account can reach the contact, and what kind of party it is, with how
//! far that is proven (a
//! root-signed profile, or only the contact's own claim; see
//! [`crate::identity`]). A name that looks like another principal's, or
//! mixes scripts, carries a warning with the principal id beside it. The
//! row also says where the identity it holds came from: looked up by id
//! (provisional: anyone could have registered the name first), pinned by
//! the contact's card, or confirmed.

use crate::style::DesignButton as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, SharedString, Window, div};
use ramflux_desktop_core::{
    ContactAction, ContactLink, IdentityState, Moment, MomentKind, Participant, ParticipantId,
    RootSource, Scope, Store, UiEventKind, selectors,
};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{space, text_caption, text_secondary};

use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args};
use crate::identity;
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
            // A request the daemon lists is shown as such (see `requests`).
            .filter(|p| {
                !store
                    .contact_request(&p.id, ramflux_desktop_core::RequestDirection::Incoming)
                    .is_some_and(ramflux_desktop_core::ContactRequest::awaits_answer)
            })
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
        .gap(space(2.))
        .px(space(3.))
        .py(space(1.))
        .child(Icon::new(IconName::CircleUser).small().text_color(cx.theme().warning))
        .child(div().flex_1().min_w_0().text_size(text_secondary()).child(from.display.clone()))
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

/// A button handler.
pub type Click = Box<dyn Fn(&gpui_kit::ClickEvent, &mut Window, &mut App)>;

/// A contact with its identity state (a warning and Review while a change
/// waits), Message once accepted, and the actions menu.
pub fn contact_row(
    contact: &Participant,
    sink: &EventSink,
    on_message: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
    request: Option<super::requests::Outgoing>,
    cx: &App,
) -> AnyElement {
    let unreachable = (contact.reachable == Some(false))
        .then(|| tr_args("dock.contacts.unreachable_reason", &[("name", &contact.display)]));
    let message = (contact.link_state() == ContactLink::Accepted).then(|| {
        crate::gate::gated(
            Button::new("message")
                .xsmall()
                .design_secondary(cx)
                .design_label(tr("dock.contacts.message"))
                .accessibility_label(tr_args(
                    "dock.contacts.message_to",
                    &[("name", &contact.display)],
                ))
                .on_click(on_message),
            unreachable,
            cx,
        )
    });
    let color = match contact.identity {
        IdentityState::Verified => cx.theme().success,
        IdentityState::Tofu => cx.theme().muted_foreground,
        IdentityState::Changed => crate::tone::text(cx.theme().warning, cx),
    };
    // No badge for an identity first seen on this device.
    let status =
        (contact.identity != IdentityState::Tofu).then(|| i18n::identity_state(contact.identity));
    let review = contact.identity.is_changed().then(|| {
        let (sink, participant_id) = (sink.clone(), contact.id.clone());
        Button::new("contact-review")
            .xsmall()
            .design_secondary(cx)
            .design_label(tr("identity.changed.review"))
            .on_click(move |_, window, cx| {
                sink.emit(
                    UiEventKind::ReviewIdentityPressed { participant_id: participant_id.clone() },
                    window,
                    cx,
                );
            })
    });
    v_flex()
        .id(domain_id("contacts", &contact.id))
        .test_support()
        .px(space(3.))
        .py(space(1.))
        .gap(space(1.))
        .child(
            h_flex()
                .gap(space(2.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(text_secondary())
                        .font_medium()
                        .child(contact.display.clone()),
                )
                .children(review)
                .children(message)
                .child(actions_menu(contact, sink)),
        )
        // Advisory: a name that could pass for another's, with the id.
        .children(identity::name_warning("contact-name-warning", contact, cx))
        .child(
            h_flex()
                .flex_wrap()
                .gap_x(space(1.))
                .text_size(text_caption())
                .children(status.map(|status| {
                    div()
                        .id("contact-identity")
                        .role(gpui_kit::Role::Status)
                        .aria_label(status.clone())
                        .test_support()
                        .text_color(color)
                        .child(status)
                }))
                // The kind and how far it is proven, on the trust line (the
                // name keeps its room).
                .when_some(
                    identity::kind_mark("contact-kind", contact.kind_claim, cx),
                    |line, mark| {
                        line.child(div().text_color(cx.theme().muted_foreground).child("·"))
                            .child(mark)
                    },
                )
                .when_some(trust_line(contact), |line, trust| {
                    line.child(div().text_color(cx.theme().muted_foreground).child("·")).child(
                        div()
                            .id("contact-trust")
                            .aria_label(trust.clone())
                            .test_support()
                            .text_color(cx.theme().muted_foreground)
                            .child(trust),
                    )
                }),
        )
        // A named contact keeps its full id in sight, on its own line (the
        // name warning already carries it when there is one).
        .when(contact.named() && !contact.name_warnings.any(), |col| {
            col.child(identity::principal_id("contact-id", &contact.id, cx))
        })
        // This account's own request, while they have not accepted it:
        // quiet text, not a warning (waiting or declined is not a fault).
        .when_some(request, |col, request| {
            let send_again = request.send_again.then(|| {
                let sink = sink.clone();
                let participant_id = contact.id.clone();
                Button::new("contact-request-again")
                    .xsmall()
                    .ghost()
                    .label(tr("dock.contacts.request.send_again"))
                    .accessibility_label(tr_args(
                        "dock.contacts.request.send_again_for",
                        &[("name", &contact.display)],
                    ))
                    .on_click(move |_, window, cx| {
                        sink.emit(
                            UiEventKind::AddContactRequested {
                                participant_id: participant_id.clone(),
                            },
                            window,
                            cx,
                        );
                    })
            });
            col.child(
                h_flex()
                    .gap(space(2.))
                    .child(
                        div()
                            .id("contact-request")
                            .aria_label(request.note.clone())
                            .test_support()
                            .flex_1()
                            .min_w_0()
                            .text_size(text_caption())
                            .text_color(cx.theme().muted_foreground)
                            .child(request.note),
                    )
                    .children(send_again),
            )
        })
        .when_some(contact.root_source, |col, source| {
            let label = root_label(source);
            let color = if source.is_provisional() {
                cx.theme().warning
            } else {
                cx.theme().muted_foreground
            };
            col.child(
                div()
                    .id("contact-root")
                    .aria_label(label.clone())
                    .test_support()
                    .text_size(text_caption())
                    .text_color(color)
                    .child(label),
            )
        })
        .into_any_element()
}

/// Where the contact's identity came from, as the row says it.
pub fn root_label(source: RootSource) -> SharedString {
    tr(match source {
        RootSource::Lookup => "dock.contacts.root.lookup",
        RootSource::Card => "dock.contacts.root.card",
        RootSource::Verified => "dock.contacts.root.verified",
    })
}

/// Reachability, as far as the daemon reported it: "Reachable", "Not
/// reachable yet"; `None` when not known. The kind is the row's kind mark.
pub fn trust_line(contact: &Participant) -> Option<SharedString> {
    contact.reachable.map(|reachable| {
        tr(if reachable { "dock.contacts.reachable" } else { "dock.contacts.unreachable" })
    })
}

/// A blocked participant with Unblock and Remove.
pub fn blocked_row(contact: &Participant, sink: &EventSink, cx: &App) -> AnyElement {
    h_flex()
        .id(domain_id("contacts-blocked", &contact.id))
        .test_support()
        .gap(space(2.))
        .px(space(3.))
        .py(space(1.))
        .child(Icon::new(IconName::CircleX).small().text_color(cx.theme().muted_foreground))
        .child(div().flex_1().min_w_0().text_size(text_secondary()).child(contact.display.clone()))
        .child(
            Button::new("unblock")
                .xsmall()
                .design_secondary(cx)
                .design_label(action_label(ContactAction::Unblock))
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
