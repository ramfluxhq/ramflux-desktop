// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The Tasks section (design 06): categories in the list pane (needs me,
//! all, running, done) and the chosen category's task cards in the detail
//! pane, with the answers the task itself offers (start a proposal, cancel,
//! open its conversation). Answers go through the root as the conversation's
//! task card sends them, scoped to the task's moment, so the same intents
//! and confirmations apply. History is loaded per conversation: the
//! conversations whose tasks are not known yet are listed, never dropped.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::{Selectable as _, Sizable as _, StyledExt as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, EventEmitter, IntoElement, Render, SharedString,
    Subscription, Window, div,
};
use ramflux_desktop_core::{
    Actor, ContextId, MomentId, MomentKind, Scope, Store, Task, TaskId, TaskState, UiEventKind,
    selectors,
};
use ramflux_desktop_navigation::direction::{DirectionalExt as _, h_flex};
use ramflux_desktop_navigation::scale::space;

use super::directory::{self, Search};
use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{self, tr, tr_args, tr_count};
use crate::style::DesignButton as _;

/// A category of the list pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskCategory {
    /// Proposals and steps that wait for the user.
    NeedsMe,
    /// Every loaded task.
    All,
    /// Started and not finished.
    Running,
    /// Finished, failed or cancelled.
    Done,
}

impl TaskCategory {
    const ALL: [Self; 4] = [Self::NeedsMe, Self::All, Self::Running, Self::Done];

    fn key(self) -> &'static str {
        match self {
            Self::NeedsMe => "needs_me",
            Self::All => "all",
            Self::Running => "running",
            Self::Done => "done",
        }
    }

    fn holds(self, task: &Task) -> bool {
        match self {
            Self::NeedsMe => matches!(task.state, TaskState::NeedsMe | TaskState::Proposed),
            Self::All => true,
            Self::Running => task.state == TaskState::Running,
            Self::Done => task.state.is_terminal(),
        }
    }
}

/// What the area asks of the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TasksEvent {
    /// Show this conversation, focused on this moment if given.
    Open(ContextId, Option<MomentId>),
    /// A category was chosen (compact widths then show its tasks).
    CategoryChosen,
}

/// The Tasks section's view state.
pub struct TasksArea {
    store: Entity<StoreHandle>,
    sink: EventSink,
    search: Search,
    category: TaskCategory,
    /// The task whose details dialog is open.
    open_task: Option<TaskId>,
    /// Tracked by the dialog's focus trap.
    dialog_focus: gpui_kit::FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TasksEvent> for TasksArea {}

impl TasksArea {
    /// The area over the rendered account's store; answers go to `sink`.
    pub fn new(
        store: Entity<StoreHandle>,
        sink: EventSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = Search::new("sections.tasks.search", window, cx);
        let subscriptions = vec![
            cx.subscribe(&store, |_, _, event: &StoreEvent, cx| {
                if let StoreEvent::Changed(_) = event {
                    cx.notify();
                }
            }),
            cx.subscribe(search.input(), |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            crate::i18n::on_locale_switch(cx, |_, cx| cx.notify()),
        ];
        Self {
            store,
            sink,
            search,
            category: TaskCategory::NeedsMe,
            open_task: None,
            dialog_focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// Shows `category`.
    pub fn set_category(&mut self, category: TaskCategory, cx: &mut Context<Self>) {
        self.category = category;
        cx.notify();
    }

    /// Opens the details dialog of the task `id`.
    pub fn open_details(&mut self, id: TaskId, cx: &mut Context<Self>) {
        self.open_task = Some(id);
        cx.notify();
    }

    /// The task whose details dialog is open.
    pub fn open_task(&self) -> Option<&TaskId> {
        self.open_task.as_ref()
    }

    /// Closes the details dialog.
    pub fn close_details(&mut self, cx: &mut Context<Self>) {
        if self.open_task.take().is_some() {
            cx.notify();
        }
    }

    /// The task details dialog (board E05), drawn by the workspace over the
    /// whole window while a task is open: title, state, the steps with
    /// their states, then Cancel task and, for a proposal, Confirm and start.
    pub fn dialog(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.open_task.clone()?;
        let palette = ramflux_desktop_navigation::palette(cx);
        let rem = |px: f32| gpui_kit::rems(px / 15.0);
        let store = self.store.read(cx).store();
        let Some(task) = store.tasks().find(|t| t.id == id).cloned() else {
            self.open_task = None;
            return None;
        };
        let moment = task_moment(store, &task).map(|m| m.id.clone());
        let task_sink = moment.map(|moment| {
            self.sink.scoped(Scope::Context(task.context_id.clone())).scoped(Scope::Moment(moment))
        });
        let mine = task.requester == Actor::Human;
        let proposal = task.state == TaskState::Proposed;
        let steps = task.steps.iter().enumerate().map(|(index, step)| {
            v_flex()
                .id(SharedString::from(format!("task-step-{}", step.id)))
                .py(rem(22.0))
                .gap(rem(2.0))
                .border_b_1()
                .border_color(palette.line)
                .child(
                    div()
                        .text_size(rem(palette.size_body))
                        .child(i18n::join(&(index + 1).to_string(), &step.label)),
                )
                .child(
                    div()
                        .text_size(rem(palette.size_secondary))
                        .text_color(palette.secondary)
                        .child(i18n::step_state(step.state)),
                )
        });
        let body = v_flex()
            .id("task-details")
            .test_support()
            .gap(space(3.))
            .child(
                div()
                    .text_size(rem(palette.size_title))
                    .font_semibold()
                    .child(SharedString::from(task.title.clone())),
            )
            .child(
                h_flex().child(
                    div()
                        .px(space(2.))
                        .py(rem(4.0))
                        .rounded(rem(palette.radius_chip))
                        .bg(palette.surface)
                        .text_size(rem(palette.size_meta))
                        .child(i18n::task_state(task.state)),
                ),
            )
            .child(v_flex().id("task-details-steps").children(steps));
        let this = cx.weak_entity();
        let close = move |_: &mut Window, cx: &mut App| {
            this.update(cx, |area, cx| area.close_details(cx)).ok();
        };
        let this = cx.weak_entity();
        let close_button = move |_: &mut Window, cx: &mut App| {
            this.update(cx, |area, cx| area.close_details(cx)).ok();
        };
        let cancel_sink = task_sink.clone().filter(|_| mine && !task.state.is_terminal());
        let start_sink = task_sink.filter(|_| proposal && mine);
        let mut modal = crate::modal::Modal::new(
            "task-details-dialog",
            tr("sections.tasks.details_title"),
            &self.dialog_focus,
            close,
        )
        .width(crate::modal::width::WIDE)
        .child(body);
        modal = match cancel_sink {
            Some(sink) => modal.button(
                gpui_kit::component::button::Button::new("task-details-cancel")
                    .design_secondary(cx)
                    .design_label(tr("sections.tasks.cancel"))
                    .on_click(move |_, window, cx| {
                        sink.emit(UiEventKind::CancelPressed, window, cx)
                    }),
            ),
            None => modal.button(crate::modal::cancel_button(
                "task-details-close",
                tr("common.close"),
                cx,
                close_button,
            )),
        };
        if let Some(sink) = start_sink {
            modal = modal.button(
                gpui_kit::component::button::Button::new("task-details-start")
                    .design_primary(cx)
                    .design_label(tr("sections.tasks.start"))
                    .on_click(move |_, window, cx| {
                        sink.emit(UiEventKind::ProposalAccepted, window, cx)
                    }),
            );
        }
        Some(modal.render(cx))
    }

    /// The category shown.
    pub fn category(&self) -> TaskCategory {
        self.category
    }

    /// Focuses the search field.
    pub fn focus_search(&self, window: &mut Window, cx: &mut App) {
        self.search.focus(window, cx);
    }

    fn tasks<'a>(&self, store: &'a Store, category: TaskCategory, cx: &App) -> Vec<&'a Task> {
        let query = self.search.query(cx);
        let mut tasks: Vec<&Task> = store
            .tasks()
            .filter(|t| category.holds(t))
            .filter(|t| directory::matches(&query, &[&t.title]))
            .collect();
        tasks.sort_by_key(|t| t.title.to_lowercase());
        tasks
    }

    /// The detail pane: the category's header and its task cards, then the
    /// conversations whose task history is not loaded.
    pub fn detail(
        &mut self,
        tools: Option<AnyElement>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = ramflux_desktop_navigation::palette(cx);
        let rem = |px: f32| gpui_kit::rems(px / 15.0);
        let category = self.category;
        let store = self.store.read(cx).store();
        let facts: Vec<CardFacts> = self
            .tasks(store, category, cx)
            .into_iter()
            .map(|task| CardFacts {
                task: task.clone(),
                assignee: store
                    .participant(&task.assignee)
                    .map_or_else(|| task.assignee.to_string(), |p| p.display.clone()),
                source: store.context(&task.context_id).map_or_else(
                    || task.context_id.to_string(),
                    |c| selectors::context_title(store, c),
                ),
                moment: task_moment(store, task).map(|m| m.id.clone()),
            })
            .collect();
        let unknown: Vec<(ContextId, SharedString, SharedString)> = store
            .contexts()
            .filter(|c| {
                !store.context_moments(&c.id).any(|m| {
                    matches!(m.kind, MomentKind::TaskProgress(_) | MomentKind::TaskEvent(_))
                })
            })
            .map(|c| {
                (
                    c.id.clone(),
                    ramflux_desktop_navigation::row::list_text(&selectors::context_title(store, c)),
                    tr(if store.context_moment_count(&c.id) == 0 {
                        "navigation.tasks_not_loaded"
                    } else {
                        "navigation.tasks_none_loaded"
                    }),
                )
            })
            .collect();
        let cards: Vec<AnyElement> =
            facts.into_iter().map(|facts| card(facts, &self.sink, cx)).collect();
        let empty = cards.is_empty().then(|| {
            div()
                .id("tasks-empty")
                .test_support()
                .role(gpui_kit::Role::Status)
                .text_size(rem(palette.size_body))
                .text_color(palette.secondary)
                .child(tr(&format!("sections.tasks.empty.{}", category.key())))
        });
        v_flex()
            .id("tasks-detail")
            .test_support()
            .size_full()
            // The design's page header.
            .child(crate::page_header::page_header(
                tr(&format!("sections.tasks.{}", category.key())),
                ramflux_desktop_navigation::scale::style::subheading(),
                tr(&format!("sections.tasks.help.{}", category.key())),
                None,
                h_flex().gap(space(3.)).children(tools).when(
                    category != TaskCategory::All,
                    |slot| {
                        slot.child(
                            gpui_kit::component::button::Button::new("tasks-view-all")
                                .ghost()
                                .small()
                                .label(tr("sections.tasks.view_all"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_category(TaskCategory::All, cx)
                                })),
                        )
                    },
                ),
                palette,
            ))
            .child(
                v_flex()
                    .id("tasks-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(rem(32.0))
                    .py(rem(32.0))
                    .gap(space(4.))
                    .children(empty)
                    .children(cards)
                    .when(!unknown.is_empty(), |body| {
                        body.child(
                            div()
                                .pt(space(2.))
                                .text_size(rem(palette.size_secondary))
                                .text_color(palette.secondary)
                                .child(tr("navigation.tasks_history_notice")),
                        )
                        .child(
                            v_flex().id("tasks-unloaded").gap(space(1.)).children(
                                unknown.into_iter().map(|(id, title, state)| {
                                    let open = id.clone();
                                    gpui_kit::component::button::Button::new(SharedString::from(
                                        format!("tasks-context-{id}"),
                                    ))
                                    .ghost()
                                    .w_full()
                                    .h_auto()
                                    .py(space(2.))
                                    .accessibility_label(i18n::join(&title, &state))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(TasksEvent::Open(open.clone(), None))
                                    }))
                                    .child(
                                        h_flex()
                                            .w_full()
                                            .gap(space(3.))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .truncate()
                                                    .text_size(rem(palette.size_body))
                                                    .child(title),
                                            )
                                            .child(
                                                div()
                                                    .text_size(rem(palette.size_secondary))
                                                    .text_color(palette.secondary)
                                                    .child(state),
                                            ),
                                    )
                                }),
                            ),
                        )
                    }),
            )
            .into_any_element()
    }
}

/// The moment that carries `task` in its conversation.
fn task_moment<'a>(store: &'a Store, task: &Task) -> Option<&'a ramflux_desktop_core::Moment> {
    store
        .context_moments(&task.context_id)
        .find(|m| matches!(&m.kind, MomentKind::TaskProgress(p) if p.task_id == task.id))
}

/// One task card (design 06): title, steps, state, assignee, source and
/// state facts, then the answers this task offers the user.
/// What a card shows, read from the store before the card is built.
struct CardFacts {
    task: Task,
    assignee: String,
    source: String,
    moment: Option<MomentId>,
}

fn card(facts: CardFacts, sink: &EventSink, cx: &mut Context<TasksArea>) -> AnyElement {
    let palette = ramflux_desktop_navigation::palette(cx);
    let rem = |px: f32| gpui_kit::rems(px / 15.0);
    let CardFacts { task, assignee, source, moment } = facts;
    let task = &task;
    let mine = task.requester == Actor::Human;
    let proposal = task.state == TaskState::Proposed;
    let task_sink = moment
        .clone()
        .map(|id| sink.scoped(Scope::Context(task.context_id.clone())).scoped(Scope::Moment(id)));
    let steps = if task.steps.is_empty() {
        tr("sections.tasks.no_steps")
    } else {
        tr_count("sections.tasks.steps", u64::try_from(task.steps.len()).unwrap_or(u64::MAX))
    };
    let current = task
        .steps
        .iter()
        .find(|s| !matches!(s.state, ramflux_desktop_core::StepState::Done))
        .map(|s| SharedString::from(s.label.clone()));
    let fact = |label: &str, value: SharedString| {
        h_flex()
            .gap(space(1.))
            .text_size(rem(palette.size_secondary))
            .child(div().text_color(palette.secondary).child(tr(label)))
            .child(div().child(value))
    };
    let id = task.id.clone();
    let context = task.context_id.clone();
    let open_moment = moment.clone();
    let details = id.clone();
    v_flex()
        .id(SharedString::from(format!("task-card-{id}")))
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, window, cx| {
            this.open_details(details.clone(), cx);
            window.focus(&this.dialog_focus, cx);
        }))
        .test_support()
        .role(gpui_kit::Role::Article)
        .aria_label(SharedString::from(task.title.clone()))
        .w_full()
        .max_w(rem(760.0))
        .p(rem(20.0))
        .gap(space(3.))
        .rounded(rem(palette.radius_card))
        .border_1()
        .border_color(palette.line)
        .bg(palette.surface_white)
        .child(
            div()
                .text_size(rem(palette.size_subheading))
                .font_semibold()
                .child(SharedString::from(task.title.clone())),
        )
        .child(div().text_size(rem(palette.size_body)).child(match &current {
            Some(step) => i18n::join(&steps, step),
            None => steps.clone(),
        }))
        .child(
            h_flex().child(
                div()
                    .px(space(2.))
                    .py(rem(4.0))
                    .rounded(rem(palette.radius_chip))
                    .bg(palette.surface)
                    .text_size(rem(palette.size_meta))
                    .child(i18n::task_state(task.state)),
            ),
        )
        .child(
            h_flex()
                .gap(rem(32.0))
                .pb(space(3.))
                .border_b_1()
                .border_color(palette.line)
                .child(fact("sections.tasks.assignee", assignee.into()))
                .child(fact("sections.tasks.source", source.into()))
                .child(fact("sections.tasks.state", i18n::task_state(task.state))),
        )
        .child(div().text_size(rem(palette.size_body)).child(tr("sections.tasks.scope")))
        // The source sends no scope text with a task: say so rather than
        // invent one.
        .child(
            div()
                .id(SharedString::from(format!("task-scope-{id}")))
                .test_support()
                .text_size(rem(palette.size_secondary))
                .text_color(palette.secondary)
                .child(tr("sections.tasks.scope_unavailable")),
        )
        .child(
            h_flex()
                .gap(space(3.))
                // The answers do not also open the details.
                .on_mouse_up(gpui_kit::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .when_some(task_sink.clone().filter(|_| proposal && mine), |row, sink| {
                    row.child(
                        gpui_kit::component::button::Button::new(SharedString::from(format!(
                            "task-start-{id}"
                        )))
                        .design_primary(cx)
                        .design_label(tr("sections.tasks.start"))
                        .on_click(move |_, window, cx| {
                            sink.emit(UiEventKind::ProposalAccepted, window, cx)
                        }),
                    )
                })
                .child(
                    gpui_kit::component::button::Button::new(SharedString::from(format!(
                        "task-open-{id}"
                    )))
                    .design_quiet(cx)
                    .design_label(tr(if task.state == TaskState::NeedsMe {
                        "sections.tasks.handle"
                    } else {
                        "sections.tasks.details"
                    }))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(TasksEvent::Open(context.clone(), open_moment.clone()))
                    })),
                )
                .when_some(task_sink.filter(|_| mine && !task.state.is_terminal()), |row, sink| {
                    row.child(
                        gpui_kit::component::button::Button::new(SharedString::from(format!(
                            "task-cancel-{id}"
                        )))
                        .design_quiet(cx)
                        // Board 06: "Cancel task" for a proposal as for a task.
                        .design_label(tr("sections.tasks.cancel"))
                        .on_click(move |_, window, cx| {
                            sink.emit(UiEventKind::CancelPressed, window, cx)
                        }),
                    )
                }),
        )
        .into_any_element()
}

impl Render for TasksArea {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ramflux_desktop_navigation::palette(cx);
        let counts: Vec<(TaskCategory, usize)> = {
            let store = self.store.read(cx).store();
            TaskCategory::ALL
                .into_iter()
                .map(|c| (c, store.tasks().filter(|t| c.holds(t)).count()))
                .collect()
        };
        v_flex()
            .id("tasks-area")
            .test_support()
            .size_full()
            .min_w_0()
            .bg(palette.nav)
            .border_end_1()
            .border_color(palette.line)
            .px(space(3.))
            .pt(gpui_kit::rems(24.0 / 15.0))
            .gap(space(4.))
            // Board 06: the heading's "+"; a task is asked for in a
            // conversation, so here it says where.
            .child(directory::header(
                tr("navigation.tasks"),
                crate::gate::gated(
                    gpui_kit::component::button::Button::new("tasks-new")
                        .small()
                        .ghost()
                        .icon(gpui_kit::assets::IconName::Plus)
                        .accessibility_label(tr("sections.tasks.new")),
                    Some(tr("sections.tasks.new_help")),
                    cx,
                ),
                cx,
            ))
            .child(self.search.field("tasks-search", tr("sections.tasks.search"), cx))
            // Task history loads per conversation: said here, on the route.
            .child(
                div()
                    .id("tasks-history-notice")
                    .test_support()
                    .role(gpui_kit::Role::Status)
                    .px(space(2.))
                    .text_size(gpui_kit::rems(palette.size_secondary / 15.0))
                    .text_color(palette.secondary)
                    .child(tr("navigation.tasks_history_notice")),
            )
            .child(v_flex().gap(space(1.)).children(counts.into_iter().map(|(category, count)| {
                let selected = self.category == category;
                let detail = if category == TaskCategory::NeedsMe {
                    tr_args("sections.tasks.count_waiting", &[("count", &count.to_string())])
                } else {
                    tr_args("sections.tasks.count", &[("count", &count.to_string())])
                };
                gpui_kit::component::button::Button::new(SharedString::from(format!(
                    "tasks-category-{}",
                    category.key()
                )))
                .ghost()
                .w_full()
                .h(gpui_kit::rems(ramflux_desktop_navigation::theme::ROW_REM))
                .px(space(3.))
                .rounded(gpui_kit::px(palette.radius_sm))
                .selected(selected)
                .toggled(selected)
                .accessibility_label(tr(&format!("sections.tasks.{}", category.key())))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_category(category, cx);
                    cx.emit(TasksEvent::CategoryChosen);
                }))
                .child(
                    v_flex()
                        .w_full()
                        .child(
                            div()
                                .text_size(gpui_kit::rems(palette.size_body / 15.0))
                                .font_medium()
                                .child(tr(&format!("sections.tasks.{}", category.key()))),
                        )
                        .child(
                            div()
                                .text_size(gpui_kit::rems(palette.size_secondary / 15.0))
                                .text_color(palette.secondary)
                                .child(detail),
                        ),
                )
            })))
    }
}
