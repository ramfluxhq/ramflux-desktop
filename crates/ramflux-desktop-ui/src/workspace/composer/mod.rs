// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The composer: the primary input of the conversation field.
//!
//! Two modes: a message ([`UiEventKind::SubmitMessage`], becoming
//! `SendMessage`) or a one-sentence request to an AI participant of the
//! conversation ([`UiEventKind::TaskRequested`], becoming `StartTask`). A
//! request is addressed with a leading mention (`@Orbit …`), else to the
//! participant picked in the command palette, else to the conversation's
//! first AI participant ([`assignee`]); a conversation without one offers
//! no request mode. Both go through the context-scoped sink; the composer
//! never sends anything itself. The mode and the pick are view-local.
//!
//! In a new-conversation draft the composer writes the first message: it
//! submits [`UiEventKind::StartConversationRequested`] for the draft's
//! contact and id through the draft's sink, and keeps the text until the
//! conversation exists (a failed start can be sent again unchanged).
//!
//! The attach button opens the native file picker; the chosen file is
//! checked and staged ([`attachment`]) and sent with Send, before the text.
//!
//! - [`attachment`]: the staged file and its chip.
//! - [`assignee`]: who a request is addressed to.

pub mod assignee;
pub mod attachment;
mod drafts;

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, Entity, IntoElement, Render, SharedString, Subscription, Window, div,
};
use ramflux_desktop_core::{ContextId, ParticipantId, ParticipantKind, UiEventKind, selectors};

use assignee::Agent;

use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::file_dialog;
use crate::i18n::{tr, tr_args};
use attachment::Staged;

/// What the composer submits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerMode {
    /// A message to the context.
    Message,
    /// A one-sentence request that starts a task.
    Task,
}

/// The new conversation the composer writes the first message of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftTarget {
    /// The conversation's id.
    pub context_id: ContextId,
    /// The contact.
    pub participant_id: ParticipantId,
}

/// The composer view.
pub struct Composer {
    drafts: drafts::Drafts,
    store: Entity<StoreHandle>,
    input: Entity<TextareaState>,
    sink: Option<EventSink>,
    context: Option<ContextId>,
    picked: Option<ParticipantId>,
    draft: Option<DraftTarget>,
    mode: ComposerMode,
    unavailable: Unavailable,
    staged: Option<Staged>,
    _subscriptions: Vec<Subscription>,
}

/// Why a mode cannot submit on the attached backend (translated reasons).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Unavailable {
    /// Why messages cannot be sent here.
    pub message: Option<SharedString>,
    /// Why tasks cannot be started here.
    pub task: Option<SharedString>,
    /// Why files cannot be attached here.
    pub attach: Option<SharedString>,
}

impl Composer {
    /// A composer with no target context, following `store` for the
    /// outcome of a sent attachment.
    pub fn new(store: Entity<StoreHandle>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 6)
                .submit_on_enter(true)
                .placeholder(tr("composer.placeholder"))
        });
        let subscriptions = vec![
            cx.subscribe_in(&input, window, |this, _, event, window, cx| {
                // The send button turns primary once there is text.
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
                if matches!(event, InputEvent::PressEnter { secondary: false, shift: false }) {
                    let composing = this.input.update(cx, |input, cx| {
                        let marked =
                            gpui_kit::EntityInputHandler::marked_text_range(input, window, cx)
                                .is_some();
                        if marked {
                            gpui_kit::EntityInputHandler::unmark_text(input, window, cx);
                        }
                        marked
                    });
                    if !composing {
                        this.submit(window, cx);
                    }
                }
            }),
            cx.subscribe_in(&store, window, |this, store, event: &StoreEvent, window, cx| {
                if let StoreEvent::Changed(changes) = event
                    && (changes.is_reset()
                        || !ramflux_desktop_core::projection::conversations::is_visible(
                            store.read(cx).store(),
                        ))
                {
                    this.drafts.clear();
                    this.input.update(cx, |input, cx| input.set_value("", window, cx));
                    this.staged = None;
                }
                if matches!(event, StoreEvent::Changed(_)) {
                    let source = store.read(cx).store();
                    this.drafts.prune(source, this.draft.as_ref().map(|draft| &draft.context_id));
                    if this.context.as_ref().is_some_and(|id| source.context(id).is_none()) {
                        this.input.update(cx, |input, cx| input.set_value("", window, cx));
                    }
                    let next = this.staged.as_ref().and_then(|s| s.observe(store.read(cx).store()));
                    if let Some(next) = next {
                        this.staged = next;
                        cx.notify();
                    }
                }
            }),
            crate::i18n::observe_locale(window, cx, |this, window, cx| {
                this.set_placeholder(window, cx);
            }),
        ];
        Self {
            drafts: drafts::Drafts::default(),
            store,
            input,
            sink: None,
            context: None,
            picked: None,
            draft: None,
            mode: ComposerMode::Message,
            unavailable: Unavailable::default(),
            staged: None,
            _subscriptions: subscriptions,
        }
    }

    /// Targets a context through its scoped sink, or nothing. A new
    /// context starts in message mode, with no staged file, no draft and
    /// no picked AI participant.
    pub fn set_sink(
        &mut self,
        context: Option<ContextId>,
        sink: Option<EventSink>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.context != context || self.draft.is_some() {
            self.save_draft(cx);
            let text =
                context.as_ref().and_then(|id| self.drafts.get(id)).cloned().unwrap_or_default();
            self.input.update(cx, |input, cx| input.set_value(text, window, cx));
        }
        self.sink = sink;
        self.context = context;
        self.picked = None;
        self.draft = None;
        self.staged = None;
        self.set_mode(ComposerMode::Message, window, cx);
        self.set_placeholder(window, cx);
        cx.notify();
    }

    /// Writes the first message of a new conversation through the draft's
    /// `sink`.
    pub fn set_draft(
        &mut self,
        draft: DraftTarget,
        sink: EventSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = self.draft.as_ref().map(|draft| &draft.context_id).or(self.context.as_ref());
        if previous != Some(&draft.context_id) {
            self.save_draft(cx);
            let text = self.drafts.get(&draft.context_id).cloned().unwrap_or_default();
            self.input.update(cx, |input, cx| input.set_value(text, window, cx));
        }
        self.sink = Some(sink);
        self.context = None;
        self.picked = None;
        self.draft = Some(draft);
        self.staged = None;
        self.set_mode(ComposerMode::Message, window, cx);
        self.set_placeholder(window, cx);
        cx.notify();
    }

    fn save_draft(&mut self, cx: &App) {
        if let Some(id) =
            self.draft.as_ref().map(|draft| &draft.context_id).or(self.context.as_ref())
        {
            if self.draft.is_none() && self.store.read(cx).store().context(id).is_none() {
                self.drafts.remove(id);
                return;
            }
            let text = self.input.read(cx).value().to_string();
            if text.is_empty() {
                self.drafts.remove(id);
            } else {
                self.drafts.insert(id.clone(), text);
            }
        }
    }

    /// The draft the composer writes for, if any.
    pub fn draft(&self) -> Option<&DraftTarget> {
        self.draft.as_ref()
    }

    /// Records which modes the backend cannot perform; their controls are
    /// shown disabled with the reason.
    pub fn set_unavailable(&mut self, unavailable: Unavailable, cx: &mut Context<Self>) {
        if self.unavailable != unavailable {
            self.unavailable = unavailable;
            cx.notify();
        }
    }

    /// Why the current mode cannot submit, if it cannot.
    pub fn unavailable_reason(&self) -> Option<&SharedString> {
        match self.mode {
            ComposerMode::Message => self.unavailable.message.as_ref(),
            ComposerMode::Task => self.unavailable.task.as_ref(),
        }
    }

    /// The text input, for focus and tests.
    pub fn input(&self) -> &Entity<TextareaState> {
        &self.input
    }

    /// The current mode.
    pub fn mode(&self) -> ComposerMode {
        self.mode
    }

    /// The staged file, for tests.
    pub fn staged(&self) -> Option<&Staged> {
        self.staged.as_ref()
    }

    /// Clears the text.
    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) =
            self.draft.as_ref().map(|draft| &draft.context_id).or(self.context.as_ref())
        {
            self.drafts.remove(id);
        }
        self.input.update(cx, |input, cx| input.set_value("", window, cx));
    }

    /// Switches between message and task mode (a draft is message only,
    /// and a conversation without an AI participant has nobody to ask).
    pub fn set_mode(&mut self, mode: ComposerMode, window: &mut Window, cx: &mut Context<Self>) {
        let nobody = mode == ComposerMode::Task && self.agents(cx).is_empty();
        if self.mode == mode || (self.draft.is_some() && mode == ComposerMode::Task) || nobody {
            return;
        }
        self.mode = mode;
        self.set_placeholder(window, cx);
        cx.notify();
    }

    /// Addresses requests to `agent` (from the command palette) and
    /// switches to request mode.
    pub fn ask(&mut self, agent: ParticipantId, window: &mut Window, cx: &mut Context<Self>) {
        self.picked = Some(agent);
        self.set_mode(ComposerMode::Task, window, cx);
        self.set_placeholder(window, cx);
        self.focus(window, cx);
        cx.notify();
    }

    /// The AI participants of the shown conversation.
    pub fn agents(&self, cx: &App) -> Vec<Agent> {
        self.context
            .as_ref()
            .map(|c| assignee::agents(self.store.read(cx).store(), c))
            .unwrap_or_default()
    }

    /// Why there is nobody to ask: a member whose kind nothing reported
    /// (added by id: only a contact card says what kind of party it is)
    /// may be an AI participant, but a task is never addressed on a guess.
    pub fn no_agent_reason(&self, cx: &App) -> SharedString {
        let store = self.store.read(cx).store();
        let me = selectors::self_participant(store);
        let unknown = self.context.as_ref().and_then(|c| {
            selectors::participants(store, c)
                .into_iter()
                .find(|p| Some(&p.id) != me.as_ref() && p.kind == ParticipantKind::Unknown)
        });
        match unknown {
            Some(p) => tr_args("composer.kind_unknown", &[("name", &p.display)]),
            None => tr("composer.no_agent"),
        }
    }

    /// Whether a member's kind is unknown (it may be an agent, so the
    /// receiver line stays to say why a task cannot be asked of it).
    fn has_unknown_member(&self, cx: &App) -> bool {
        let store = self.store.read(cx).store();
        let me = selectors::self_participant(store);
        self.context.as_ref().is_some_and(|c| {
            selectors::participants(store, c)
                .into_iter()
                .any(|p| Some(&p.id) != me.as_ref() && p.kind == ParticipantKind::Unknown)
        })
    }

    /// The AI participant a request without a mention goes to.
    pub fn addressee(&self, cx: &App) -> Option<Agent> {
        let agents = self.agents(cx);
        assignee::resolve(&agents, self.picked.as_ref(), "").map(|(a, _)| a.clone())
    }

    fn set_placeholder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = match (self.mode, &self.draft) {
            (ComposerMode::Task, _) => match self.addressee(cx) {
                Some(agent) => tr_args("composer.ask_placeholder", &[("name", &agent.name)]),
                None => tr("composer.task_placeholder"),
            },
            (ComposerMode::Message, Some(_)) => tr("composer.first_placeholder"),
            (ComposerMode::Message, None) => tr("composer.placeholder"),
        };
        self.input.update(cx, |input, cx| input.set_placeholder(placeholder, window, cx));
    }

    /// Opens the native file picker; the chosen file is staged.
    pub fn pick_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        file_dialog::open_file(
            tr("composer.attach.prompt"),
            window,
            cx,
            move |path, window, cx| {
                this.update(cx, |composer, cx| composer.attach_chosen(&path, window, cx)).ok();
            },
        );
    }

    /// Stages the file the picker returned (checked against the frame
    /// budget at once).
    pub fn attach_chosen(&mut self, path: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.staged = Some(Staged::check(path));
        cx.notify();
    }

    /// Removes the staged file without sending it.
    pub fn remove_staged(&mut self, cx: &mut Context<Self>) {
        if self.staged.take().is_some() {
            cx.notify();
        }
    }

    /// Emits the staged file (if ready), then the current text as a
    /// message or a task request, and clears the input. In a draft, emits
    /// the first message and keeps the text.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sink) = self.sink.clone() else {
            return;
        };
        if self.unavailable_reason().is_some() {
            return;
        }
        let text = self.input.read(cx).value().trim().to_owned();
        if let Some(draft) = self.draft.clone() {
            if !text.is_empty() {
                sink.emit(
                    UiEventKind::StartConversationRequested {
                        context_id: draft.context_id,
                        participant_id: draft.participant_id,
                        text,
                    },
                    window,
                    cx,
                );
            }
            return;
        }
        let file = self.staged.as_ref().and_then(Staged::ready_path).map(str::to_owned);
        if let Some(path) = file.filter(|_| self.mode == ComposerMode::Message) {
            if self.unavailable.attach.is_some() {
                return;
            }
            let sent = self.staged.as_ref().and_then(|s| s.sent(self.store.read(cx).store()));
            self.staged = sent;
            sink.emit(UiEventKind::AttachFile { path }, window, cx);
            cx.notify();
        }
        if text.is_empty() {
            return;
        }
        let kind = match self.mode {
            ComposerMode::Message => UiEventKind::SubmitMessage { text },
            ComposerMode::Task => {
                let agents = self.agents(cx);
                let Some((agent, text)) = assignee::resolve(&agents, self.picked.as_ref(), &text)
                else {
                    return;
                };
                if text.is_empty() {
                    return;
                }
                UiEventKind::TaskRequested { assignee: agent.id.clone(), text }
            }
        };
        if let Some(id) = self.context.as_ref() {
            self.drafts.remove(id);
        }
        self.input.update(cx, |input, cx| input.set_value("", window, cx));
        sink.emit(kind, window, cx);
    }
}

impl Render for Composer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = self.sink.is_some();
        let reason = self.unavailable_reason().cloned();
        let can_send = enabled && reason.is_none();
        let nobody = enabled && self.draft.is_none() && self.agents(cx).is_empty();
        let task_reason =
            self.unavailable.task.clone().or_else(|| nobody.then(|| self.no_agent_reason(cx)));
        let in_draft = self.draft.is_some();
        let attach_reason = if in_draft {
            Some(tr("composer.attach.after_start"))
        } else {
            self.unavailable.attach.clone()
        };
        let chip = self.staged.as_ref().map(|staged| {
            attachment::chip(staged, cx.listener(|this, _, _, cx| this.remove_staged(cx)), cx)
        });
        let palette = ramflux_desktop_navigation::palette(cx);
        // Board 04: a task request's send button names the addressee.
        let ask_name = (self.mode == ComposerMode::Task)
            .then(|| self.addressee(cx).map(|agent| agent.name))
            .flatten();
        let send_label = match (self.mode, &ask_name) {
            (ComposerMode::Task, Some(name)) => tr_args("composer.ask_send", &[("name", name)]),
            (ComposerMode::Task, None) => tr("composer.start"),
            (ComposerMode::Message, _) => tr("composer.send"),
        };
        // Design 04: where the conversation has an agent, a receiver line
        // above the field names it and switches between a plain message
        // and a task request (which still waits for an explicit start).
        let has_text = !self.input.read(cx).value().trim().is_empty() || self.staged.is_some();
        let agents = self.agents(cx);
        let can_ask = (!agents.is_empty() || self.has_unknown_member(cx)) && !in_draft;
        // Board 04 draws the receiver line in a conversation with an agent;
        // boards 02/I24 draw a group's composer without it. A group keeps
        // the mode switch as an icon in the field and shows the line only
        // while a task request is being written.
        let direct = {
            let store = self.store.read(cx).store();
            self.context
                .as_ref()
                .and_then(|id| store.context(id))
                .is_some_and(|c| c.kind == ramflux_desktop_core::ContextKind::Direct)
        };
        let task_mode = self.mode == ComposerMode::Task;
        let mode_icon = (can_ask && !direct && !task_mode).then(|| {
            Button::new("composer-mode")
                .ghost()
                .small()
                .icon(gpui_kit::assets::IconName::ListChecks)
                .accessibility_label(tr("composer.task_mode"))
                .tooltip(task_reason.clone().unwrap_or_else(|| tr("composer.task_mode")))
                .disabled(!enabled || task_reason.is_some())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.set_mode(ComposerMode::Task, window, cx);
                }))
        });
        let receiver = (can_ask && (direct || task_mode)).then(|| {
            let name = self
                .picked
                .as_ref()
                .and_then(|id| agents.iter().find(|a| &a.id == id))
                .or(agents.first())
                .map(|a| a.name.clone())
                .unwrap_or_default();
            let task = self.mode == ComposerMode::Task;
            v_flex()
                .id("composer-receiver")
                .test_support()
                .gap_1()
                .child(
                    h_flex()
                        .gap_2()
                        .text_size(gpui_kit::rems(palette.size_secondary / 15.0))
                        .child(div().text_color(palette.secondary).child(tr("composer.receiver")))
                        .child(div().child(name))
                        .child(
                            Button::new("composer-mode")
                                .outline()
                                .xsmall()
                                .border_color(palette.text)
                                .selected(task)
                                .toggled(task)
                                .label(tr(if task {
                                    "composer.mode_task"
                                } else {
                                    "composer.mode_message"
                                }))
                                .accessibility_label(tr("composer.task_mode"))
                                .tooltip(
                                    task_reason.clone().unwrap_or_else(|| tr("composer.task_mode")),
                                )
                                .disabled(!enabled || (task_reason.is_some() && !task))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let next = match this.mode {
                                        ComposerMode::Message => ComposerMode::Task,
                                        ComposerMode::Task => ComposerMode::Message,
                                    };
                                    this.set_mode(next, window, cx);
                                })),
                        ),
                )
                .when(task, |line| {
                    line.child(
                        div()
                            .text_size(gpui_kit::rems(palette.size_meta / 15.0))
                            .text_color(palette.secondary)
                            .child(tr("composer.task_help")),
                    )
                })
        });
        let row = h_flex()
            .items_center()
            .w_full()
            .gap_2()
            .child(
                Button::new("composer-attach")
                    .ghost()
                    .small()
                    .icon(gpui_kit::assets::IconName::Paperclip)
                    .accessibility_label(tr("composer.attach.label"))
                    .tooltip(attach_reason.clone().unwrap_or_else(|| tr("composer.attach.label")))
                    .disabled(
                        !enabled || attach_reason.is_some() || self.mode == ComposerMode::Task,
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.pick_file(window, cx))),
            )
            .children(mode_icon)
            .child(
                div().id("composer-input").test_support().flex_1().min_w_0().child(
                    // The design's composer is one bordered field: the editor
                    // itself draws no second box.
                    Textarea::new(&self.input)
                        .aria_label(tr("composer.label"))
                        .appearance(false)
                        .bordered(false)
                        .text_size(gpui_kit::rems(palette.size_body / 15.0))
                        .disabled(!enabled),
                ),
            )
            .child(
                Button::new("composer-send")
                    .primary()
                    // Design: a quiet surface button until there is text.
                    .when(!has_text, |button| {
                        button.ghost().bg(palette.surface).text_color(palette.control)
                    })
                    .small()
                    .icon(IconName::ArrowUp)
                    .when(ask_name.is_some(), |button| button.label(send_label.clone()))
                    .rounded(gpui_kit::px(palette.radius_xl))
                    .accessibility_label(send_label.clone())
                    .tooltip(send_label)
                    .disabled(!can_send)
                    .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
            );
        // Design board 02: the composer region is the 880 px reading column,
        // 8 px above and 16 px below.
        v_flex()
            .id("composer")
            .test_support()
            .w_full()
            .max_w(gpui_kit::rems(palette.column_max / 15.0))
            .mx_auto()
            .gap_1()
            .pt_2()
            .pb(gpui_kit::rems(16.0 / 15.0))
            .children(receiver)
            // Design `Composer / Default`: the generated shell (no empty slot
            // adds space since PEN-GEN-05), the staged file above the editor.
            .child({
                let slots = ramflux_desktop_navigation::generated::catalog::ComposerSlots::new(
                    "conversation".into(),
                    row,
                    div(),
                );
                let slots = match chip {
                    Some(chip) => slots.attachment(chip),
                    None => slots,
                };
                ramflux_desktop_navigation::generated::catalog::composer_shell(slots, palette)
            })
            .when_some(reason.filter(|_| enabled), |col, reason| {
                col.child(
                    div()
                        .id("composer-unavailable")
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(reason)
                        .test_support(),
                )
            })
    }
}

impl Composer {
    /// Focuses the text input.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }
}
