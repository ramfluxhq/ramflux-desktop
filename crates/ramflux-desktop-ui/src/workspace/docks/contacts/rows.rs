// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Rows of the contacts dock: a contact request (from a moment or from an
//! incoming link), a contact with its safety state and actions, and a
//! blocked participant.
//!
//! Every action emits a UI event; the root turns it into an intent. Block
//! and Remove revoke what was shared with the contact and need the hold.
//! Message (on an accepted contact) is navigation: it shows the direct
//! conversation with the contact, or a draft of a new one; it is disabled,
//! with the reason, while the contact is not reachable yet.
//!
//! A contact row says how far the contact is trusted: its safety state
//! (unverified until safety numbers are compared), whether this account can
//! reach it, and what kind of party it is, marked when that is only the
//! contact's own claim. It also says where the identity it holds came
//! from: looked up by id (provisional: anyone could have registered the
//! name first), pinned by the contact's card, or compared.
//!
//! Verify does not mark anything at once: it opens the safety number
//! comparison in the row, with the number and what to compare it with, and
//! only "The numbers match" marks the contact verified. Without a number
//! to compare there is nothing to confirm.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, SharedString, Window, div};
use ramflux_desktop_core::{
    ContactAction, ContactLink, Moment, MomentKind, Participant, ParticipantId, ParticipantKind,
    RootSource, SafetyState, Scope, Store, UiEventKind, selectors,
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

/// A button handler.
pub type Click = Box<dyn Fn(&gpui_kit::ClickEvent, &mut Window, &mut App)>;

/// The Verify action of a contact row and its comparison.
pub struct VerifyHandlers {
    /// Why verifying is unavailable, if it is.
    pub reason: Option<SharedString>,
    /// Whether the comparison is open in this row.
    pub open: bool,
    /// Verify pressed: open the comparison.
    pub start: Click,
    /// The numbers match: mark verified.
    pub confirm: Click,
    /// Cancel pressed.
    pub cancel: Click,
}

/// A contact with its safety state, safety number, Verify while unverified
/// (opening the comparison), Message once accepted, and the actions menu.
pub fn contact_row(
    contact: &Participant,
    sink: &EventSink,
    verify: VerifyHandlers,
    on_message: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let VerifyHandlers { reason: verify_reason, open, start, confirm, cancel } = verify;
    let unreachable = (contact.reachable == Some(false))
        .then(|| tr_args("dock.contacts.unreachable_reason", &[("name", &contact.display)]));
    let message = (contact.link_state() == ContactLink::Accepted).then(|| {
        crate::gate::gated(
            Button::new("message")
                .xsmall()
                .outline()
                .label(tr("dock.contacts.message"))
                .accessibility_label(tr_args(
                    "dock.contacts.message_to",
                    &[("name", &contact.display)],
                ))
                .on_click(on_message),
            unreachable,
        )
    });
    let color = match contact.safety {
        SafetyState::Verified => cx.theme().success,
        SafetyState::Unverified => cx.theme().muted_foreground,
        SafetyState::KeyChanged | SafetyState::Forked => cx.theme().danger,
    };
    let unverified = contact.safety != SafetyState::Verified;
    let verify = (unverified && !open).then(|| {
        crate::gate::gated(
            Button::new("verify")
                .xsmall()
                .outline()
                .label(tr("moment.safety.verify"))
                .accessibility_label(tr_args(
                    "dock.contacts.verify_for",
                    &[("name", &contact.display)],
                ))
                .on_click(start),
            verify_reason.clone(),
        )
    });
    let comparison = (unverified && open)
        .then(|| comparison(contact, confirm, cancel, verify_reason.as_ref(), cx));
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
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .font_medium()
                        .child(contact.display.clone()),
                )
                .children(verify)
                .children(message)
                .child(actions_menu(contact, sink)),
        )
        .child(
            h_flex()
                .flex_wrap()
                .gap_x_1()
                .text_xs()
                .child(
                    div()
                        .id("contact-safety")
                        .test_support()
                        .text_color(color)
                        .child(safety_label(contact.safety)),
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
                    .text_xs()
                    .text_color(color)
                    .child(label),
            )
        })
        .when_some(contact.safety_number.clone(), |col, number| {
            col.child(
                div()
                    .text_xs()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_color(cx.theme().muted_foreground)
                    .child(tr_args("dock.contacts.safety_number", &[("number", &number)])),
            )
        })
        .children(comparison)
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

/// The safety number comparison: what to compare, the number, and "The
/// numbers match" (only with a number to compare) or Cancel.
fn comparison(
    contact: &Participant,
    confirm: Click,
    cancel: Click,
    reason: Option<&SharedString>,
    cx: &App,
) -> AnyElement {
    let name = contact.display.as_str();
    let number = contact.safety_number.clone();
    let help = if number.is_some() {
        tr_args("dock.contacts.verify.compare", &[("name", name)])
    } else {
        tr_args("dock.contacts.verify.no_number", &[("name", name)])
    };
    let blocked = reason.cloned().or_else(|| {
        number.is_none().then(|| tr_args("dock.contacts.verify.no_number", &[("name", name)]))
    });
    v_flex()
        .id("contact-verify")
        .gap_1()
        .p_2()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .id("contact-verify-help")
                .role(gpui_kit::Role::Status)
                .aria_label(help.clone())
                .test_support()
                .text_xs()
                .child(help),
        )
        .when_some(number, |col, number| {
            col.child(
                div()
                    .id("contact-verify-number")
                    .test_support()
                    .text_sm()
                    .font_family(cx.theme().mono_font_family.clone())
                    .child(number),
            )
        })
        .child(
            h_flex()
                .gap_2()
                .child(crate::gate::gated(
                    Button::new("verify-confirm")
                        .xsmall()
                        .primary()
                        .label(tr("dock.contacts.verify.confirm"))
                        .on_click(confirm),
                    blocked,
                ))
                .child(
                    Button::new("verify-cancel")
                        .xsmall()
                        .ghost()
                        .label(tr("dock.contacts.verify.cancel"))
                        .on_click(cancel),
                ),
        )
        .into_any_element()
}

/// The safety state as a contact row says it: an unverified contact is
/// unverified until safety numbers are compared.
pub fn safety_label(safety: SafetyState) -> SharedString {
    match safety {
        SafetyState::Unverified => tr("dock.contacts.unverified"),
        other => i18n::safety(other),
    }
}

/// Reachability and kind, as far as the daemon reported them: "Reachable ·
/// Person", "Not reachable yet · AI agent (self-declared)"; `None` when
/// neither is known.
pub fn trust_line(contact: &Participant) -> Option<SharedString> {
    let reach = contact.reachable.map(|reachable| {
        tr(if reachable { "dock.contacts.reachable" } else { "dock.contacts.unreachable" })
    });
    let kind = match contact.kind {
        ParticipantKind::Human => Some("dock.contacts.kind.human"),
        ParticipantKind::Agent => Some("dock.contacts.kind.agent"),
        _ => None,
    }
    .map(|key| {
        if contact.kind_asserted {
            tr_args("dock.contacts.kind.asserted", &[("kind", &tr(key))])
        } else {
            tr(key)
        }
    });
    let parts: Vec<SharedString> = reach.into_iter().chain(kind).collect();
    (!parts.is_empty()).then(|| SharedString::from(parts.join(" · ")))
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
