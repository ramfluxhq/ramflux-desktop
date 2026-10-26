// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Contact requests in the contacts dock (DESK-F21, on CONTACT-02):
//!
//! - a pending request from someone: who asks (with their kind as far as
//!   it is proven, and a warning on a name that could pass for another's,
//!   see [`crate::identity`]), their note, the safety
//!   number to compare before accepting, the messages they sent meanwhile
//!   (held until an answer: a "message request", with a preview of the
//!   first), and Accept, Decline and Block (Block drops anything they send
//!   from now on, and needs the hold);
//! - the account's setting: accept every request automatically;
//! - on a contact this account asked: that the request waits for them
//!   (with Send again, the explicit retry of the same request), that they
//!   declined it, or that it expired (Send again asks anew).
//!
//! Every answer emits a UI event; the root turns it into an intent. While
//! the daemon has no contact requests, the controls are disabled with the
//! reason. A list the daemon could not read just now (locked, an outage)
//! is said so; the requests already known stay answerable, and the next
//! poll reads them again.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, SharedString, div};
use ramflux_desktop_core::{
    ContactRequest, Participant, RequestAnswer, RequestPolicy, RequestState, Store, UiEventKind,
    methods, selectors,
};

use crate::bubble::EventSink;
use crate::i18n::{tr, tr_args};
use crate::ids::domain_id;

/// Why answering requests is unavailable, if it is.
pub fn reason(store: &Store) -> Option<SharedString> {
    crate::gate::reason(store, methods::CONTACT_REQUEST_ACCEPT)
}

/// A pending incoming request with its note, held messages, safety number
/// and the three answers.
pub fn pending_row(
    request: &ContactRequest,
    from: Option<&Participant>,
    sink: &EventSink,
    reason: Option<&SharedString>,
    cx: &App,
) -> AnyElement {
    let name: SharedString =
        from.map_or_else(|| request.peer.short(), |p| p.display.clone()).into();
    let muted = cx.theme().muted_foreground;
    let answer = |id: &'static str, answer: RequestAnswer| {
        let sink = sink.clone();
        let request_id = request.id.clone();
        let label = match answer {
            RequestAnswer::Accept => "dock.contacts.request.accept",
            RequestAnswer::Decline => "dock.contacts.request.decline",
            RequestAnswer::Block => "dock.contacts.request.block",
        };
        let button = Button::new(id)
            .xsmall()
            .label(tr(label))
            .accessibility_label(tr_args(&format!("{label}_for"), &[("name", &name)]))
            .on_click(move |_, window, cx| {
                sink.emit(
                    UiEventKind::ContactRequestAnswered { request_id: request_id.clone(), answer },
                    window,
                    cx,
                );
            });
        let button = match answer {
            RequestAnswer::Accept => button.primary(),
            RequestAnswer::Decline => button.outline(),
            RequestAnswer::Block => button.ghost(),
        };
        crate::gate::gated(button, reason.cloned())
    };
    v_flex()
        .id(domain_id("contacts-request", &request.peer))
        .test_support()
        .px_3()
        .py_1()
        .gap_1()
        .child(
            h_flex()
                .gap_2()
                .child(Icon::new(IconName::CircleUser).small().text_color(cx.theme().warning))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .font_medium()
                        .child(tr_args("dock.contacts.request.asks", &[("name", &name)])),
                )
                .children(
                    from.and_then(|p| crate::identity::kind_mark("request-kind", p.kind_claim, cx)),
                ),
        )
        .children(from.and_then(|p| crate::identity::name_warning("request-name-warning", p, cx)))
        // The daemon sends the note and the preview as delimited, sanitized
        // quoted fields ("…" with escapes); they are shown exactly as sent,
        // untrusted text, never quoted again.
        .when_some(request.note.clone(), |col, note| {
            let note = tr_args("dock.contacts.request.note", &[("note", &note)]);
            col.child(
                div()
                    .id("request-note")
                    .aria_label(note.clone())
                    .test_support()
                    .text_sm()
                    .child(note),
            )
        })
        .when(request.held_messages > 0, |col| {
            let count = request.held_messages.to_string();
            let one = request.held_messages == 1;
            let held = match (&request.preview, one) {
                (Some(preview), true) => {
                    tr_args("dock.contacts.request.held_one_preview", &[("preview", preview)])
                }
                (Some(preview), false) => tr_args(
                    "dock.contacts.request.held_preview",
                    &[("count", &count), ("preview", preview)],
                ),
                (None, true) => tr("dock.contacts.request.held_one"),
                (None, false) => tr_args("dock.contacts.request.held", &[("count", &count)]),
            };
            col.child(
                div()
                    .id("request-held")
                    .aria_label(held.clone())
                    .test_support()
                    .text_xs()
                    .text_color(muted)
                    .child(held),
            )
        })
        .child(
            div()
                .id("request-safety")
                .test_support()
                .text_xs()
                .font_family(cx.theme().mono_font_family.clone())
                .text_color(muted)
                .child(match &request.safety_number {
                    Some(number) => tr_args("dock.contacts.safety_number", &[("number", number)]),
                    None => tr("dock.contacts.request.no_safety_number"),
                }),
        )
        .child(
            h_flex()
                .gap_2()
                .justify_end()
                .child(answer("request-block", RequestAnswer::Block))
                .child(answer("request-decline", RequestAnswer::Decline))
                .child(answer("request-accept", RequestAnswer::Accept)),
        )
        .into_any_element()
}

/// Why the requests could not be read just now, with the ones already
/// known kept (and still answerable); the next poll tries again.
pub fn read_error(store: &Store, cx: &App) -> Option<AnyElement> {
    let message = store.contact_requests_read_error()?;
    let text = crate::i18n::backend_text(message);
    Some(
        div()
            .id("contacts-requests-error")
            .aria_label(text.clone())
            .test_support()
            .px_3()
            .pb_1()
            .text_xs()
            .text_color(cx.theme().warning)
            .child(text)
            .into_any_element(),
    )
}

/// The setting: accept every contact request automatically.
pub fn policy_switch(store: &Store, sink: &EventSink, cx: &App) -> AnyElement {
    let reason = crate::gate::reason(store, methods::CONTACT_REQUESTS_POLICY);
    let auto = store.request_policy() == Some(RequestPolicy::AutoAccept);
    let sink = sink.clone();
    v_flex()
        .px_3()
        .py_1()
        .gap_1()
        .child(
            Switch::new("contacts-auto-accept")
                .label(tr("dock.contacts.request.auto_accept"))
                .checked(auto)
                .disabled(reason.is_some() || store.request_policy().is_none())
                .on_change(move |checked, window, cx| {
                    sink.emit(
                        UiEventKind::ContactRequestPolicySwitched { auto_accept: *checked },
                        window,
                        cx,
                    );
                }),
        )
        .child(
            div()
                .id("contacts-auto-accept-help")
                .test_support()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(reason.unwrap_or_else(|| {
                    tr(if auto {
                        "dock.contacts.request.auto_accept_on_help"
                    } else {
                        "dock.contacts.request.auto_accept_help"
                    })
                })),
        )
        .into_any_element()
}

/// This account's own request to a contact, while they have not accepted
/// it: what the contact row says, and whether it offers to send it again.
pub struct Outgoing {
    /// "Waiting for …", "… declined your request", "… expired".
    pub note: SharedString,
    /// Send again (an explicit retry of the same request while it waits,
    /// or a new one after it expired). Never offered after a decline: the
    /// desktop does not ask again on its own, and the add form explains a
    /// request refused while the decline cools down.
    pub send_again: bool,
}

/// What a contact row says about this account's own request to `contact`,
/// while they have not accepted it.
pub fn outgoing(store: &Store, contact: &Participant) -> Option<Outgoing> {
    let request = selectors::outgoing_contact_request(store, &contact.id)?;
    let (key, send_again) = match request.state {
        RequestState::Pending => ("dock.contacts.request.waiting", true),
        RequestState::Declined => ("dock.contacts.request.declined", false),
        // A requester is not told about a block; shown as not accepted.
        RequestState::Blocked => ("dock.contacts.request.not_accepted", false),
        RequestState::Expired => ("dock.contacts.request.expired", true),
        RequestState::Accepted => return None,
    };
    Some(Outgoing { note: tr_args(key, &[("name", &contact.display)]), send_again })
}
