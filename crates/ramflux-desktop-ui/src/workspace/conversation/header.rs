// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The conversation header: title (a field while a group is renamed; in a
//! direct conversation, the peer's kind mark and name warning beside it,
//! see [`crate::identity`]), members, local settings (pin, mute,
//! disappearing timer behind a menu, the secondary entry besides the
//! command palette; Rename… for a group), and a banner with a Review
//! button while a member's identity change is not accepted (no dismiss: it
//! stays until the change is accepted).

use crate::style::DesignButton as _;
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, IntoElement, SharedString, div};
use ramflux_desktop_core::{
    Context, ContextKind, Participant, Scope, Store, UiEventKind, selectors,
};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::{design_px, space, text_secondary};

use crate::bubble::EventSink;
use crate::i18n::{tr, tr_args};
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

/// The header's trailing controls: while no dock tool is open, the tool
/// switcher (design H01).
pub struct Controls {
    /// The dock tool switcher, when the header carries it.
    pub tools: Option<AnyElement>,
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
    controls: Controls,
    cx: &App,
) -> AnyElement {
    let Controls { tools } = controls;
    let Some((context, sink)) = context.zip(sink) else {
        let palette = ramflux_desktop_navigation::palette(cx);
        return h_flex()
            .id("conversation-header")
            .test_support()
            .gap(space(3.))
            .px(gpui_kit::rems(32.0 / 15.0))
            .py(gpui_kit::rems(20.0 / 15.0))
            .min_h(gpui_kit::rems(88.0 / 15.0))
            .border_b_1()
            .border_color(palette.line)
            .child(
                div()
                    .flex_1()
                    .text_size(gpui_kit::rems(palette.size_subheading / 15.0))
                    .font_semibold()
                    .child(tr("conversation.none")),
            )
            .children(tools)
            .into_any_element();
    };
    let members = context.participants.len();
    let me = selectors::self_participant(store);
    let warnings: Vec<Participant> = selectors::participants(store, &context.id)
        .into_iter()
        .filter(|p| p.identity.is_changed() && Some(&p.id) != me.as_ref())
        .cloned()
        .collect();
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
    let palette = ramflux_desktop_navigation::palette(cx);
    let peer_agent =
        peer.as_ref().is_some_and(|p| p.kind == ramflux_desktop_core::ParticipantKind::Agent);
    let title = field.unwrap_or_else(|| {
        div()
            .id("conversation-title")
            .test_support()
            .text_size(gpui_kit::rems(palette.size_subheading / 15.0))
            .font_semibold()
            .text_color(palette.text)
            .child(selectors::context_title(store, context))
            .into_any_element()
    });
    let fact = |text: SharedString| {
        div()
            .text_size(gpui_kit::rems(palette.size_secondary / 15.0))
            .text_color(muted_color)
            .child(text)
    };
    let dot = || fact("·".into());
    // Design `RF / Chat header`: the title over one context line, then the
    // conversation's commands at the right; pinning lives in "More".
    // Board 04: an agent's private conversation reads "role · status"
    // where a group reads its member count.
    let agent_line = peer.as_ref().filter(|_| peer_agent).map(|p| {
        crate::i18n::join(
            &tr("sections.agents.role"),
            &crate::workspace::sections::agents::status(store, &p.id),
        )
    });
    // A direct chat names no member count; only a group does.
    let members = if agent_line.is_some() || !is_group { 0 } else { members };
    let context_line = h_flex()
        .gap(space(1.))
        .min_w_0()
        .children(agent_line.map(|line| fact(line).id("header-agent-status").test_support()))
        .when(members > 0, |line| {
            line.child(fact(tr_args("conversation.members", &[("count", &members.to_string())])))
        })
        .when(muted, |line| {
            line.when(members > 0, |line| line.child(dot()))
                .child(fact(tr("conversation.muted")).id("header-muted").test_support())
        })
        .when_some(timer, |line, secs| {
            line.when(members > 0 || muted, |line| line.child(dot())).child(
                fact(tr_args("conversation.timer", &[("timer", &timer_label(Some(secs)))]))
                    .id("header-timer")
                    .test_support(),
            )
        });
    let row = h_flex()
        .w_full()
        .gap(space(3.))
        .px(gpui_kit::rems(32.0 / 15.0))
        // Design `RF / Chat header`: 88 px, contents centred.
        .h(gpui_kit::rems(88.0 / 15.0))
        .items_center()
        // An agent's private conversation (design 04): its avatar leads
        // and "View profile" opens its management page.
        .when(peer_agent, |row| {
            row.child(
                div()
                    .flex_none()
                    .size(gpui_kit::rems(40.0 / 15.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(gpui_kit::rems(10.0 / 15.0))
                    .bg(palette.agent_av)
                    .text_color(palette.on_agent_av)
                    .child(ramflux_desktop_navigation::row::initial(&selectors::context_title(
                        store, context,
                    ))),
            )
        })
        .child(v_flex().flex_1().min_w_0().gap(gpui_kit::px(2.)).child(title).child(context_line))
        .when(peer_agent, |row| {
            row.child(
                crate::style::secondary_button("conversation-agent-profile", cx)
                    .label(tr("conversation.agent_profile"))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(crate::actions::OpenAgentsSettings), cx)
                    }),
            )
        })
        .when(context.kind == ContextKind::Direct, |row| {
            row.child(call_button(store, context, &sink, cx))
        })
        .children(tools)
        .child(
            Button::new("conversation-more")
                .ghost()
                .small()
                // `RF / Icon button`: 32 px square, the glyph 20 px in
                // `rf-secondary`.
                .size(design_px(32.0))
                .child(
                    Icon::new(IconName::Ellipsis)
                        .size(design_px(20.0))
                        .text_color(palette.secondary),
                )
                .accessibility_label(tr("conversation.more"))
                .tooltip(tr("conversation.more"))
                .dropdown_menu(move |menu, _, _| {
                    let mute = menu_sink.clone();
                    let pin = pin_sink.clone();
                    let menu = menu
                        .item(
                            PopupMenuItem::new(tr(if pinned {
                                "conversation.unpin"
                            } else {
                                "conversation.pin"
                            }))
                            .on_click(move |_, window, cx| {
                                pin.emit(UiEventKind::PinToggled { pinned: !pinned }, window, cx);
                            }),
                        )
                        .separator();
                    let menu = if is_group {
                        let start = start.clone();
                        let leave = menu_sink.clone();
                        menu.item(
                            PopupMenuItem::new(tr("group.rename.action"))
                                .on_click(move |_, window, cx| start(window, cx)),
                        )
                        .item(PopupMenuItem::new(tr("group.leave.action")).on_click(
                            move |_, window, cx| {
                                leave.emit(UiEventKind::LeaveGroupPressed, window, cx);
                            },
                        ))
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
                .gap(space(3.))
                .px(space(4.))
                .pb(space(2.))
                .children(kind)
                .children(warning)
        })
    });
    v_flex()
        .id("conversation-header")
        .test_support()
        .w_full()
        .border_b_1()
        .border_color(palette.line)
        .child(row)
        .children(identity)
        .children(warnings.into_iter().map(|participant| identity_banner(&participant, &sink, cx)))
        .into_any_element()
}

/// The call button of a direct conversation: disabled, with the reason as
/// its tooltip, while a call cannot be placed. To a contact whose identity
/// change is not accepted it opens the identity change sheet instead, and
/// says why in its tooltip.
fn call_button(store: &Store, context: &Context, sink: &EventSink, cx: &App) -> AnyElement {
    let blocked = crate::workspace::call_blocked(store, &context.id);
    let peer = ramflux_desktop_core::call_peer(store, &context.id);
    let changed = peer
        .as_ref()
        .and_then(|id| store.participant(id))
        .filter(|p| p.identity.is_changed())
        .map(|p| (p.id.clone(), p.display.clone()));
    let sink = sink.clone();
    let name = selectors::participants(store, &context.id)
        .into_iter()
        .find(|p| Some(&p.id) == peer.as_ref())
        .map_or_else(|| context.title.clone(), |p| p.display.clone());
    let gate_note =
        changed.as_ref().map(|(_, name)| tr_args("identity.changed.banner", &[("name", name)]));
    let _ = cx;
    Button::new("conversation-call")
        .ghost()
        .small()
        .icon(gpui_kit::assets::IconName::Phone)
        .label(tr("call.start"))
        .accessibility_label(tr_args("call.start_a11y", &[("name", &name)]))
        .disabled(blocked.is_some())
        .when_some(blocked.or(gate_note), |button, reason| button.tooltip(reason))
        .on_click(crate::identity::call_gate(changed.map(|(id, _)| id), sink))
        .into_any_element()
}

/// The banner of an identity change that waits for the user: the name, and
/// Review (primary), which opens the identity change sheet. It has no
/// dismiss: it stays until the change is accepted.
fn identity_banner(participant: &Participant, sink: &EventSink, cx: &App) -> AnyElement {
    let sink = sink.scoped(Scope::Panel("header".into()));
    let participant_id = participant.id.clone();
    let color = cx.theme().danger;
    let text = tr_args("identity.changed.banner", &[("name", &participant.display)]);
    h_flex()
        .id(domain_id("identity-banner", &participant.id))
        .aria_label(text.clone())
        .gap(space(2.))
        .px(space(4.))
        .py(space(2.))
        .bg(color.opacity(0.12))
        .text_size(text_secondary())
        .child(
            Icon::new(gpui_kit::assets::IconName::TriangleAlert)
                .small()
                .text_color(crate::tone::text(color, cx)),
        )
        .child(div().flex_1().min_w_0().child(text))
        .child(
            Button::new(domain_id("banner-review", &participant.id))
                .small()
                .design_primary(cx)
                .design_label(tr("identity.changed.review"))
                .on_click(move |_, window, cx| {
                    sink.emit(
                        UiEventKind::ReviewIdentityPressed {
                            participant_id: participant_id.clone(),
                        },
                        window,
                        cx,
                    );
                }),
        )
        .test_support()
        .into_any_element()
}
