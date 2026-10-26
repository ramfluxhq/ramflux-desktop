// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The conversation header: title (a field while a group is renamed; in a
//! direct conversation, the peer's kind mark and name warning beside it,
//! see [`crate::identity`]), members, local settings (pin, mute,
//! disappearing timer behind a menu, the secondary entry besides the
//! command palette; Rename… for a group), and a sticky safety banner while
//! any member's key change is unresolved or a verification went stale.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, IntoElement, SharedString, div};
use ramflux_desktop_core::{
    Context, ContextKind, Participant, Scope, Store, UiEventKind, selectors,
};

use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args};
use crate::ids::domain_id;

/// Disappearing timers offered in menus: off, one hour, one day, one week.
pub const TIMERS: [(Option<u32>, &str); 4] = [
    (None, "timer.off"),
    (Some(3_600), "timer.hour"),
    (Some(86_400), "timer.day"),
    (Some(604_800), "timer.week"),
];

/// Label of a timer value.
pub fn timer_label(seconds: Option<u32>) -> SharedString {
    TIMERS.iter().find(|(value, _)| *value == seconds).map_or_else(
        || tr_args("timer.seconds", &[("seconds", &seconds.unwrap_or(0).to_string())]),
        |(_, key)| tr(key),
    )
}

/// A command run with the window.
pub type Command = std::rc::Rc<dyn Fn(&mut gpui_kit::Window, &mut App)>;

/// The acknowledgement a call to a contact whose verification went stale
/// asks for (view-local): whether it is open here, and how to open and
/// close it.
pub struct CallAck {
    /// The acknowledgement is shown for this conversation.
    pub open: bool,
    /// Call pressed on a stale contact: show the acknowledgement.
    pub ask: Command,
    /// Verify again or Continue chosen: hide it.
    pub close: Command,
}

/// What renaming adds to the header: the inline field while a rename is
/// open, and the Rename… command.
pub struct Renaming {
    /// The field shown in place of the title.
    pub field: Option<AnyElement>,
    /// Rename… chosen.
    pub start: Command,
}

/// Renders the header for `context`, emitting through the context sink.
pub fn render(
    store: &Store,
    context: Option<&Context>,
    sink: Option<EventSink>,
    renaming: Renaming,
    call_ack: CallAck,
    cx: &App,
) -> AnyElement {
    let Some((context, sink)) = context.zip(sink) else {
        return h_flex()
            .id("conversation-header")
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().font_semibold().child(tr("conversation.none")))
            .into_any_element();
    };
    let members = context.participants.len();
    let warnings: Vec<Participant> = selectors::participants(store, &context.id)
        .into_iter()
        .filter(|p| p.safety.needs_attention())
        .cloned()
        .collect();
    let me = selectors::self_participant(store);
    let peer = (context.kind == ContextKind::Direct)
        .then(|| {
            selectors::participants(store, &context.id)
                .into_iter()
                .find(|p| Some(&p.id) != me.as_ref())
                .cloned()
        })
        .flatten();
    let pin_sink = sink.clone();
    let pinned = context.pinned;
    let muted = context.muted;
    let timer = context.disappearing_secs;
    let menu_sink = sink.clone();
    let muted_color = cx.theme().muted_foreground;
    let is_group = context.kind == ContextKind::Group;
    let Renaming { field, start } = renaming;
    let title = field.unwrap_or_else(|| {
        div()
            .id("conversation-title")
            .test_support()
            .font_semibold()
            .text_color(cx.theme().foreground)
            .child(selectors::context_title(store, context))
            .into_any_element()
    });
    let row = h_flex()
        .w_full()
        .gap_2()
        .px_4()
        .py_2()
        .child(title)
        .when(members > 0, |row| {
            row.child(
                div()
                    .text_xs()
                    .text_color(muted_color)
                    .child(tr_args("conversation.members", &[("count", &members.to_string())])),
            )
        })
        .when(muted, |row| {
            row.child(
                div()
                    .id("header-muted")
                    .text_xs()
                    .text_color(muted_color)
                    .child(tr("conversation.muted"))
                    .test_support(),
            )
        })
        .when_some(timer, |row, secs| {
            row.child(
                div()
                    .id("header-timer")
                    .text_xs()
                    .text_color(muted_color)
                    .child(tr_args("conversation.timer", &[("timer", &timer_label(Some(secs)))]))
                    .test_support(),
            )
        })
        .child(div().flex_1())
        .when(context.kind == ContextKind::Direct, |row| {
            row.child(call_button(store, context, &sink, &call_ack))
        })
        .child(
            Button::new("conversation-pin")
                .ghost()
                .small()
                .icon(if pinned { IconName::StarFill } else { IconName::Star })
                .label(tr(if pinned { "conversation.unpin" } else { "conversation.pin" }))
                .on_click(move |_, window, cx| {
                    pin_sink.emit(UiEventKind::PinToggled { pinned: !pinned }, window, cx);
                }),
        )
        .child(
            Button::new("conversation-more")
                .ghost()
                .small()
                .icon(IconName::Ellipsis)
                .accessibility_label(tr("conversation.more"))
                .tooltip(tr("conversation.more"))
                .dropdown_menu(move |menu, _, _| {
                    let mute = menu_sink.clone();
                    let menu = if is_group {
                        let start = start.clone();
                        menu.item(
                            PopupMenuItem::new(tr("group.rename.action"))
                                .on_click(move |_, window, cx| start(window, cx)),
                        )
                        .separator()
                    } else {
                        menu
                    };
                    let mut menu = menu.item(
                        PopupMenuItem::new(tr(if muted {
                            "conversation.unmute"
                        } else {
                            "conversation.mute"
                        }))
                        .on_click(move |_, window, cx| {
                            mute.emit(UiEventKind::MuteToggled { muted: !muted }, window, cx);
                        }),
                    );
                    menu = menu
                        .separator()
                        .item(PopupMenuItem::label(tr("conversation.disappearing")));
                    for (seconds, key) in TIMERS {
                        let sink = menu_sink.clone();
                        menu = menu.item(
                            PopupMenuItem::new(tr(key)).checked(timer == seconds).on_click(
                                move |_, window, cx| {
                                    sink.emit(
                                        UiEventKind::DisappearingChosen { seconds },
                                        window,
                                        cx,
                                    );
                                },
                            ),
                        );
                    }
                    menu
                }),
        );
    // Who the peer of a direct conversation is: the kind as far as it is
    // proven, and a warning on a name that could pass for another's. Its
    // own line under the title, so it never fights with the controls.
    let identity = peer.as_ref().and_then(|peer| {
        let kind = crate::identity::kind_mark("header-kind", peer.kind_claim, cx);
        let warning = crate::identity::name_warning("header-name-warning", peer, cx);
        (kind.is_some() || warning.is_some()).then(|| {
            h_flex()
                .id("header-identity")
                .w_full()
                .flex_wrap()
                .gap_3()
                .px_4()
                .pb_2()
                .children(kind)
                .children(warning)
        })
    });
    v_flex()
        .id("conversation-header")
        .w_full()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(row)
        .children(identity)
        .children(warnings.into_iter().map(|participant| safety_banner(&participant, &sink, cx)))
        .children(call_ack_strip(store, context, &sink, &call_ack, cx))
        .into_any_element()
}

/// The peer of a direct conversation whose verification went stale, if so.
fn stale_peer<'a>(store: &'a Store, context: &Context) -> Option<&'a Participant> {
    let peer = ramflux_desktop_core::call_peer(store, &context.id)?;
    store.participant(&peer).filter(|p| p.safety.is_stale())
}

/// The acknowledgement before a call to a contact whose verification went
/// stale: Verify again, or Continue (places the call).
fn call_ack_strip(
    store: &Store,
    context: &Context,
    sink: &EventSink,
    ack: &CallAck,
    cx: &App,
) -> Option<AnyElement> {
    if !ack.open {
        return None;
    }
    let peer = stale_peer(store, context)?;
    let prompt = tr_args("identity.ack.call", &[("name", &peer.display)]);
    let (verify_sink, call_sink) = (sink.scoped(Scope::Panel("header".into())), sink.clone());
    let (close_verify, close_call) = (ack.close.clone(), ack.close.clone());
    let participant_id = peer.id.clone();
    Some(
        div()
            .px_4()
            .pb_2()
            .child(crate::identity::stale_ack(
                "call-stale-ack",
                prompt,
                Box::new(move |_, window, cx| {
                    close_verify(window, cx);
                    verify_sink.emit(
                        UiEventKind::VerifySafetyPressed { participant_id: participant_id.clone() },
                        window,
                        cx,
                    );
                }),
                Box::new(move |_, window, cx| {
                    close_call(window, cx);
                    call_sink.emit(UiEventKind::CallStartRequested, window, cx);
                }),
                cx,
            ))
            .into_any_element(),
    )
}

/// The call button of a direct conversation: disabled, with the reason as
/// its tooltip, while a call cannot be placed. To a contact whose
/// verification went stale it first asks for the acknowledgement.
fn call_button(store: &Store, context: &Context, sink: &EventSink, ack: &CallAck) -> AnyElement {
    let blocked = crate::workspace::call_blocked(store, &context.id);
    let stale = stale_peer(store, context).is_some();
    let ask = ack.ask.clone();
    let sink = sink.clone();
    let name = selectors::participants(store, &context.id)
        .into_iter()
        .find(|p| Some(&p.id) == ramflux_desktop_core::call_peer(store, &context.id).as_ref())
        .map_or_else(|| context.title.clone(), |p| p.display.clone());
    Button::new("conversation-call")
        .ghost()
        .small()
        .icon(gpui_kit::assets::IconName::Phone)
        .label(tr("call.start"))
        .accessibility_label(tr_args("call.start_a11y", &[("name", &name)]))
        .disabled(blocked.is_some())
        .when_some(blocked, |button, reason| button.tooltip(reason))
        .on_click(crate::identity::call_gate(stale, Some(ask), sink))
        .into_any_element()
}

/// The sticky banner: stays until the participant is verified again. A key
/// change or fork is a danger; a stale verification a warning.
fn safety_banner(participant: &Participant, sink: &EventSink, cx: &App) -> AnyElement {
    let sink = sink.scoped(Scope::Panel("header".into()));
    let participant_id = participant.id.clone();
    let (color, icon) = if participant.safety.is_stale() {
        (cx.theme().warning, gpui_kit::assets::IconName::ShieldAlert)
    } else {
        (cx.theme().danger, gpui_kit::assets::IconName::TriangleAlert)
    };
    let text = if participant.safety.is_stale() {
        tr_args("identity.safety.stale_banner", &[("name", &participant.display)])
    } else {
        tr_args(
            "conversation.safety_banner",
            &[("name", &participant.display), ("state", &i18n::safety(participant.safety))],
        )
    };
    h_flex()
        .id(domain_id("safety-banner", &participant.id))
        .aria_label(text.clone())
        .gap_2()
        .px_4()
        .py_2()
        .bg(color.opacity(0.12))
        .text_sm()
        .child(Icon::new(icon).small().text_color(crate::tone::text(color, cx)))
        .child(div().flex_1().min_w_0().child(text))
        .child(
            Button::new(domain_id("banner-verify", &participant.id))
                .small()
                .outline()
                .label(crate::identity::verify_label(participant.safety))
                .on_click(move |_, window, cx| {
                    sink.emit(
                        UiEventKind::VerifySafetyPressed { participant_id: participant_id.clone() },
                        window,
                        cx,
                    );
                }),
        )
        .test_support()
        .into_any_element()
}
