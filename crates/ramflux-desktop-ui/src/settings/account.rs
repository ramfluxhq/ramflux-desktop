// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Settings · Account: the active account, lock (returns to entry),
//! passphrase rotation (two masked inputs handed to the root as
//! secrets), and the username claim (a name and the recovery phrase that
//! authorizes it, handed to the root as a secret). Masked inputs are cleared
//! the moment they are taken.

use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::setting::{SettingGroup, SettingItem, SettingPage};
use gpui_kit::component::switch::Switch;
use gpui_kit::prelude::*;
use gpui_kit::{App, Entity, SharedString, TestSupportExt as _, Window, div};
use ramflux_desktop_core::{
    Account, AccountId, AppLockCodeRefusal, AppLockState, IntentKind, LockState, SecretInput,
    Store, UiEventKind, UsernameClaim, methods,
};
use ramflux_desktop_navigation::direction::h_flex;
use ramflux_desktop_navigation::scale::space;

use super::{SectionContext, SettingsSection, design, note};
use crate::i18n::{self, tr, tr_args};
use crate::root::SUBMITTED;
use crate::style::DesignButton as _;

/// The masked inputs of the account section, owned by the settings page.
pub struct AccountInputs {
    /// Current passphrase (rotation).
    pub current: Entity<InputState>,
    /// New passphrase (rotation).
    pub next: Entity<InputState>,
    /// The username to claim (or change to), typed with or without `@`.
    pub username: Entity<InputState>,
    /// The recovery phrase that authorizes the username change.
    pub username_phrase: Entity<InputState>,
    /// Whether this computer has a username centre to claim against, read
    /// once when the settings page opens.
    pub username_configured: bool,
    /// Set for a moment after the username status was asked for again, so
    /// repeated presses do not repeat the read.
    pub refresh_cooldown: std::rc::Rc<std::cell::Cell<bool>>,
    /// The account code typed into the app lock sheet (masked).
    pub app_lock_code: Entity<InputState>,
    /// The last app lock wait the window saw, and when it saw it.
    pub code_clock: std::rc::Rc<std::cell::Cell<Option<(u64, std::time::Instant)>>>,
    /// Whether a refresh for the app lock countdown is already scheduled.
    pub code_ticking: std::rc::Rc<std::cell::Cell<bool>>,
}

/// How long the status refresh ignores further presses.
const REFRESH_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(2);

/// Clears every masked and typed input of the account section (leaving the
/// section, locking the account or switching accounts), the account code
/// included.
pub(super) fn clear_inputs(inputs: &AccountInputs, window: &mut Window, cx: &mut App) {
    for input in [&inputs.current, &inputs.next, &inputs.username, &inputs.username_phrase] {
        input.update(cx, |input, cx| input.set_value("", window, cx));
    }
    clear_code(&inputs.app_lock_code, window, cx);
}

/// Empties the account code input and masks it again (a code revealed once
/// is not shown, or copyable, in the next sheet).
pub(super) fn clear_code(code: &Entity<InputState>, window: &mut Window, cx: &mut App) {
    code.update(cx, |input, cx| {
        input.set_value("", window, cx);
        input.set_masked(true, window, cx);
    });
}

/// The account the section is about, and whether it is unlocked: the key of
/// what the typed inputs belong to.
pub(super) fn input_scope(context: &SectionContext, cx: &App) -> (Option<AccountId>, bool) {
    active_account(context, cx)
        .map_or((None, false), |a| (Some(a.id.clone()), a.lock_state == LockState::Unlocked))
}

pub(super) fn active_account(context: &SectionContext, cx: &App) -> Option<Account> {
    let handle = context.store.read(cx);
    let store = handle.store();
    handle
        .active_account()
        .and_then(|id| store.account(id))
        .or_else(|| store.accounts().find(|a| a.lock_state == LockState::Unlocked))
        .cloned()
}

/// The line under the username form: what the last claim or change of
/// `account` says, and whether the status can be read again (the claim is
/// still publishing and waits for a refresh).
fn claim_status(
    store: &Store,
    account: &AccountId,
) -> Option<(SharedString, Option<Option<String>>)> {
    let claim = store.username_claim(account)?;
    let refresh = match claim {
        UsernameClaim::StillPublishing { operation_id, .. } => Some(operation_id.clone()),
        _ => None,
    };
    let text = match claim {
        UsernameClaim::Publishing { .. } => tr("settings.account.username_publishing"),
        UsernameClaim::StillPublishing { .. } => tr("settings.account.username_unsettled"),
        UsernameClaim::Committed { .. } => tr("settings.account.username_committed"),
        UsernameClaim::Failed { code, retry_after_ms, .. } => match code.as_str() {
            "username_taken" => tr("username.error.taken"),
            "authentication_required" => tr("username.error.authentication"),
            "invalid_username" => tr("username.error.invalid"),
            "aborted" => tr("username.error.aborted"),
            _ => i18n::username_refusal(code, *retry_after_ms),
        },
    };
    Some((text, refresh))
}

fn take(input: &Entity<InputState>, window: &mut Window, cx: &mut App) -> SecretInput {
    let secret = SecretInput::new(input.read(cx).value().to_string());
    input.update(cx, |input, cx| input.set_value("", window, cx));
    secret
}

/// The account section.
pub fn page(context: &SectionContext, inputs: &AccountInputs, cx: &App) -> SettingPage {
    let account = active_account(context, cx);
    let app_lock = app_lock_item(context, account.as_ref().map(|a| a.id.clone()), inputs);
    let sink = context.sink(SettingsSection::Account);
    let username_configured = inputs.username_configured;
    // Design G01: the current account (its row with a "Current" chip, the
    // default node and device, Lock) and the passphrase change, each a
    // section of the 760 px column.
    let current = {
        let account = account.clone();
        let sink = sink.clone();
        let store = context.store.clone();
        SettingItem::render(move |_, _, cx: &mut App| {
            // The username the directory reports, or why it cannot be used
            // (a daemon without a provisioned directory refuses; claiming
            // and changing then wait for it).
            let username = match store.read(cx).store().username() {
                Some(ramflux_desktop_core::UsernameStatus::Known(Some(name))) => {
                    SharedString::from(format!("@{name}"))
                }
                Some(ramflux_desktop_core::UsernameStatus::Known(None)) => {
                    tr("settings.account.username_none")
                }
                // Why is for the log; the line says it in one sentence.
                Some(ramflux_desktop_core::UsernameStatus::Unavailable(_)) => {
                    tr("settings.account.username_unavailable")
                }
                None => tr("settings.account.username_unknown"),
            };
            let section =
                design::section("account-current", tr("settings.account.current_title"), cx);
            let Some(a) = account.clone() else {
                return section.child(note("settings.account.none", cx)).into_any_element();
            };
            let state = tr(match a.lock_state {
                LockState::Unlocked => "settings.account.unlocked",
                _ => "settings.account.locked",
            });
            let node = a
                .node_id
                .as_ref()
                .map_or_else(|| tr("settings.account.no_node"), |id| id.to_string().into());
            let device = a
                .device_id
                .clone()
                .map_or_else(|| tr("settings.account.no_device"), SharedString::from);
            let lock_id = a.id.clone();
            let lock = sink.clone();
            // The username rides on the account's own line (no taller page).
            let named = tr_args("settings.account.username_line", &[("username", &username)]);
            section
                .child(div().id("account-username").test_support().aria_label(username).child(
                    design::row(
                        ramflux_desktop_navigation::row::list_text(&a.label),
                        i18n::join(&i18n::join(&i18n::account_kind(a.kind), &state), &named),
                        Some(design::chip(tr("settings.account.current_chip"), cx)),
                        cx,
                    ),
                ))
                .child(design::row(
                    tr("settings.account.default_node"),
                    tr_args(
                        "settings.account.node_device",
                        &[("node", node.as_ref()), ("device", device.as_ref())],
                    ),
                    None,
                    cx,
                ))
                .child(
                    h_flex().child(
                        crate::style::secondary_button("settings-lock", cx)
                            .label(tr("settings.account.lock"))
                            .on_click(move |_, window, cx| {
                                lock.emit(
                                    UiEventKind::LockPressed { account_id: lock_id.clone() },
                                    window,
                                    cx,
                                )
                            }),
                    ),
                )
                .into_any_element()
        })
    };
    let rotation = {
        let current = inputs.current.clone();
        let next = inputs.next.clone();
        let root = context.root.clone();
        let account = account.as_ref().map(|a| a.id.clone());
        SettingItem::render(move |_, _, cx: &mut App| {
            let (current, next, root, account) =
                (current.clone(), next.clone(), root.clone(), account.clone());
            design::section("account-rotate", tr("settings.account.rotate_title"), cx)
                .child(super::presentation::field(
                    tr("settings.account.current"),
                    Input::new(&current)
                        .id("rotate-current")
                        .aria_label(tr("settings.account.current"))
                        .mask_toggle()
                        .large(),
                    cx,
                ))
                .child(super::presentation::field(
                    tr("settings.account.next"),
                    Input::new(&next)
                        .id("rotate-next")
                        .aria_label(tr("settings.account.next"))
                        .mask_toggle()
                        .large(),
                    cx,
                ))
                .child(
                    h_flex().child(
                        crate::style::primary_button("settings-rotate", cx)
                            .label(tr("settings.account.rotate"))
                            .on_click(move |_, window, cx| {
                                let old = take(&current, window, cx);
                                let new = take(&next, window, cx);
                                let Some(account_id) = account.clone() else { return };
                                let kind = IntentKind::RotatePassphrase {
                                    account_id,
                                    current: old,
                                    next: new,
                                };
                                root.update(cx, |root, cx| {
                                    root.submit_secret(kind, window, cx);
                                });
                            }),
                    ),
                )
                .into_any_element()
        })
    };
    // Claim or change this account's username. The recovery phrase is taken
    // with the name and handed to the root as a secret; it never stays in
    // the field.
    let claim = {
        let name = inputs.username.clone();
        let phrase = inputs.username_phrase.clone();
        let root = context.root.clone();
        let store = context.store.clone();
        let account = account.as_ref().map(|a| a.id.clone());
        let sink = sink.clone();
        let cooldown = inputs.refresh_cooldown.clone();
        SettingItem::render(move |_, _, cx: &mut App| {
            let Some(account_id) = account.clone() else { return h_flex().into_any_element() };
            // A build or computer without a username centre cannot claim one:
            // the section says so and offers no form.
            if !username_configured {
                return design::section(
                    "account-username-claim",
                    tr("settings.account.username_claim_title"),
                    cx,
                )
                .child(design::help(tr("settings.account.username_not_set_up"), cx))
                .into_any_element();
            }
            let (name, phrase, root, store, sink) =
                (name.clone(), phrase.clone(), root.clone(), store.clone(), sink.clone());
            let status = claim_status(store.read(cx).store(), &account_id);
            let section = design::section(
                "account-username-claim",
                tr("settings.account.username_claim_title"),
                cx,
            )
            .child(design::help(tr("settings.account.username_help"), cx))
            .child(super::presentation::field(
                tr("settings.account.username_name"),
                Input::new(&name)
                    .id("username-name")
                    .aria_label(tr("settings.account.username_name"))
                    .large(),
                cx,
            ))
            .child(super::presentation::field(
                tr("settings.account.username_phrase"),
                Input::new(&phrase)
                    .id("username-phrase")
                    .aria_label(tr("settings.account.username_phrase"))
                    .mask_toggle()
                    .large(),
                cx,
            ))
            .children(status.as_ref().map(|(text, _)| design::help(text.clone(), cx)));
            let cooling = cooldown.get();
            let refresh = status.clone().and_then(|(_, refresh)| refresh).map(|operation_id| {
                let cooldown = cooldown.clone();
                h_flex().child(
                    crate::style::secondary_button("settings-username-refresh", cx)
                        .label(tr("settings.account.username_refresh"))
                        .design_disabled(cooling, cx)
                        .on_click(move |_, window, cx| {
                            if cooldown.replace(true) {
                                return;
                            }
                            sink.emit(
                                UiEventKind::UsernameStatusRefreshRequested {
                                    operation_id: operation_id.clone(),
                                },
                                window,
                                cx,
                            );
                            window.refresh();
                            let cooldown = cooldown.clone();
                            window
                                .spawn(cx, async move |cx| {
                                    cx.background_executor().timer(REFRESH_COOLDOWN).await;
                                    cooldown.set(false);
                                    cx.update(|window, _| window.refresh()).ok();
                                })
                                .detach();
                        }),
                )
            });
            section
                .child(
                    h_flex().child(
                        crate::style::primary_button("settings-username-claim", cx)
                            .label(tr("settings.account.username_submit"))
                            .on_click(move |_, window, cx| {
                                // The phrase is taken (and cleared) at once. The
                                // name stays while the claim is refused before it
                                // is sent.
                                let secret = take(&phrase, window, cx);
                                let typed = name.read(cx).value().trim().to_owned();
                                if typed.is_empty() {
                                    return;
                                }
                                let kind = IntentKind::ClaimUsername {
                                    account_id: account_id.clone(),
                                    name: typed,
                                    secret,
                                };
                                let answer = root
                                    .update(cx, |root, cx| root.submit_secret(kind, window, cx));
                                if answer.starts_with(SUBMITTED) {
                                    name.update(cx, |input, cx| input.set_value("", window, cx));
                                }
                            }),
                    ),
                )
                .children(refresh)
                .into_any_element()
        })
    };
    // The open-account switch (G25) has no backend contract yet: one
    // section with the typed reason, its heading said once.
    let privacy = SettingItem::render(|_, _, cx: &mut App| {
        design::section("account-privacy", tr("settings_ui.privacy"), cx)
            .child(
                div()
                    .id("account-privacy-unavailable")
                    .test_support()
                    .role(gpui_kit::Role::Status)
                    .p(space(3.))
                    .rounded(gpui_kit::rems(
                        ramflux_desktop_navigation::palette(cx).radius_md / 15.0,
                    ))
                    .bg(ramflux_desktop_navigation::palette(cx).surface)
                    .child(design::help(
                        super::presentation::contract_reason(
                            ramflux_desktop_sdk::business::ContractGap::AccountPrivacy,
                        ),
                        cx,
                    )),
            )
            .into_any_element()
    });
    SettingPage::new(tr("settings.section.account")).group(
        SettingGroup::new().item(current).item(rotation).item(claim).item(app_lock).item(privacy),
    )
}

/// The timeout choices of the app lock: the daemon's value, its label key
/// and the element id of its button.
const APP_LOCK_TIMEOUTS: [(u64, &str, &str); 5] = [
    (0, "settings.app_lock.immediately", "app-lock-timeout-0"),
    (60_000, "settings.app_lock.minute", "app-lock-timeout-60000"),
    (300_000, "settings.app_lock.minutes_5", "app-lock-timeout-300000"),
    (900_000, "settings.app_lock.minutes_15", "app-lock-timeout-900000"),
    (3_600_000, "settings.app_lock.hour", "app-lock-timeout-3600000"),
];

/// Settings · Account · App lock: the switch (off by default), the timeout
/// after which the app asks for the account code, the account code sheet the
/// daemon opens for a weakening, the wait after wrong codes, and the biometric
/// row the daemon does not offer yet.
fn app_lock_item(
    context: &SectionContext,
    account: Option<AccountId>,
    inputs: &AccountInputs,
) -> SettingItem {
    let sink = context.sink(SettingsSection::Account);
    let store = context.store.clone();
    let root = context.root.clone();
    let code = inputs.app_lock_code.clone();
    let clock = inputs.code_clock.clone();
    let ticking = inputs.code_ticking.clone();
    SettingItem::render(move |_, window, cx: &mut App| {
        let Some(account_id) = account.clone() else { return h_flex().into_any_element() };
        let (lock, refusal, failure, sheet_open) = {
            let handle = store.read(cx);
            let store = handle.entry_store();
            let failure = store
                .failures()
                .iter()
                .rev()
                .find(|f| f.method == methods::APP_LOCK_CONFIGURE)
                .map(|f| f.message.clone());
            (
                store.app_lock(&account_id).cloned(),
                store.app_lock_refusal(&account_id),
                failure,
                store.app_lock_sheet_open(&account_id),
            )
        };
        let sheet = sheet_open.then(|| code_sheet(&account_id, &code, &root, cx));
        let section = design::section("account-app-lock", tr("settings.app_lock.title"), cx);
        let Some(lock) = lock else {
            return section
                .child(design::help(tr("settings.app_lock.loading"), cx))
                .into_any_element();
        };
        if lock.state == AppLockState::Unavailable {
            return section
                .child(design::help(tr("settings.app_lock.unavailable"), cx))
                .into_any_element();
        }
        let wait_ms = lock.code_retry_after_ms.map(|ms| remaining_ms(&clock, ms)).unwrap_or(0);
        let waiting = wait_ms > 0;
        let exhausted = lock.code_attempts_exhausted;
        if waiting && !ticking.get() {
            // The countdown is shown live: the window is drawn again each second.
            ticking.set(true);
            let ticking = ticking.clone();
            window
                .spawn(cx, async move |cx| {
                    cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
                    ticking.set(false);
                    cx.update(|window, _| window.refresh()).ok();
                })
                .detach();
        }
        // Weakening (a switch off, a longer timeout) needs the code: none is
        // asked during a wait, after exhaustion, or while a sheet is open.
        let blocked = waiting || exhausted || sheet_open;
        let (enabled, current) = (lock.enabled, lock.timeout_ms);
        let switch_sink = sink.clone();
        let switch_id = account_id.clone();
        let timeouts = APP_LOCK_TIMEOUTS.iter().map(|&(ms, key, id)| {
            let (button_sink, button_id) = (sink.clone(), account_id.clone());
            let button = if ms == current {
                crate::style::primary_button(id, cx)
            } else {
                crate::style::secondary_button(id, cx)
            };
            let weakening = enabled && ms > current;
            button.label(tr(key)).design_disabled(!enabled || (weakening && blocked), cx).on_click(
                move |_, window, cx| {
                    button_sink.emit(
                        UiEventKind::AppLockConfigureRequested {
                            account_id: button_id.clone(),
                            enabled,
                            timeout_ms: ms,
                        },
                        window,
                        cx,
                    )
                },
            )
        });
        let switch_blocked = enabled && blocked;
        let section = section
            .child(
                Switch::new("app-lock-switch")
                    .label(tr("settings.app_lock.switch"))
                    .checked(enabled)
                    .on_change(move |checked, window, cx| {
                        // Turning the lock off while it is blocked is not sent: the
                        // switch shows the state the daemon reports.
                        if switch_blocked && !*checked {
                            return;
                        }
                        switch_sink.emit(
                            UiEventKind::AppLockConfigureRequested {
                                account_id: switch_id.clone(),
                                enabled: *checked,
                                timeout_ms: current,
                            },
                            window,
                            cx,
                        )
                    }),
            )
            .child(design::help(tr("settings.app_lock.help"), cx))
            .child(h_flex().gap(space(2.)).children(timeouts))
            .child(design::row(
                tr("settings.app_lock.biometric"),
                tr("settings.app_lock.biometric_unavailable"),
                None,
                cx,
            ));
        let section = section.children(sheet);
        let line = match refusal {
            Some(AppLockCodeRefusal::Wrong) => Some(tr("entry.app_lock.wrong_code")),
            Some(AppLockCodeRefusal::TimedOut) => Some(tr("settings.app_lock.not_changed")),
            // A dismissed sheet is no error; a wait or exhaustion has its own line below.
            Some(
                AppLockCodeRefusal::Cancelled
                | AppLockCodeRefusal::RetryLater
                | AppLockCodeRefusal::Exhausted,
            )
            | None => None,
        };
        // One status line: the daemon's wait or exhaustion, else the code
        // refusal, else the failure mapped to a sentence (never its text).
        let status = if exhausted {
            Some(tr("settings.app_lock.code_exhausted"))
        } else if waiting {
            Some(tr_args("settings.app_lock.code_wait", &[("time", &countdown(wait_ms))]))
        } else if let Some(line) = line {
            Some(line)
        } else {
            failure.map(|message| {
                tr_args(
                    "settings.app_lock.failed",
                    &[("reason", &i18n::backend_text(&message))],
                )
            })
        };
        let section = section.children(status.map(|text| design::help(text, cx)));
        section.into_any_element()
    })
}

/// The wait left of a daemon wrong-code throttle, counted from the moment
/// this window first saw that value (the daemon's own figure, never a
/// restarted one).
pub(crate) fn remaining_ms(
    clock: &std::cell::Cell<Option<(u64, std::time::Instant)>>,
    ms: u64,
) -> u64 {
    let at = match clock.get() {
        Some((seen, at)) if seen == ms => at,
        _ => {
            let at = std::time::Instant::now();
            clock.set(Some((ms, at)));
            at
        }
    };
    ms.saturating_sub(u64::try_from(at.elapsed().as_millis()).unwrap_or(u64::MAX))
}

/// `mm:ss`, rounded up to the second.
pub(crate) fn countdown(ms: u64) -> String {
    let secs = ms.div_ceil(1_000);
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// The account code sheet of an app lock change: a masked input and the two
/// answers. The code is taken at once when answered; cancelling sends an
/// empty answer, which refuses the prompt.
fn code_sheet(
    account_id: &AccountId,
    code: &Entity<InputState>,
    root: &Entity<crate::root::IntentRoot>,
    cx: &mut App,
) -> gpui_kit::AnyElement {
    let (confirm_root, confirm_code, confirm_id) = (root.clone(), code.clone(), account_id.clone());
    let (cancel_root, cancel_code, cancel_id) = (root.clone(), code.clone(), account_id.clone());
    design::section("account-app-lock-code", tr("settings.app_lock.code_title"), cx)
        .child(design::help(tr("settings.app_lock.code_body"), cx))
        .child(
            Input::new(code)
                .id("app-lock-code")
                .aria_label(tr("settings.app_lock.code"))
                .mask_toggle(),
        )
        .child(
            h_flex()
                .gap(space(2.))
                .child(
                    crate::style::primary_button("app-lock-code-confirm", cx)
                        .label(tr("settings.app_lock.confirm"))
                        .on_click(move |_, window, cx| {
                            let secret = take(&confirm_code, window, cx);
                            if secret.is_empty() {
                                return;
                            }
                            confirm_root.update(cx, |root, cx| {
                                root.submit_secret(
                                    IntentKind::AnswerAppLockCode {
                                        account_id: confirm_id.clone(),
                                        secret,
                                    },
                                    window,
                                    cx,
                                )
                            });
                        }),
                )
                .child(
                    crate::style::secondary_button("app-lock-code-cancel", cx)
                        .label(tr("settings.app_lock.cancel"))
                        .on_click(move |_, window, cx| {
                            let _ = take(&cancel_code, window, cx);
                            cancel_root.update(cx, |root, cx| {
                                root.submit_secret(
                                    IntentKind::AnswerAppLockCode {
                                        account_id: cancel_id.clone(),
                                        secret: SecretInput::new(String::new()),
                                    },
                                    window,
                                    cx,
                                )
                            });
                        }),
                ),
        )
        .into_any_element()
}

/// Dismisses every open account code sheet when the window loses focus (see
/// [`refuse_app_lock_codes`]).
pub(super) fn dismiss_app_lock_codes(
    store: &Entity<crate::app::StoreHandle>,
    root: &Entity<crate::root::IntentRoot>,
    code: &Entity<InputState>,
    window: &mut Window,
    cx: &mut App,
) {
    if window.is_window_active() {
        return;
    }
    refuse_app_lock_codes(store, root, code, window, cx);
}

/// Dismisses every open account code sheet now (the window lost focus, the
/// settings overlay closed, the lock or cover rose, or the account changed):
/// the typed code is cleared and every prompt is refused, so nothing typed
/// can be answered later from a sheet shown again.
pub(super) fn refuse_app_lock_codes(
    store: &Entity<crate::app::StoreHandle>,
    root: &Entity<crate::root::IntentRoot>,
    code: &Entity<InputState>,
    window: &mut Window,
    cx: &mut App,
) {
    clear_code(code, window, cx);
    // The sheets live in the daemon session's store, where the configure runs.
    let accounts: Vec<AccountId> =
        store.read(cx).entry_store().app_lock_sheets().cloned().collect();
    for account_id in accounts {
        root.update(cx, |root, cx| {
            root.submit_secret(
                IntentKind::AnswerAppLockCode {
                    account_id,
                    secret: SecretInput::new(String::new()),
                },
                window,
                cx,
            )
        });
    }
}
