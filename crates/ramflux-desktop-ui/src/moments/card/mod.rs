// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Agent cards, drawn natively from the card runtime's tree
//! (`ramflux.card.tree.v1`, held by the store per surface).
//!
//! Every node is drawn by a local component of its kind; the tree is data.
//! All tree text is untrusted and drawn as plain text: no markup, no link
//! detection, no image loading. Events leave as `CardInteracted` (typed
//! input, tap, tab, modal) and `CardSlotAnswered` (link sheet, approval),
//! addressed by the runtime's node keys and slot ids; the runtime decides
//! what they mean. Interaction the runtime does not keep (which tab is shown,
//! which sheet is open, which list is expanded) is held in [`CardLocal`].
//!
//! Text fields keep their draft in an input entity and send it to the runtime
//! just before a Button press (the press is the submit's trigger), so the
//! runtime sees the value the user typed.
//!
//! - `layout`: row, column, list, card, divider, tabs, modal trigger.
//! - `content`: text, icon, image, chart, table, task.
//! - `controls`: button, text field, check box, choice picker, slider, date.
//! - `slots`: approval, the modal sheet and the link confirmation.
//! - `chrome`: lifecycle status, notices, fallback text and the surface's
//!   own actions (the task-step decision buttons).

mod chrome;
mod content;
mod controls;
mod layout;
mod slots;
#[cfg(feature = "card-gallery")]
pub mod trace;

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use gpui_kit::component::input::InputState;
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, App, Entity, IntoElement, RenderOnce, SharedString, Window};
use ramflux_desktop_core::{
    CardNode, CardRecord, CardSeen, CardStatus, CardTree, JsonValue, Moment, MomentKind, Store,
    SurfaceAction, SurfaceHash, SurfaceId, SurfaceState,
};
use ramflux_desktop_navigation::direction::LayoutDirection;

use crate::bubble::EventSink;
use crate::i18n::bidi::strip_controls;

/// What a card row needs from the store, copied when the row is built.
#[derive(Clone, Debug, PartialEq)]
pub struct CardSnapshot {
    /// The surface (the daemon's `surface_ref`).
    pub surface_id: SurfaceId,
    /// The content hash the surface's actions are bound to.
    pub hash: Option<SurfaceHash>,
    /// The version number the actions are bound to.
    pub version: u64,
    /// The author's fallback text.
    pub fallback: SharedString,
    /// The surface's own actions (task-step decisions).
    pub actions: Vec<SurfaceAction>,
    /// The authority's state of the surface.
    pub state: Option<SurfaceState>,
    /// The runtime's card for the surface, when one is open.
    pub record: Option<CardRecord>,
    /// The author's display name (shown in the link confirmation).
    pub author: SharedString,
}

impl CardSnapshot {
    /// The snapshot of an agent surface moment; `None` for other kinds.
    pub fn of(store: &Store, moment: &Moment, author: SharedString) -> Option<Self> {
        let MomentKind::AgentSurface(payload) = &moment.kind else { return None };
        let surface = store.surface(&moment.id);
        Some(Self {
            surface_id: payload.surface_id.clone(),
            hash: surface.map(|s| s.hash.clone()),
            version: surface.map_or(0, |s| s.version),
            fallback: payload.fallback_text.clone().into(),
            actions: payload.actions.clone(),
            state: surface.map(|s| s.state),
            record: store.card(&payload.surface_id).cloned(),
            author,
        })
    }

    /// Whether the runtime may take a host event now: a live, current card.
    fn interactive(&self) -> bool {
        self.state == Some(SurfaceState::Open)
            && self.record.as_ref().is_some_and(|r| r.status == CardStatus::Live)
    }
}

/// Interaction the runtime does not keep, per surface, held by the row.
#[derive(Default, Clone)]
pub struct CardLocal {
    /// The selected tab index, by Tabs node key.
    pub tabs: BTreeMap<String, u64>,
    /// The Modal node whose sheet is open.
    pub modal: Option<String>,
    /// Lists expanded past their first five items, by List node key.
    pub expanded: BTreeSet<String>,
    /// The text each text field last sent to the runtime, by node key.
    pub sent: BTreeMap<String, String>,
    /// The tree value each text field was last synced to, by node key.
    pub synced: BTreeMap<String, String>,
    /// The tree version a forced field reset (a refusal or a discarded
    /// draft) was applied at, so it happens once per version.
    pub reset_at: Option<u64>,
}

/// Text field drafts of one card, by node key.
pub type Inputs = Rc<BTreeMap<String, Entity<InputState>>>;

/// The shared view state of one card row.
pub type Local = Rc<RefCell<CardLocal>>;

/// The card body of an agent surface row.
#[derive(IntoElement)]
pub struct CardBody {
    snap: CardSnapshot,
    sink: EventSink,
    inputs: Inputs,
    local: Local,
}

impl CardBody {
    /// A card body emitting through `sink`, which must be scoped to the surface.
    pub fn new(snap: CardSnapshot, sink: EventSink, inputs: Inputs, local: Local) -> Self {
        Self { snap, sink, inputs, local }
    }
}

impl RenderOnce for CardBody {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self { snap, sink, inputs, local } = self;
        #[cfg(feature = "card-gallery")]
        let _trace = trace::Scope::begin(snap.surface_id.as_str());
        let tree = snap.record.as_ref().and_then(|r| r.tree.as_ref());
        let shown =
            tree.filter(|_| snap.record.as_ref().is_some_and(|r| r.status != CardStatus::Fallback));
        let body = match (shown, snap.record.as_ref()) {
            (Some(tree), Some(record)) => {
                let view = View {
                    tree,
                    interactive: snap.interactive(),
                    status: record.status,
                    sink: sink.clone(),
                    inputs: inputs.clone(),
                    local: local.clone(),
                    dir: direction_of(tree),
                    author: snap.author.clone(),
                };
                let root = tree.root().map(str::to_owned);
                let card = match root.as_deref().and_then(|key| view.node(key)) {
                    Some(node) => view.render(node, window, cx),
                    None => chrome::fallback(&snap, cx),
                };
                v_stack_card(cx)
                    .child(chrome::status(record, tree, &snap, &sink, window, cx))
                    .child(card)
                    .when_some(slots::link_sheet(&view, record, &snap, window, cx), |col, sheet| {
                        col.child(sheet)
                    })
                    .into_any_element()
            }
            _ => v_stack_card(cx).child(chrome::fallback(&snap, cx)).into_any_element(),
        };
        v_stack_card(cx).child(body).child(chrome::surface_actions(&snap, &sink, window, cx))
    }
}

/// The frame of a card: a neutral inset block, no second container inside it.
fn v_stack_card(cx: &App) -> gpui_kit::Div {
    let _ = cx;
    gpui_kit::component::v_flex().gap(ramflux_desktop_navigation::scale::space(2.))
}

/// The layout direction a tree asks for.
fn direction_of(tree: &CardTree) -> LayoutDirection {
    if tree.direction() == "rtl" {
        LayoutDirection::RightToLeft
    } else {
        LayoutDirection::LeftToRight
    }
}

/// The view a card's nodes are drawn through.
pub(crate) struct View<'a> {
    pub tree: &'a CardTree,
    /// The runtime accepts host events now.
    pub interactive: bool,
    /// The overlay status of the card.
    pub status: CardStatus,
    /// Scoped to the surface.
    pub sink: EventSink,
    pub inputs: Inputs,
    pub local: Local,
    pub dir: LayoutDirection,
    /// The author's display name (untrusted text).
    pub author: SharedString,
}

impl View<'_> {
    /// A node by key.
    pub fn node(&self, key: &str) -> Option<&CardNode> {
        self.tree.node(key)
    }

    /// What the user sees of `node` now: the held tree's state, carried by
    /// every host event on the node.
    pub fn seen(&self, node: &CardNode) -> CardSeen {
        self.tree.seen(&node.key).unwrap_or_default()
    }

    /// What the user sees of the whole card now (slot answers).
    pub fn seen_card(&self) -> CardSeen {
        CardSeen { stamp: self.tree.stamp(), ..CardSeen::default() }
    }

    /// The children of a node, in order.
    pub fn children<'b>(&'b self, node: &CardNode) -> Vec<&'b CardNode> {
        node.children.iter().filter_map(|key| self.tree.node(key)).collect()
    }

    /// Whether `node` takes host events now.
    pub fn enabled(&self, node: &CardNode) -> bool {
        self.interactive && !attr_bool(node, "disabled")
    }

    /// Draws one node by its kind. An unknown kind shows the unsupported
    /// notice (the card as a whole falls back only when the tree does).
    pub fn render(&self, node: &CardNode, window: &mut Window, cx: &mut App) -> AnyElement {
        #[cfg(feature = "card-gallery")]
        trace::drawn(&node.key, &node.kind);
        match node.kind.as_str() {
            "Text" => content::text(self, node, cx),
            "Icon" => content::icon(node, cx),
            "Row" => layout::row(self, node, window, cx),
            "Column" => layout::column(self, node, window, cx),
            "List" => layout::list(self, node, window, cx),
            "Card" => layout::card(self, node, window, cx),
            "Divider" => layout::divider(node, cx),
            "Tabs" => layout::tabs(self, node, window, cx),
            "Modal" => layout::modal(self, node, window, cx),
            "Button" => controls::button(self, node, window, cx),
            "TextField" => controls::text_field(self, node, cx),
            "CheckBox" => controls::check_box(self, node, window, cx),
            "ChoicePicker" => controls::choice_picker(self, node, window, cx),
            "Slider" => controls::slider(self, node, window, cx),
            "DateTimeInput" => controls::date_time(self, node, cx),
            "Image" => content::image(node, cx),
            "Approval" => slots::approval(self, node, window, cx),
            "Task" => content::task(node, cx),
            "Chart" => content::chart(node, cx),
            "Table" => content::table(node, cx),
            _ => chrome::unsupported(cx),
        }
    }
}

/// A string attribute.
pub(crate) fn attr_str<'a>(node: &'a CardNode, name: &str) -> Option<&'a str> {
    node.attrs.get(name).and_then(JsonValue::as_str)
}

/// Tree text as it is drawn: directional and other controls removed, so a
/// card cannot reorder or hide the text around it. Every tree string passes
/// through here before it reaches the screen.
pub(crate) fn shown(text: &str) -> String {
    strip_controls(text).into_owned()
}

/// A string attribute as drawn text (see [`shown`]); empty when absent.
pub(crate) fn attr_text(node: &CardNode, name: &str) -> String {
    attr_str(node, name).map(shown).unwrap_or_default()
}

/// A boolean attribute (false when absent).
pub(crate) fn attr_bool(node: &CardNode, name: &str) -> bool {
    node.attrs.get(name).and_then(JsonValue::as_bool).unwrap_or(false)
}

/// A number attribute.
pub(crate) fn attr_f64(node: &CardNode, name: &str) -> Option<f64> {
    node.attrs.get(name).and_then(JsonValue::as_f64)
}

/// The validation messages a node carries (plain text).
pub(crate) fn errors(node: &CardNode) -> Vec<String> {
    node.attrs
        .get("errors")
        .and_then(JsonValue::as_array)
        .map(|rows| rows.iter().filter_map(|row| row.as_str().map(shown)).collect())
        .unwrap_or_default()
}
