// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The objects dock: every transfer in a table (name, direction, progress,
//! state) with the actions its state allows (pull, save, retry).

use gpui_kit::component::button::Button;
use gpui_kit::component::table::{Column, DataTable, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, Entity, IntoElement, Render, Subscription, Window, div, px,
};
use ramflux_desktop_core::{
    Direction, Listing, Object, ObjectAction, TransferState, UiEventKind, selectors,
};

use crate::app::{StoreEvent, StoreHandle};
use crate::bubble::EventSink;
use crate::i18n::{self, tr};
use crate::ids::domain_id;
use crate::moments::save_with_dialog;

const COLUMNS: [(&str, &str, f32); 5] = [
    ("name", "dock.objects.name", 128.),
    ("direction", "dock.objects.direction", 52.),
    ("progress", "dock.objects.progress", 52.),
    ("state", "dock.objects.state", 64.),
    ("actions", "dock.objects.actions", 92.),
];

/// Table rows and how to emit their actions.
pub struct ObjectsDelegate {
    rows: Vec<Object>,
    sink: EventSink,
}

impl TableDelegate for ObjectsDelegate {
    fn columns_count(&self, _: &App) -> usize {
        COLUMNS.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let (key, label, width) = COLUMNS[col_ix];
        Column::new(key, tr(label)).width(px(width))
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(object) = self.rows.get(row_ix) else {
            return div().into_any_element();
        };
        match COLUMNS[col_ix].0 {
            "name" => div().truncate().child(object.name.clone()).into_any_element(),
            "direction" => div().child(i18n::direction(object.direction)).into_any_element(),
            "progress" => {
                div().child(format!("{:.0}%", object.progress() * 100.0)).into_any_element()
            }
            "state" => div()
                .text_color(match object.transfer {
                    TransferState::Failed { .. } => cx.theme().danger,
                    _ => cx.theme().foreground,
                })
                .child(i18n::transfer(&object.transfer))
                .into_any_element(),
            _ => actions(object, &self.sink),
        }
    }
}

fn actions(object: &Object, sink: &EventSink) -> AnyElement {
    let action = match (&object.transfer, object.direction) {
        (TransferState::Queued, Direction::Inbound) => {
            Some(("pull", "moment.object.pull", ObjectAction::Pull))
        }
        (TransferState::Complete, Direction::Inbound) if object.saved_to.is_none() => {
            return Button::new(domain_id("objects-save", &object.id))
                .xsmall()
                .outline()
                .label(tr("moment.object.save"))
                .on_click(save_with_dialog(sink, object))
                .into_any_element();
        }
        (TransferState::Failed { .. }, _) => {
            Some(("retry", "moment.object.retry", ObjectAction::Retry))
        }
        _ => None,
    };
    let Some((key, label, action)) = action else {
        return div().into_any_element();
    };
    let sink = sink.clone();
    let object_id = object.id.clone();
    Button::new(domain_id(&format!("objects-{key}"), &object.id))
        .xsmall()
        .outline()
        .label(tr(label))
        .on_click(move |_, window, cx| {
            sink.emit(
                UiEventKind::ObjectActionPressed {
                    object_id: object_id.clone(),
                    action: action.clone(),
                },
                window,
                cx,
            );
        })
        .into_any_element()
}

/// The objects dock content.
pub struct ObjectsDock {
    store: Entity<StoreHandle>,
    table: Entity<TableState<ObjectsDelegate>>,
    _subscription: Subscription,
}

impl ObjectsDock {
    /// A dock over `store`, emitting through `sink` (scoped to the dock).
    pub fn new(
        store: Entity<StoreHandle>,
        sink: EventSink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let rows = selectors::objects(store.read(cx).store()).into_iter().cloned().collect();
        let table = cx.new(|cx| TableState::new(ObjectsDelegate { rows, sink }, window, cx));
        let subscription = cx.subscribe(&store, |this, store, event: &StoreEvent, cx| {
            if let StoreEvent::Changed(changes) = event
                && changes.touches_docks()
            {
                let rows: Vec<Object> =
                    selectors::objects(store.read(cx).store()).into_iter().cloned().collect();
                this.table.update(cx, |table, cx| {
                    table.delegate_mut().rows = rows;
                    table.refresh(cx);
                });
                cx.notify();
            }
        });
        Self { store, table, _subscription: subscription }
    }

    /// Ids of the listed objects, in table order.
    pub fn object_ids(&self, cx: &App) -> Vec<String> {
        self.table.read(cx).delegate().rows.iter().map(|o| o.id.to_string()).collect()
    }
}

impl Render for ObjectsDock {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx).store();
        let empty = selectors::objects(store).is_empty();
        let state = crate::list_state::of(store, Listing::Objects, empty);
        v_flex()
            .id("dock-objects")
            .size_full()
            .min_h_0()
            .children(crate::list_state::placeholder(
                "objects-list-state",
                state,
                "dock.objects.empty",
                cx,
            ))
            .when(!empty, |col| {
                col.child(
                    h_flex()
                        .flex_1()
                        .min_h_0()
                        .p_2()
                        .child(DataTable::new(&self.table).stripe(true).bordered(true).small()),
                )
            })
    }
}
