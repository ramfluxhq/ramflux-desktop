// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The gallery window: every fixture drawn through the production renderer,
//! live (events go to the standalone runtime), with a conformance control.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::InputState;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ClickEvent, Context, IntoElement, Render, SharedString, Window, WindowOptions, div, px,
};
use ramflux_desktop_core::{
    CardInput, CardRecord, CardStatus, CardTree, SurfaceId, SurfaceState, UiEventKind,
};

use super::conformance::{self, Opened, Record, Report};
use super::fixtures::{self, Case, Trust};
use crate::bubble::EventSink;
use crate::i18n::{tr, tr_args};
use crate::moments::card::trace;
use crate::moments::card::{CardBody, CardLocal, CardSnapshot, Inputs, Local};
use crate::style::DesignButton as _;

/// What the gallery was launched with.
#[derive(Clone, Debug)]
pub struct GalleryConfig {
    /// The dark presentation instead of the light one.
    pub dark: bool,
    /// The locale tag of the presentation (`en` or `zh-CN`).
    pub locale: String,
    /// Run the conformance after the first frame, write it and exit.
    pub exit_after: bool,
}

impl GalleryConfig {
    /// The configuration of the gallery flags, or `None` when none is given.
    #[must_use]
    pub fn from_args(args: &[String]) -> Option<Self> {
        let flagged = args.iter().any(|arg| super::is_gallery_flag(arg));
        if !flagged {
            return None;
        }
        let locale = args
            .iter()
            .position(|arg| arg == "--card-gallery-locale")
            .and_then(|index| args.get(index + 1).cloned())
            .unwrap_or_else(|| "en".to_owned());
        Some(Self {
            dark: args.iter().any(|arg| arg == "--card-gallery-dark"),
            locale,
            exit_after: args.iter().any(|arg| arg == "--card-gallery-exit"),
        })
    }

    /// The presentation key in the trust package.
    fn presentation(&self) -> String {
        format!("{}_{}", self.locale, if self.dark { "dark" } else { "light" })
    }
}

/// One fixture as the gallery holds it. The runtime keeps a bounded number of
/// resident VMs, so only the card the user last touched holds a live session;
/// the others draw their last tree.
struct Entry {
    case: Case,
    /// The last tree the runtime gave for this fixture.
    shown: Option<CardTree>,
    /// The live session, while this card is the one being interacted with.
    opened: Option<Opened>,
    inputs: Inputs,
    local: Local,
    note: Option<String>,
}

/// Where the conformance run is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    /// A and C ran; the next frame schedules B's read after it is drawn.
    Requested,
    /// The frame that draws the cards is being drawn.
    Drawing,
    Done,
}

/// The gallery view.
pub struct CardGallery {
    config: GalleryConfig,
    entries: Vec<Entry>,
    trust: Option<Trust>,
    error: Option<String>,
    /// The initial held tree of each fixture, the starting point of check B.
    initial: BTreeMap<String, CardTree>,
    report: Report,
    phase: Phase,
    summary: Option<String>,
}

/// Opens the gallery window.
pub fn open(options: WindowOptions, cx: &mut App, config: GalleryConfig) {
    let opened = cx.open_window(options, move |window, cx| {
        cx.new(|cx| CardGallery::new(config.clone(), window, cx))
    });
    if let Err(error) = opened {
        ramflux_desktop_sdk::local::app_log::warn(&format!(
            "cannot open the card gallery: {error}"
        ));
    }
}

impl CardGallery {
    fn new(config: GalleryConfig, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut gallery = Self {
            config,
            entries: Vec::new(),
            trust: None,
            error: None,
            initial: BTreeMap::new(),
            report: Report::default(),
            phase: Phase::Idle,
            summary: None,
        };
        match load(&gallery.config.presentation()) {
            Ok((trust, cases)) => {
                gallery.entries = cases
                    .into_iter()
                    .map(|case| entry(case, &trust, &gallery.config, window, cx))
                    .collect();
                gallery.trust = Some(trust);
            }
            Err(Failure::Presentation(error)) if gallery.config.exit_after => {
                // The runtime ships en and zh-CN only: right-to-left is not available,
                // which is a note, not a failure.
                let note = Record::summary("rtl", "RTL", true, format!("not available: {error}"));
                println!("{}", note.line());
                std::process::exit(0);
            }
            Err(failure) => {
                let error = failure.to_string();
                if gallery.config.exit_after {
                    ramflux_desktop_sdk::local::app_log::warn(&error);
                    std::process::exit(1);
                }
                gallery.error = Some(error);
            }
        }
        if gallery.config.exit_after {
            gallery.start();
        }
        gallery
    }

    /// Runs checks A and C (no frame needed). B reads the next drawn frame.
    fn start(&mut self) {
        let Some(trust) = &self.trust else {
            if self.config.exit_after {
                std::process::exit(1);
            }
            return;
        };
        let cases: Vec<Case> = self.entries.iter().map(|e| e.case.clone()).collect();
        let mut report = Report::default();
        self.initial = conformance::run_ac(trust, &cases, &mut report);
        self.report = report;
        self.summary = None;
        trace::clear();
        self.phase = Phase::Requested;
    }

    /// Check B and the kind coverage, from the frame that drew every card.
    fn finish(&mut self) {
        let mut report = std::mem::take(&mut self.report);
        let mut kinds = BTreeSet::new();
        for entry in &self.entries {
            let drawn = trace::take(&entry.case.id);
            match self.initial.get(&entry.case.id) {
                Some(held) => conformance::check_b(&entry.case, held, &drawn, &mut report),
                None if !entry.case.refused => report.records.push(conformance::Record::summary(
                    &entry.case.id,
                    "B",
                    false,
                    "not run: the fixture did not open".to_owned(),
                )),
                None => {}
            }
            kinds.extend(drawn.iter().filter(|d| d.known).map(|d| d.kind.clone()));
        }
        let total = trace::KINDS.len();
        let covered = trace::KINDS.iter().filter(|kind| kinds.contains(**kind)).count();
        report.records.push(Record::summary(
            "kinds",
            "B",
            covered == total,
            format!("{covered}/{total} kinds drawn"),
        ));
        if let Err(error) = report.write(&std::env::temp_dir()) {
            ramflux_desktop_sdk::local::app_log::warn(&format!(
                "cannot write the conformance records: {error}"
            ));
        }
        let [a, b, c] = ["A", "B", "C"].map(|check| {
            let (pass, total) = report.count(check);
            format!("{pass}/{total}")
        });
        self.summary =
            Some(tr_args("card_gallery.summary", &[("a", &a), ("b", &b), ("c", &c)]).to_string());
        let pass = report.records.iter().all(|r| r.pass);
        self.report = report;
        self.phase = Phase::Done;
        if self.config.exit_after {
            std::process::exit(if pass { 0 } else { 1 });
        }
    }

    /// A host event from a card of the gallery. The card gets a live session
    /// (the others give theirs up), then the event goes to the runtime.
    fn on_event(&mut self, index: usize, kind: UiEventKind, cx: &mut Context<Self>) {
        let UiEventKind::CardInteracted { key, event, .. } = kind else {
            if let Some(entry) = self.entries.get_mut(index) {
                entry.note = Some(tr("card_gallery.note.slot").to_string());
            }
            cx.notify();
            return;
        };
        for (position, entry) in self.entries.iter_mut().enumerate() {
            if position != index {
                entry.opened = None;
            }
        }
        let Some(trust) = &self.trust else { return };
        let presentation = self.config.presentation();
        let Some(entry) = self.entries.get_mut(index) else { return };
        if entry.opened.is_none() {
            match conformance::open(trust, &entry.case, &presentation) {
                Ok(opened) => entry.opened = Some(opened),
                Err(error) => {
                    entry.note = Some(error);
                    cx.notify();
                    return;
                }
            }
        }
        let Some(opened) = entry.opened.as_mut() else { return };
        let label = event_label(&event);
        entry.note = conformance::send(opened, &key, &event, label, &mut Vec::new()).err();
        entry.shown = Some(opened.held.clone());
        cx.notify();
    }
}

/// Why the gallery cannot open.
enum Failure {
    /// The trust package has no presentation for the requested locale and theme.
    Presentation(String),
    /// The fixtures or the trust package are unusable.
    Fixtures(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Presentation(text) | Self::Fixtures(text) => f.write_str(text),
        }
    }
}

/// The trust package and the cases of the presentation.
fn load(presentation: &str) -> Result<(Trust, Vec<Case>), Failure> {
    let dir = std::path::Path::new(fixtures::DIR);
    fixtures::verify(dir).map_err(Failure::Fixtures)?;
    let trust = fixtures::trust(dir).map_err(Failure::Fixtures)?;
    if !trust.presentations.contains_key(presentation) {
        return Err(Failure::Presentation(format!(
            "no presentation {presentation} in the trust package (the runtime ships en and zh-CN)"
        )));
    }
    let cases = fixtures::cases(dir).map_err(Failure::Fixtures)?;
    Ok((trust, cases))
}

/// The entry of one fixture: its first tree, and the text fields' drafts.
fn entry(
    case: Case,
    trust: &Trust,
    config: &GalleryConfig,
    window: &mut Window,
    cx: &mut App,
) -> Entry {
    // The session is dropped at once: only the tree is kept for drawing.
    let shown = conformance::open(trust, &case, &config.presentation()).ok().map(|o| o.held);
    let mut fields = BTreeMap::new();
    if let Some(tree) = &shown {
        for node in tree.nodes() {
            if node.kind == "TextField" || node.kind == "DateTimeInput" {
                let text = node.attrs.get("value").and_then(|v| v.as_str()).unwrap_or_default();
                let text = text.to_owned();
                fields.insert(
                    node.key.clone(),
                    cx.new(|cx| InputState::new(window, cx).default_value(text)),
                );
            }
        }
    }
    Entry {
        case,
        shown,
        opened: None,
        inputs: Rc::new(fields),
        local: Rc::new(RefCell::new(CardLocal::default())),
        note: None,
    }
}

/// The record the card body draws, from a held tree.
fn card_record(tree: &CardTree) -> Option<CardRecord> {
    let session = tree.token_for(tree.root()?)?.session;
    Some(CardRecord {
        session,
        tree: Some(tree.clone()),
        status: CardStatus::Live,
        link: None,
        refusal: None,
        draft_discarded: false,
        receipt: None,
        submit_failed: false,
        chain: vec![tree.stamp()],
    })
}

/// The label of a host event, for the note line.
const fn event_label(event: &CardInput) -> &'static str {
    match event {
        CardInput::Input { .. } => "input",
        CardInput::Activate => "activate",
        CardInput::SelectTab { .. } => "select_tab",
        CardInput::ModalOpen => "modal_open",
        CardInput::ModalClose => "modal_close",
    }
}

impl Render for CardGallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.phase == Phase::Requested {
            // The callback runs after this frame, which draws every card.
            self.phase = Phase::Drawing;
            let gallery = cx.weak_entity();
            window.on_next_frame(move |_, cx| {
                let _ = gallery.update(cx, |this, _| this.finish());
            });
        }
        let running = matches!(self.phase, Phase::Requested | Phase::Drawing);
        let run = Button::new("card-gallery-run")
            .small()
            .design_primary(cx)
            .design_disabled(running || self.trust.is_none(), cx)
            .design_label(tr("card_gallery.run"))
            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                this.start();
                cx.notify();
            }));
        let theme = cx.theme();
        let count = self.entries.len().to_string();
        let header = match (&self.error, &self.summary) {
            (Some(error), _) => {
                tr_args("card_gallery.error", &[("error", error.as_str())]).to_string()
            }
            (None, Some(summary)) => summary.clone(),
            (None, None) => tr_args("card_gallery.title", &[("count", count.as_str())]).to_string(),
        };
        let mut list = v_flex().gap(px(16.)).p(px(16.));
        for (index, entry) in self.entries.iter().enumerate() {
            let sink = {
                let weak = cx.weak_entity();
                EventSink::root(
                    move |event, _window, cx| {
                        let kind = event.kind;
                        let _ = weak.update(cx, |this, cx| this.on_event(index, kind, cx));
                    },
                    |_, _, _| {},
                )
            };
            let record = entry.shown.as_ref().and_then(card_record);
            let snap = CardSnapshot {
                surface_id: SurfaceId::from(entry.case.id.as_str()),
                hash: None,
                version: 1,
                fallback: SharedString::from(entry.case.id.clone()),
                actions: Vec::new(),
                state: Some(SurfaceState::Open),
                record,
                author: SharedString::from("gallery"),
            };
            let note = entry.note.clone().unwrap_or_default();
            list = list.child(
                v_flex()
                    .gap(px(6.))
                    .child(div().text_color(theme.muted_foreground).child(entry.case.id.clone()))
                    .child(CardBody::new(snap, sink, entry.inputs.clone(), entry.local.clone()))
                    .child(div().text_color(theme.danger).child(note)),
            );
        }
        v_flex()
            .size_full()
            .child(div().p(px(16.)).child(header).child(run))
            .child(div().id("card-gallery-list").size_full().overflow_y_scroll().child(list))
    }
}
