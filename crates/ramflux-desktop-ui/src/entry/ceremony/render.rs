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
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Context, IntoElement, Render, Role, SharedString, Styled, Window, div, rems,
};
use ramflux_desktop_core::{
    CHALLENGE_WORDS, Ceremony, CeremonyAccount, CeremonyFailure, CeremonyPhase, Networking,
    PHRASE_WORDS,
};

use super::{CeremonyPanel, Hint, Pending, Screen};
use crate::actions::CEREMONY_CONTEXT;
use crate::entry::parts::{announced, clock, muted, ringed, warning};
use crate::i18n::{tr, tr_args};
use crate::settings::backup::text::Strength;

/// The ids of the three word inputs.
const WORD_IDS: [&str; CHALLENGE_WORDS] = ["ceremony-word-1", "ceremony-word-2", "ceremony-word-3"];

/// The narrowest a word cell may be (the number, eight letters of the
/// longest word at semibold, and the padding): a row that cannot fit four
/// wraps to three, so a word is never clipped.
const CELL_MIN_REMS: f32 = 8.5;

impl CeremonyPanel {
    /// The screen's heading. On the screens where focus lands on it (the
    /// reader hears what the screen is), it takes focus and draws the ring.
    fn heading(&self, key: &str, window: &Window, cx: &App) -> AnyElement {
        let text = tr(key);
        let focused = self.heading_focus.is_focused(window);
        ringed(
            div()
                .id("ceremony-heading")
                .role(Role::Heading)
                .aria_label(text.clone())
                .track_focus(&self.heading_focus)
                .test_support()
                .px_1()
                .rounded(cx.theme().radius)
                .text_lg()
                .font_semibold()
                .child(text),
            focused,
            window,
            cx,
        )
        .into_any_element()
    }

    /// An error that takes focus when its screen appears (the screen has
    /// nothing to type into): the reader hears the failure itself.
    fn focusable_failure(
        &self,
        id: &'static str,
        key: &str,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let text = tr(key);
        self.alert_id.set(id);
        ringed(
            div()
                .id(id)
                .role(Role::Alert)
                .aria_label(text.clone())
                .track_focus(&self.alert_focus)
                .test_support()
                .p_1()
                .rounded(cx.theme().radius)
                .child(Alert::error(SharedString::from(format!("{id}-alert")), text).small()),
            self.alert_focus.is_focused(window),
            window,
            cx,
        )
        .into_any_element()
    }

    fn cancel_button(&self, key: &str, cx: &mut Context<Self>) -> Button {
        let waiting = self.pending == Some(Pending::Cancel);
        Button::new("ceremony-cancel")
            .outline()
            .label(tr(key))
            .loading(waiting)
            .disabled(waiting)
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

    /// A request-sending button: busy while its own answer is awaited and
    /// off while any answer is.
    fn sending(&self, button: Button, own: Pending) -> Button {
        button.loading(self.pending == Some(own)).disabled(self.pending.is_some())
    }

    fn name_screen(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let typed = !self.name.read(cx).value().trim().is_empty();
        v_flex()
            .id("ceremony-name")
            .test_support()
            .gap_3()
            .child(self.heading("ceremony.title", window, cx))
            .child(muted(tr("ceremony.name.help"), cx))
            .child(match &self.node {
                Some(node) => announced(
                    "ceremony-node",
                    Role::Status,
                    tr_args("ceremony.name.node", &[("node", node.as_str())]),
                    muted(tr_args("ceremony.name.node", &[("node", node.as_str())]), cx),
                ),
                None => announced(
                    "ceremony-no-node",
                    Role::Alert,
                    tr("ceremony.name.no_node"),
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(tr("ceremony.name.no_node")),
                ),
            })
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
                        self.sending(
                            Button::new("ceremony-continue")
                                .primary()
                                .label(tr("ceremony.continue"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.begin(window, cx);
                                })),
                            Pending::Begin,
                        )
                        .loading(
                            matches!(self.pending, Some(Pending::Begin | Pending::Node))
                                || self.is_abandoned(),
                        )
                        .disabled(
                            self.pending.is_some()
                                || self.is_abandoned()
                                || !typed
                                || self.node.is_none(),
                        ),
                    )
                    .child(self.cancel_button("ceremony.back", cx)),
            )
            .into_any_element()
    }

    fn ready_screen(
        &self,
        ceremony: &Ceremony,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .id("ceremony-ready")
            .test_support()
            .gap_3()
            .child(self.heading("ceremony.ready.title", window, cx))
            // One block: what to do and what not to do, before any word.
            .child(warning("ceremony-ready-warning", "ceremony.ready.warning"))
            .child(self.countdown(ceremony, cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        self.sending(
                            Button::new("ceremony-show")
                                .primary()
                                .label(tr("ceremony.ready.show"))
                                .on_click(cx.listener(|this, _, window, cx| this.show(window, cx))),
                            Pending::Show,
                        ),
                    )
                    .child(self.cancel_button("ceremony.cancel", cx)),
            )
            .into_any_element()
    }

    /// The 24 words, numbered. Covered words are not drawn. The cells wrap
    /// instead of shrinking below the width a word needs.
    fn grid(&self, window: &Window, cx: &App) -> AnyElement {
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
                    .w(rems(CELL_MIN_REMS))
                    .flex_grow(1.0)
                    .flex_shrink_0()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius)
                    .child(
                        div()
                            .w(rems(1.5))
                            .flex_shrink_0()
                            .text_right()
                            .text_xs()
                            .text_color(muted_color)
                            .child(number.to_string()),
                    )
                    .child(div().whitespace_nowrap().font_semibold().child(match shown {
                        Some(word) => SharedString::from(word.to_owned()),
                        None => {
                            SharedString::from("\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}")
                        }
                    }))
                    .into_any_element()
            })
            .collect();
        ringed(
            h_flex()
                .id("ceremony-grid")
                .track_focus(&self.grid_focus)
                .role(Role::List)
                .aria_label(tr("ceremony.show.grid"))
                .test_support()
                // Room for the ring: it sits outside the edge.
                .p_1()
                .rounded(cx.theme().radius)
                .flex_wrap()
                .gap_2()
                .children(cells),
            self.grid_focus.is_focused(window),
            window,
            cx,
        )
        .into_any_element()
    }

    fn phrase_screen(
        &self,
        ceremony: &Ceremony,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let screen = v_flex().id("ceremony-phrase").test_support().gap_3().child(self.heading(
            "ceremony.show.title",
            window,
            cx,
        ));
        if !self.held.holds_phrase() {
            // The hide was sent: the daemon is choosing the words. Nothing
            // is wrong, so nothing alerts.
            if self.pending == Some(Pending::Hide) {
                return self.hiding_screen(window, cx);
            }
            // The daemon handed the phrase out once; if it is not here, it
            // cannot be asked for again.
            return screen
                .child(self.focusable_failure(
                    "ceremony-phrase-missing",
                    "ceremony.show.missing",
                    window,
                    cx,
                ))
                .child(h_flex().child(self.cancel_button("ceremony.cancel", cx)))
                .into_any_element();
        }
        let covered = self.held.covered;
        screen
            .child(warning("ceremony-phrase-warning", "ceremony.show.warning"))
            .child(self.grid(window, cx))
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
            .child(announced(
                "ceremony-no-copy",
                Role::Status,
                tr("ceremony.show.no_copy"),
                muted(tr("ceremony.show.no_copy"), cx),
            ))
            .child(self.countdown(ceremony, cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        self.sending(
                            Button::new("ceremony-hide")
                                .primary()
                                .label(tr("ceremony.show.done"))
                                .on_click(cx.listener(|this, _, window, cx| this.hide(window, cx))),
                            Pending::Hide,
                        ),
                    )
                    .child(self.cancel_button("ceremony.cancel", cx)),
            )
            .into_any_element()
    }

    /// Waiting for the daemon to choose the words to ask.
    fn hiding_screen(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .id("ceremony-hiding")
            .test_support()
            .gap_3()
            .child(self.heading("ceremony.show.title", window, cx))
            .child(ringed(
                div()
                    .id("ceremony-hiding-text")
                    .role(Role::Status)
                    .aria_label(tr("ceremony.hiding"))
                    .track_focus(&self.status_focus)
                    .test_support()
                    .p_1()
                    .rounded(cx.theme().radius)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(Spinner::new().small())
                            .child(tr("ceremony.hiding")),
                    ),
                self.status_focus.is_focused(window),
                window,
                cx,
            ))
            .child(h_flex().child(self.cancel_button("ceremony.cancel", cx)))
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

    fn words_screen(
        &self,
        ceremony: &Ceremony,
        positions: [u8; 3],
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let ready = self.can_continue(cx);
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
            .child(self.heading("ceremony.confirm.title", window, cx))
            .child(warning("ceremony-confirm-once", "ceremony.confirm.once"))
            .children(fields)
            .child(self.countdown(ceremony, cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("ceremony-next")
                            .primary()
                            .label(tr("ceremony.confirm.next"))
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, window, cx| this.next(window, cx))),
                    )
                    .child(self.cancel_button("ceremony.cancel", cx)),
            )
            .into_any_element()
    }

    /// The passphrase that unlocks the account on this device, typed twice
    /// under the rules of the backup passphrase. Sent with the words, once.
    fn passphrase_screen(
        &self,
        ceremony: &Ceremony,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // One materialized copy of each input per frame (the input hands
        // out nothing borrowed), compared and rated once: nothing below
        // copies them again.
        let typed = self.passphrase.read(cx).value();
        let repeated = self.repeat.read(cx).value();
        let strength = Strength::of(&typed);
        let mismatch = !repeated.is_empty() && repeated != typed;
        let ready = strength != Strength::TooShort && repeated == typed;
        let hint_color = match strength {
            Strength::TooShort | Strength::Weak => cx.theme().warning,
            Strength::Fair | Strength::Strong => cx.theme().muted_foreground,
        };
        let field = |key: &'static str,
                     input: &gpui_kit::Entity<gpui_kit::component::input::InputState>,
                     id: &'static str| {
            v_flex()
                .gap_1()
                .child(muted(tr(key), cx))
                .child(Input::new(input).id(id).aria_label(tr(key)).mask_toggle())
        };
        v_flex()
            .id("ceremony-passphrase")
            .test_support()
            .gap_3()
            .child(self.heading("ceremony.passphrase.title", window, cx))
            .child(muted(tr("ceremony.passphrase.help"), cx))
            .child(field(
                "ceremony.passphrase.label",
                &self.passphrase,
                "ceremony-passphrase-input",
            ))
            .child(field("ceremony.passphrase.repeat", &self.repeat, "ceremony-passphrase-repeat"))
            .child(
                v_flex()
                    .gap_1()
                    .text_sm()
                    .children((!typed.is_empty()).then(|| {
                        announced(
                            "ceremony-strength",
                            Role::Status,
                            strength.hint(),
                            div().text_color(hint_color).child(strength.hint()),
                        )
                    }))
                    .children(mismatch.then(|| {
                        announced(
                            "ceremony-mismatch",
                            Role::Alert,
                            tr("settings.backup.mismatch"),
                            div()
                                .text_color(cx.theme().danger)
                                .child(tr("settings.backup.mismatch")),
                        )
                    })),
            )
            .child(self.countdown(ceremony, cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        self.sending(
                            Button::new("ceremony-confirm-submit")
                                .primary()
                                .label(tr("ceremony.passphrase.submit"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm(window, cx);
                                })),
                            Pending::Confirm,
                        )
                        .disabled(self.pending.is_some() || !ready),
                    )
                    .child(self.cancel_button("ceremony.cancel", cx)),
            )
            .into_any_element()
    }

    fn done_screen(
        &self,
        account: &CeremonyAccount,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // The account list says whether it connected since (the daemon
        // registers it after it is made, and the page reads that again).
        let networking = self
            .store
            .read(cx)
            .entry_store()
            .account(&ramflux_desktop_core::AccountId::new(&account.local_account_id))
            .map_or(account.networking, |listed| listed.networking);
        let live = networking == Networking::Live;
        let (state, state_text, text_key) = if live {
            ("ceremony.done.live_state", "ceremony.done.live_state_text", "ceremony.done.live_text")
        } else {
            ("ceremony.done.state", "ceremony.done.state_text", "ceremony.done.text")
        };
        let node = self
            .node
            .as_ref()
            .map_or_else(|| tr("ceremony.done.node_unknown"), |n| n.as_str().to_owned().into());
        let text = tr_args(text_key, &[("name", &account.display_name), ("node", &node)]);
        v_flex()
            .id("ceremony-done")
            .test_support()
            .gap_3()
            .child(self.heading("ceremony.done.title", window, cx))
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
    fn ended_screen(
        &self,
        key: &'static str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .id("ceremony-ended")
            .test_support()
            .gap_3()
            .child(self.focusable_failure("ceremony-ended-text", key, window, cx))
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

    /// Escape asked: cancelling erases the phrase and creates nothing.
    fn cancel_question(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        ringed(
            v_flex()
                .id("ceremony-cancel-question")
                .track_focus(&self.question_focus)
                .role(Role::Status)
                .aria_label(tr("ceremony.cancel_question"))
                .test_support()
                .gap_2()
                .p_3()
                .border_1()
                .border_color(cx.theme().border)
                .rounded(cx.theme().radius),
            self.question_focus.is_focused(window),
            window,
            cx,
        )
        .child(div().text_sm().child(tr("ceremony.cancel_question")))
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("ceremony-cancel-yes")
                        .danger()
                        .label(tr("ceremony.cancel_yes"))
                        .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
                )
                .child(
                    Button::new("ceremony-cancel-no")
                        .outline()
                        .label(tr("ceremony.cancel_no"))
                        .on_click(cx.listener(|this, _, window, cx| this.keep_going(window, cx))),
                ),
        )
        .into_any_element()
    }

    /// Moves keyboard focus when a screen is first drawn: to the input to
    /// type in; to the word list when the words appear and to the pending
    /// line while three words are chosen (reading comes first); otherwise
    /// to the panel itself. Never to a button: the first press after a
    /// screen appears must not be able to do something that cannot be
    /// undone (hide the words, cancel). Tab reaches the buttons.
    fn focus_screen(&mut self, screen: Screen, window: &mut Window, cx: &mut Context<Self>) {
        if self.focused_screen == Some(screen) {
            return;
        }
        self.focused_screen = Some(screen);
        let panel = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            panel
                .update(cx, |panel, cx| {
                    let input = match screen {
                        Screen::Name => Some(panel.name.clone()),
                        Screen::Words => Some(panel.words[0].clone()),
                        Screen::Passphrase => Some(panel.passphrase.clone()),
                        _ => None,
                    };
                    match (input, screen) {
                        (Some(input), _) => input.update(cx, |input, cx| input.focus(window, cx)),
                        (None, Screen::Phrase) => window.focus(&panel.grid_focus, cx),
                        (None, Screen::Hiding) => window.focus(&panel.status_focus, cx),
                        // The screens with nothing to type into or read in
                        // order: the heading says where the reader is; a
                        // failure says itself.
                        (None, Screen::Ready | Screen::Done) => {
                            window.focus(&panel.heading_focus, cx);
                        }
                        (None, Screen::PhraseMissing | Screen::Ended) => {
                            window.focus(&panel.alert_focus, cx);
                        }
                        (None, _) => window.focus(&panel.focus, cx),
                    }
                })
                .ok();
        });
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ceremony = self.ceremony(cx);
        let (screen, body) = match ceremony {
            // The five minutes passed: the expiry stays, whatever the
            // ceremony became since it was let go.
            Some(_) if self.expired => {
                (Screen::Ended, self.ended_screen("ceremony.ended.expired", window, cx))
            }
            None => (Screen::Name, self.name_screen(window, cx)),
            Some(ceremony) => match ceremony.phase.clone() {
                CeremonyPhase::Prepared => {
                    (Screen::Ready, self.ready_screen(&ceremony, window, cx))
                }
                CeremonyPhase::PhraseReady => {
                    let screen = if self.held.holds_phrase() {
                        Screen::Phrase
                    } else if self.pending == Some(Pending::Hide) {
                        Screen::Hiding
                    } else {
                        Screen::PhraseMissing
                    };
                    (screen, self.phrase_screen(&ceremony, window, cx))
                }
                CeremonyPhase::Challenged { positions } if self.typed.is_none() => {
                    (Screen::Words, self.words_screen(&ceremony, positions, window, cx))
                }
                CeremonyPhase::Challenged { .. } => {
                    (Screen::Passphrase, self.passphrase_screen(&ceremony, window, cx))
                }
                CeremonyPhase::Created(account) => {
                    (Screen::Done, self.done_screen(&account, window, cx))
                }
                CeremonyPhase::Failed(failure) => {
                    (Screen::Ended, self.ended_screen(failure_key(failure), window, cx))
                }
                CeremonyPhase::Cancelled => {
                    (Screen::Ended, self.ended_screen("ceremony.ended.cancelled", window, cx))
                }
            },
        };
        self.focus_screen(screen, window, cx);
        // Above the screen: it must be in view when Escape is pressed.
        let question = self.confirming_cancel.then(|| self.cancel_question(window, cx));
        v_flex()
            .id("ceremony")
            .key_context(CEREMONY_CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_escape))
            .test_support()
            .w_full()
            .gap_3()
            .children(question)
            .child(body)
    }
}
