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

use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{Context, Entity, EventEmitter, IntoElement, Render, Window, div, px, rems};
use ramflux_desktop_core::{AccountId, ContextId, Scope, UiEventKind};

use crate::actions::{ClosePalette, PALETTE_CONTEXT};
use crate::app::StoreHandle;
use crate::bubble::EventSink;
use crate::i18n::{tr, tr_args};

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
        Self {
            store,
            sink: sink.scoped(Scope::Panel("palette".into())),
            state,
            groups: Vec::new(),
            current: None,
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
        let groups = self.groups.iter().map(|group| {
            CommandGroup::new().label(group.label.clone()).items(group.entries.iter().map(|e| {
                CommandItem::new()
                    .label(e.label.clone())
                    .icon(e.icon.clone())
                    .keywords(e.keywords.clone())
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
            .footer(|state, _, cx| {
                h_flex()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr_args(
                        "palette.matches",
                        &[("count", &state.matched_count().to_string())],
                    ))
                    .child(div().flex_1())
                    .child(tr("palette.hint"))
            });
        div()
            .id("palette-overlay")
            .key_context(PALETTE_CONTEXT)
            .on_action(cx.listener(Self::close))
            .absolute()
            .inset_0()
            .bg(cx.theme().background.opacity(0.4))
            .flex()
            .justify_center()
            .items_start()
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(PaletteEvent::Close)),
            )
            .child(
                div()
                    .id("palette")
                    .mt(rems(5.))
                    .w(px(560.))
                    .max_w_full()
                    .h_auto()
                    .bg(cx.theme().popover)
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius_lg)
                    .shadow_lg()
                    .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(command),
            )
    }
}
