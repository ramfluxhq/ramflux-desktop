// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The ceremony screens. Each screen is one function of the ceremony the
//! store reports; every message a person must not miss is a node with a
//! role (status or alert) whose name is its text, so assistive technology
//! and the tests read the same words the screen shows.

use gpui_kit::TestSupportExt as _;
use gpui_kit::component::StyledExt as _;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, IntoElement, Render, Role, SharedString, Window, div, rems,
};
use ramflux_desktop_core::{
    CHALLENGE_WORDS, Ceremony, CeremonyAccount, CeremonyFailure, CeremonyPhase, Networking,
    PHRASE_WORDS,
};

use super::{CeremonyPanel, Hint};
use crate::i18n::{tr, tr_args};

/// The ids of the three word inputs.
const WORD_IDS: [&str; CHALLENGE_WORDS] = ["ceremony-word-1", "ceremony-word-2", "ceremony-word-3"];

/// The words in a row of the grid.
const GRID_COLUMNS: usize = 4;

/// A node that announces `text` with `role`, which `child` shows. A `div`
/// with no role is not in the accessibility tree and the alert's own text
/// is not exposed, so this node carries the message.
fn announced(
    id: &'static str,
    role: Role,
    text: SharedString,
    child: impl IntoElement,
) -> AnyElement {
    div().id(id).role(role).aria_label(text).test_support().child(child).into_any_element()
}

fn warning(id: &'static str, key: &str) -> AnyElement {
    let text = tr(key);
    announced(
        id,
        Role::Status,
        text.clone(),
        Alert::warning(SharedString::from(format!("{id}-alert")), text).small(),
    )
}

fn failure(id: &'static str, key: &str) -> AnyElement {
    let text = tr(key);
    announced(
        id,
        Role::Alert,
        text.clone(),
        Alert::error(SharedString::from(format!("{id}-alert")), text).small(),
    )
}

fn muted(text: SharedString, cx: &App) -> AnyElement {
    div().text_sm().text_color(cx.theme().muted_foreground).child(text).into_any_element()
}

fn heading(key: &str) -> AnyElement {
    div().text_lg().font_semibold().child(tr(key)).into_any_element()
}

fn clock(remaining_ms: u64) -> String {
    let seconds = remaining_ms.div_ceil(1_000);
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

impl CeremonyPanel {
    fn cancel_button(&self, key: &str, cx: &mut Context<Self>) -> Button {
        Button::new("ceremony-cancel")
            .outline()
            .label(tr(key))
            .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx)))
    }

    /// The time left, as a timer node (a timer is not announced on every
    /// tick); expiry is its own alert, drawn by [`Self::expired_screen`].
    fn countdown(&self, ceremony: &Ceremony, cx: &App) -> AnyElement {
        let left = clock(ceremony.remaining_ms(self.now(cx)));
        let text = tr_args("ceremony.expires_in", &[("time", &left)]);
        div()
            .id("ceremony-countdown")
            .role(Role::Timer)
            .aria_label(text.clone())
            .test_support()
            .child(muted(text, cx))
            .into_any_element()
    }

    fn name_screen(&self, cx: &mut Context<Self>) -> AnyElement {
        let waiting = self.awaiting_begin;
        let typed = !self.name.read(cx).value().trim().is_empty();
        v_flex()
            .id("ceremony-name")
            .test_support()
            .gap_3()
            .child(heading("ceremony.title"))
            .child(muted(tr("ceremony.name.help"), cx))
            .child(
                v_flex().gap_1().child(muted(tr("ceremony.name.label"), cx)).child(
                    Input::new(&self.name)
                        .id("ceremony-name-input")
                        .aria_label(tr("ceremony.name.label")),
                ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("ceremony-continue")
                            .primary()
                            .label(tr("ceremony.continue"))
                            .loading(waiting)
                            .disabled(waiting || !typed)
                            .on_click(cx.listener(|this, _, window, cx| this.begin(window, cx))),
                    )
                    .child(self.cancel_button("ceremony.back", cx)),
            )
            .into_any_element()
    }

    fn ready_screen(&self, ceremony: &Ceremony, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .id("ceremony-ready")
            .test_support()
            .gap_3()
            .child(heading("ceremony.ready.title"))
            .child(warning("ceremony-ready-offline", "ceremony.ready.offline"))
            .child(warning("ceremony-ready-capture", "ceremony.ready.capture"))
            .child(warning("ceremony-ready-once", "ceremony.ready.once"))
            .child(self.countdown(ceremony, cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("ceremony-show")
                            .primary()
                            .label(tr("ceremony.ready.show"))
                            .on_click(cx.listener(|this, _, window, cx| this.show(window, cx))),
                    )
                    .child(self.cancel_button("ceremony.cancel", cx)),
            )
            .into_any_element()
    }

    /// The 24 words, numbered, in rows of four. Covered words are not drawn.
    fn grid(&self, cx: &App) -> AnyElement {
        let covered = self.held.covered;
        let phrase = self.held.phrase.as_ref();
        let muted_color = cx.theme().muted_foreground;
        let cells: Vec<AnyElement> = (1..=PHRASE_WORDS)
            .map(|number| {
                let shown = phrase.and_then(|p| p.word(number)).filter(|_| !covered);
                let label = match shown {
                    Some(word) => tr_args(
                        "ceremony.show.cell",
                        &[("number", &number.to_string()), ("word", word)],
                    ),
                    None => {
                        tr_args("ceremony.show.cell_hidden", &[("number", &number.to_string())])
                    }
                };
                h_flex()
                    .id(SharedString::from(format!("ceremony-word-cell-{number}")))
                    .role(Role::ListItem)
                    .aria_label(label)
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius)
                    .child(
                        div()
                            .w(rems(1.5))
                            .text_right()
                            .text_xs()
                            .text_color(muted_color)
                            .child(number.to_string()),
                    )
                    .child(div().font_semibold().child(match shown {
                        Some(word) => SharedString::from(word.to_owned()),
                        None => {
                            SharedString::from("\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}")
                        }
                    }))
                    .into_any_element()
            })
            .collect();
        let mut rows = Vec::new();
        let mut cells = cells.into_iter();
        while let Some(first) = cells.next() {
            let row: Vec<AnyElement> =
                std::iter::once(first).chain(cells.by_ref().take(GRID_COLUMNS - 1)).collect();
            rows.push(h_flex().gap_2().children(row));
        }
        v_flex()
            .id("ceremony-grid")
            .role(Role::List)
            .aria_label(tr("ceremony.show.grid"))
            .test_support()
            .gap_2()
            .children(rows)
            .into_any_element()
    }

    fn phrase_screen(&self, ceremony: &Ceremony, cx: &mut Context<Self>) -> AnyElement {
        let screen = v_flex()
            .id("ceremony-phrase")
            .test_support()
            .gap_3()
            .child(heading("ceremony.show.title"));
        if !self.held.holds_phrase() {
            // The daemon hands the phrase out once; if it is not here, it
            // cannot be asked for again.
            return screen
                .child(failure("ceremony-phrase-missing", "ceremony.show.missing"))
                .child(h_flex().child(self.cancel_button("ceremony.cancel", cx)))
                .into_any_element();
        }
        let covered = self.held.covered;
        screen
            .child(warning("ceremony-phrase-warning", "ceremony.show.warning"))
            .child(self.grid(cx))
            .when(covered, |screen| {
                screen.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(announced(
                            "ceremony-covered",
                            Role::Status,
                            tr("ceremony.show.covered"),
                            muted(tr("ceremony.show.covered"), cx),
                        ))
                        .child(
                            Button::new("ceremony-reveal")
                                .outline()
                                .label(tr("ceremony.show.reveal"))
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.reveal(window, cx)),
                                ),
                        ),
                )
            })
            .child(muted(tr("ceremony.show.no_copy"), cx))
            .child(self.countdown(ceremony, cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("ceremony-hide")
                            .primary()
                            .label(tr("ceremony.show.done"))
                            .on_click(cx.listener(|this, _, window, cx| this.hide(window, cx))),
                    )
                    .child(self.cancel_button("ceremony.cancel", cx)),
            )
            .into_any_element()
    }

    fn hint(&self, index: usize, typed: &str, cx: &mut Context<Self>) -> AnyElement {
        let muted_color = cx.theme().muted_foreground;
        match Hint::of(typed) {
            Hint::Nothing => div().into_any_element(),
            Hint::Reads(word) => {
                let text = tr_args("ceremony.confirm.reads", &[("word", word)]);
                announced(
                    ["ceremony-hint-1", "ceremony-hint-2", "ceremony-hint-3"][index],
                    Role::Status,
                    text.clone(),
                    div().text_sm().text_color(muted_color).child(text),
                )
            }
            Hint::Suggest(candidates) => h_flex()
                .id(["ceremony-suggest-1", "ceremony-suggest-2", "ceremony-suggest-3"][index])
                .test_support()
                .flex_wrap()
                .gap_1()
                .children(candidates.into_iter().map(|word| {
                    Button::new(SharedString::from(format!(
                        "ceremony-suggest-{}-{word}",
                        index + 1
                    )))
                    .xsmall()
                    .ghost()
                    .label(SharedString::from(word))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.words[index].update(cx, |input, cx| input.set_value(word, window, cx));
                        cx.notify();
                    }))
                }))
                .into_any_element(),
            Hint::Unknown => failure_line(index, "ceremony.confirm.unknown", cx),
            Hint::NotOneWord => failure_line(index, "ceremony.confirm.one_word", cx),
        }
    }

    fn confirm_screen(
        &self,
        ceremony: &Ceremony,
        positions: [u8; 3],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ready = self.can_confirm(cx);
        let fields: Vec<AnyElement> = positions
            .iter()
            .enumerate()
            .map(|(index, position)| {
                let label = tr_args("ceremony.confirm.word", &[("number", &position.to_string())]);
                let typed = self.words[index].read(cx).value().to_string();
                v_flex()
                    .gap_1()
                    .child(muted(label.clone(), cx))
                    .child(Input::new(&self.words[index]).id(WORD_IDS[index]).aria_label(label))
                    .child(self.hint(index, &typed, cx))
                    .into_any_element()
            })
            .collect();
        v_flex()
            .id("ceremony-confirm")
            .test_support()
            .gap_3()
            .child(heading("ceremony.confirm.title"))
            .child(warning("ceremony-confirm-once", "ceremony.confirm.once"))
            .children(fields)
            .child(self.countdown(ceremony, cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("ceremony-confirm-submit")
                            .primary()
                            .label(tr("ceremony.confirm.submit"))
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
                    )
                    .child(self.cancel_button("ceremony.cancel", cx)),
            )
            .into_any_element()
    }

    fn done_screen(&self, account: &CeremonyAccount, cx: &mut Context<Self>) -> AnyElement {
        let live = account.networking == Networking::Live;
        let (state, state_text, text_key) = if live {
            ("ceremony.done.live_state", "ceremony.done.live_state_text", "ceremony.done.live_text")
        } else {
            ("ceremony.done.state", "ceremony.done.state_text", "ceremony.done.text")
        };
        let text = tr_args(text_key, &[("name", &account.display_name)]);
        v_flex()
            .id("ceremony-done")
            .test_support()
            .gap_3()
            .child(heading("ceremony.done.title"))
            .child(announced(
                "ceremony-state",
                Role::Status,
                tr(state_text),
                h_flex().child(if live {
                    Tag::success().child(tr(state))
                } else {
                    Tag::warning().child(tr(state))
                }),
            ))
            .child(announced(
                "ceremony-done-text",
                Role::Status,
                text.clone(),
                Alert::success("ceremony-done-alert", text).small(),
            ))
            .child(
                h_flex().child(
                    Button::new("ceremony-done-close")
                        .primary()
                        .label(tr("ceremony.done.close"))
                        .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                ),
            )
            .into_any_element()
    }

    /// The ceremony ended without an account.
    fn ended_screen(&self, key: &'static str, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .id("ceremony-ended")
            .test_support()
            .gap_3()
            .child(failure("ceremony-ended-text", key))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("ceremony-restart")
                            .primary()
                            .label(tr("ceremony.restart"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.start_over(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("ceremony-close")
                            .outline()
                            .label(tr("ceremony.close"))
                            .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
                    ),
            )
            .into_any_element()
    }
}

/// An error line under a word input.
fn failure_line(index: usize, key: &str, cx: &App) -> AnyElement {
    let text = tr(key);
    announced(
        ["ceremony-hint-1", "ceremony-hint-2", "ceremony-hint-3"][index],
        Role::Alert,
        text.clone(),
        div().text_sm().text_color(cx.theme().danger).child(text),
    )
}

/// The message of a ceremony that ended without an account.
fn failure_key(failure: CeremonyFailure) -> &'static str {
    match failure {
        CeremonyFailure::Disabled => "ceremony.ended.disabled",
        CeremonyFailure::ConfirmationFailed => "ceremony.ended.mismatch",
        CeremonyFailure::VaultUnavailable => "ceremony.ended.vault",
        CeremonyFailure::Expired => "ceremony.ended.expired",
        CeremonyFailure::Invalid => "ceremony.ended.invalid",
    }
}

impl Render for CeremonyPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ceremony = self.ceremony(cx);
        let body = match ceremony {
            // The five minutes passed: the expiry stays, whatever the
            // ceremony became since it was let go.
            Some(_) if self.expired => self.ended_screen("ceremony.ended.expired", cx),
            None => self.name_screen(cx),
            Some(ceremony) => match ceremony.phase.clone() {
                CeremonyPhase::Prepared => self.ready_screen(&ceremony, cx),
                CeremonyPhase::PhraseReady => self.phrase_screen(&ceremony, cx),
                CeremonyPhase::Challenged { positions } => {
                    self.confirm_screen(&ceremony, positions, cx)
                }
                CeremonyPhase::Created(account) => self.done_screen(&account, cx),
                CeremonyPhase::Failed(failure) => self.ended_screen(failure_key(failure), cx),
                CeremonyPhase::Cancelled => self.ended_screen("ceremony.ended.cancelled", cx),
            },
        };
        div().id("ceremony").test_support().w_full().child(body)
    }
}
