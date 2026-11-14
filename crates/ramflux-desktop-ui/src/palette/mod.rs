// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The command palette (secondary-k): search contexts, contacts, objects and
//! history, and run actions. It replaces the context switch menu; the full
//! conversation list is reachable only here.
//!
//! The palette owns its query and selection (`CommandState`). Choosing an
//! entry that changes business state (pin, mute, disappearing timer) emits a
//! UI event through the root sink, scoped `[Workspace, Panel(palette),
//! Context]`; everything else is navigation, reported to the shell as a
//! [`PaletteEvent`].

mod entries;

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, Entity, EventEmitter, IntoElement, Render, Window, div, rems};
use ramflux_desktop_core::{AccountId, ContextId, Scope, UiEventKind};

use crate::actions::{ClosePalette, PALETTE_CONTEXT};
use crate::app::StoreHandle;
use crate::bubble::EventSink;
use crate::i18n::tr;

pub use entries::{Entry, Group, PaletteChoice};

/// Reported to the shell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteEvent {
    /// A navigation choice; the shell carries it out and closes the palette.
    Navigate(PaletteChoice),
    /// The palette should close (an intent was emitted, or it was cancelled).
    Close,
}

/// The palette view.
pub struct CommandPalette {
    store: Entity<StoreHandle>,
    sink: EventSink,
    state: Entity<CommandState>,
    groups: Vec<Group>,
    _locale: gpui_kit::Subscription,
    current: Option<ContextId>,
}

impl EventEmitter<PaletteEvent> for CommandPalette {}

impl CommandPalette {
    /// A palette over `store`; `sink` is the root sink.
    pub fn new(
        store: Entity<StoreHandle>,
        sink: EventSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = cx.new(|cx| CommandState::new(window, cx));
        // The entries are translated text built when the palette opens: a
        // language switch builds them again.
        let locale = crate::i18n::on_locale_switch(cx, |this, cx| {
            this.refresh(cx);
            cx.notify();
        });
        Self {
            store,
            sink: sink.scoped(Scope::Panel("palette".into())),
            state,
            groups: Vec::new(),
            current: None,
            _locale: locale,
        }
    }

    /// Rebuilds the entries for the shown context, clears the query and
    /// focuses the search field.
    pub fn open(
        &mut self,
        current: Option<ContextId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.current = current;
        self.refresh(cx);
        self.state.update(cx, |state, cx| state.set_query("", window, cx));
        self.state.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// Rebuilds the entries from the store.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let handle = self.store.read(cx);
        let accounts: Vec<(AccountId, String)> = handle
            .accounts()
            .filter_map(|id| handle.account_label(id).map(|label| (id.clone(), label)))
            .collect();
        self.groups = entries::groups(
            handle.store(),
            self.current.as_ref(),
            &accounts,
            handle.active_account(),
        );
    }

    /// The command state (query, selection), for tests.
    pub fn state(&self) -> &Entity<CommandState> {
        &self.state
    }

    /// Labels of the entries in group order.
    pub fn labels(&self) -> Vec<String> {
        self.groups.iter().flat_map(|g| g.entries.iter().map(|e| e.label.to_string())).collect()
    }

    /// Carries out the entry at (`section`, `row`).
    pub fn choose(
        &mut self,
        section: usize,
        row: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(choice) =
            self.groups.get(section).and_then(|g| g.entries.get(row)).map(|e| e.choice.clone())
        else {
            return;
        };
        let intent = |context: &ContextId, kind: UiEventKind| (context.clone(), kind);
        let business = match &choice {
            PaletteChoice::Pin(id, pinned) => {
                Some(intent(id, UiEventKind::PinToggled { pinned: *pinned }))
            }
            PaletteChoice::Mute(id, muted) => {
                Some(intent(id, UiEventKind::MuteToggled { muted: *muted }))
            }
            PaletteChoice::Disappearing(id, seconds) => {
                Some(intent(id, UiEventKind::DisappearingChosen { seconds: *seconds }))
            }
            _ => None,
        };
        match business {
            Some((context, kind)) => {
                cx.emit(PaletteEvent::Close);
                self.sink.scoped(Scope::Context(context)).emit(kind, window, cx);
            }
            None => cx.emit(PaletteEvent::Navigate(choice)),
        }
    }

    fn close(&mut self, _: &ClosePalette, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PaletteEvent::Close);
    }
}

impl Render for CommandPalette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        let cancel = cx.weak_entity();
        // Design B09: each result names the thing, what choosing it does,
        // and the verb as an outline chip (the whole row is the target).
        let selected = self.state.read(cx).selected_index().map(|path| (path.section, path.row));
        let groups = self.groups.iter().enumerate().map(|(section, group)| {
            let kind = group.label.clone();
            CommandGroup::new().items(group.entries.iter().enumerate().map(move |(row, e)| {
                let (label, kind) = (e.label.clone(), kind.clone());
                let verb = verb(&e.choice);
                let on = selected == Some((section, row));
                CommandItem::new().label(e.label.clone()).keywords(e.keywords.clone()).child(
                    move |_, cx| {
                        result_row(label.clone(), kind.clone(), verb, on, cx).into_any_element()
                    },
                )
            }))
        });
        let mut command = Command::new(&self.state);
        for group in groups {
            command = command.group(group);
        }
        let command = command
            .placeholder(tr("palette.placeholder"))
            .max_h(rems(24.))
            .bordered(false)
            .empty(|_, _, cx| {
                v_flex()
                    .items_center()
                    .gap_2()
                    .py_4()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(Icon::new(IconName::Search))
                    .child(tr("palette.empty"))
            })
            .on_confirm(move |path, window, cx| {
                this.update(cx, |palette, cx| palette.choose(path.section, path.row, window, cx))
                    .ok();
            })
            .on_cancel(move |_, cx| {
                cancel.update(cx, |_, cx| cx.emit(PaletteEvent::Close)).ok();
            })
            .footer(|_, _, cx| {
                let palette = ramflux_desktop_navigation::palette(cx);
                h_flex()
                    .pt_3()
                    .text_size(rems(palette.size_secondary / 15.0))
                    .text_color(palette.secondary)
                    .child(tr("palette.hint"))
            });
        let palette = ramflux_desktop_navigation::palette(cx);
        let close = cx.weak_entity();
        div()
            .id("palette-overlay")
            .key_context(PALETTE_CONTEXT)
            .on_action(cx.listener(Self::close))
            .absolute()
            .inset_0()
            .bg(palette.scrim)
            .flex()
            .justify_center()
            .items_center()
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(PaletteEvent::Close)),
            )
            .child(
                v_flex()
                    .id("palette")
                    .test_support()
                    .role(gpui_kit::Role::Dialog)
                    .aria_label(tr("palette.title"))
                    .w(rems(608.0 / 15.0))
                    .max_w_full()
                    .h_auto()
                    .gap_4()
                    .p(rems(24.0 / 15.0))
                    .bg(palette.overlay)
                    .border_1()
                    .border_color(palette.overlay_border)
                    .rounded(rems(palette.radius_dialog / 15.0))
                    .shadow_lg()
                    .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(rems(palette.size_subheading / 15.0))
                                    .font_semibold()
                                    .child(tr("palette.title")),
                            )
                            .child(
                                Button::new("palette-close")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Close)
                                    .accessibility_label(tr("common.close"))
                                    .on_click(move |_, _, cx| {
                                        close.update(cx, |_, cx| cx.emit(PaletteEvent::Close)).ok();
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .text_size(rems(palette.size_secondary / 15.0))
                            .font_medium()
                            .child(tr("palette.search")),
                    )
                    .child(command),
            )
    }
}

/// The verb a result row shows for what choosing it does.
fn verb(choice: &entries::PaletteChoice) -> &'static str {
    use entries::PaletteChoice as Choice;
    match choice {
        Choice::Context(_) | Choice::Moment(..) | Choice::Object(_) | Choice::Contact(_) => {
            "palette.verb.open"
        }
        _ => "palette.verb.go",
    }
}

/// One result: its name over what it is, and the verb as an outline chip.
fn result_row(
    label: gpui_kit::SharedString,
    kind: gpui_kit::SharedString,
    verb: &'static str,
    selected: bool,
    cx: &App,
) -> impl IntoElement {
    let palette = ramflux_desktop_navigation::palette(cx);
    // Design B09: rows on a rule; the chosen row's verb is the filled
    // (primary) button, the others outline.
    h_flex()
        .w_full()
        .gap_3()
        .py_3()
        .border_b_1()
        .border_color(palette.line)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(rems(palette.size_body / 15.0))
                        .font_medium()
                        .child(label),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(rems(palette.size_meta / 15.0))
                        .text_color(palette.secondary)
                        .child(kind),
                ),
        )
        .child(
            div()
                .flex_none()
                .px_2()
                .py(gpui_kit::px(2.))
                .rounded(rems(palette.radius_button / 15.0))
                .border_1()
                .border_color(if selected { palette.primary } else { palette.text })
                .when(selected, |chip| chip.bg(palette.primary).text_color(palette.on_primary))
                .text_size(rems(palette.size_button / 15.0))
                .child(tr(verb)),
        )
}
