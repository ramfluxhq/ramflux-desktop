// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The conversation header: title (a field while a group is renamed),
//! members, local settings (pin, mute, disappearing timer behind a menu, the
//! secondary entry besides the command palette; Rename… for a group), and a
//! sticky safety banner while any member's key change is unresolved.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
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
            .child(context.title.clone())
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
    v_flex()
        .id("conversation-header")
        .w_full()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(row)
        .children(warnings.into_iter().map(|participant| safety_banner(&participant, &sink, cx)))
        .into_any_element()
}

/// The sticky banner: stays until the participant is verified again.
fn safety_banner(participant: &Participant, sink: &EventSink, cx: &App) -> AnyElement {
    let sink = sink.scoped(Scope::Panel("header".into()));
    let participant_id = participant.id.clone();
    h_flex()
        .id(domain_id("safety-banner", &participant.id))
        .gap_2()
        .px_4()
        .py_2()
        .bg(cx.theme().danger.opacity(0.12))
        .text_sm()
        .child(Icon::new(IconName::TriangleAlert).small().text_color(cx.theme().danger))
        .child(div().flex_1().min_w_0().child(tr_args(
            "conversation.safety_banner",
            &[("name", &participant.display), ("state", &i18n::safety(participant.safety))],
        )))
        .child(
            Button::new(domain_id("banner-verify", &participant.id))
                .small()
                .outline()
                .label(tr("moment.safety.verify"))
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
