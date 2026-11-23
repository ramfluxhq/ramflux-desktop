// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Board key -> how to reach the screen on the mock source.
//!
//! A recipe names the fixture it starts from, the steps that reach the
//! board's state, and landmarks (element ids) the board's screen must show.
//! Dark proofs and the English boards reuse the recipe of the screen they
//! prove; the theme and locale come from the board (or the matrix).
//!
//! Content comes from `fixtures/board_zh.ron`, which reproduces the boards'
//! names, messages, times, counts, files and devices in Simplified Chinese;
//! the chrome copy is the application's own catalogue. Entry boards use the
//! mock ceremony and recovery with the boards' account name (`account_personal`).

use ramflux_desktop_core::{
    Account, AccountId, AccountKind, Call, CallDirection, CallEndReason, CallId, CallState,
    Capabilities, ContextId, DomainEvent, EffectFailure, FailureCode, Listing, LockState,
    Networking, ParticipantId, SafetyState, SecureSessions, SourceStatus, Timestamp,
};
use ramflux_desktop_test::Scenario;
use ramflux_desktop_test::mock_ceremony::{DEMO_POSITIONS, demo_word};
use ramflux_desktop_test::mock_recovery::{DEMO_ANCHOR, demo_phrase};
use ramflux_desktop_ui::actions::OpenPalette;
use ramflux_desktop_ui::settings::SettingsSection;
use ramflux_desktop_ui::workspace::WorkspaceRoute;

use crate::harness::{Run, Start, Step};
use crate::text::t;

const PASSPHRASE: &str = "river stone lantern";
/// The epoch of the board fixture (2026-11-10T10:00Z).
const EPOCH_MS: i64 = 1_794_304_800_000;

/// How a board is captured.
pub struct Recipe {
    pub start: Start,
    /// What the capture shows and from which fixture.
    pub note: &'static str,
    pub landmarks: &'static [&'static str],
    pub steps: fn(&mut Run) -> Step,
    /// The steps leave a popover, menu or hover open: do not move the
    /// pointer or blur afterwards.
    pub keep: bool,
}

/// What the capture program does with a board.
pub enum Plan {
    Capture(Recipe),
    /// Not capturable, with the reason.
    Skip(&'static str),
}

/// The board content in Simplified Chinese (`fixtures/board_zh.ron`).
fn board() -> Result<Scenario, String> {
    Scenario::from_ron(include_str!("fixtures/board_zh.ron"))
        .map_err(|e| format!("fixture board_zh: {e}"))
}

/// The board content without the events `drop` matches.
fn board_without(drop: impl Fn(&DomainEvent) -> bool) -> Result<Scenario, String> {
    let mut s = board()?;
    s.timeline.retain(|(_, e)| !drop(e));
    Ok(s)
}

fn push(scenario: &mut Scenario, at: u64, event: DomainEvent) {
    scenario.timeline.push((at, event));
    scenario.timeline.sort_by_key(|(t, _)| *t);
}

fn participant(id: &str, display: &str, safety: SafetyState, number: Option<&str>) -> DomainEvent {
    DomainEvent::ParticipantUpserted(ramflux_desktop_core::Participant {
        id: ParticipantId::new(id),
        kind: ramflux_desktop_core::ParticipantKind::Human,
        display: display.to_owned(),
        safety,
        contact: true,
        safety_number: number.map(str::to_owned),
        link: ramflux_desktop_core::ContactLink::None,
        reachable: None,
        kind_claim: ramflux_desktop_core::KindClaim::None,
        name_warnings: ramflux_desktop_core::NameWarnings::default(),
        root_source: None,
    })
}

fn account(id: &str, label: &str, lock_state: LockState) -> Account {
    Account {
        id: AccountId::new(id),
        label: label.to_owned(),
        kind: AccountKind::HumanInteractive,
        node_id: None,
        device_id: None,
        lock_state,
        self_participant_id: None,
        secure_sessions: SecureSessions::Ready,
        networking_session: None,
        networking_custody: None,
        authority_blocked: false,
        networking_failure: None,
        networking_reason: None,
        networking: Networking::Live,
    }
}

/// An account whose collections are listed late (at 500 ms) and empty;
/// without the listing event the lists stay loading.
fn empty_account(listed: bool) -> Scenario {
    let mut timeline = vec![
        (0, DomainEvent::SourceStatusChanged(SourceStatus::Connected)),
        (
            0,
            DomainEvent::AccountUpdated(account(
                "a-personal",
                &t("account_personal"),
                LockState::Unlocked,
            )),
        ),
    ];
    if listed {
        timeline.push((
            500,
            DomainEvent::ListingsLoaded(vec![
                Listing::Contexts,
                Listing::Contacts,
                Listing::Objects,
                Listing::Approvals,
            ]),
        ));
    }
    Scenario {
        name: "empty_account".to_owned(),
        description: "An account whose collections are listed late and empty".to_owned(),
        epoch: Timestamp(EPOCH_MS),
        timeline,
        responses: Vec::new(),
        companions: Vec::new(),
    }
}

fn failure(method: &str) -> DomainEvent {
    DomainEvent::EffectFailed(EffectFailure {
        method: method.to_owned(),
        code: FailureCode::Transport,
        message: "connection reset".to_owned(),
    })
}

fn call(state: CallState, answered: bool) -> DomainEvent {
    DomainEvent::CallUpserted(Call {
        id: CallId::new("call-board"),
        context_id: ContextId::new("c-zhou"),
        peer: ParticipantId::new("p-zhou"),
        caller: ParticipantId::new("p-me"),
        direction: CallDirection::Outgoing,
        state,
        version: 1,
        started_at: Timestamp(EPOCH_MS + 1_000),
        answered_at: answered.then_some(Timestamp(EPOCH_MS + 2_000)),
        ended_at: state.is_ended().then_some(Timestamp(EPOCH_MS + 3_000)),
        order: None,
    })
}

/// The board world as the message boards (02, B01, …) draw it: no pending
/// strangers and no conversation with Nova in the list.
fn messages_world() -> Result<Scenario, String> {
    board_without(|e| match e {
        DomainEvent::ContactRequestUpserted(_)
        | DomainEvent::GrantRequested(_)
        | DomainEvent::GrantResolved { .. } => true,
        DomainEvent::ContextUpserted(c) => c.id.as_str() == "c-nova",
        DomainEvent::MomentAdded(m) => m.context_id.as_str() == "c-nova",
        _ => false,
    })
}

// ---- shared step sequences ----

/// The full board world (contacts, strangers, agents, tasks).
fn daily(run: &mut Run) -> Step {
    run.play(&board()?, 1_000);
    Ok(())
}

/// The message boards' world with the product conversation (c-product) open.
fn daily_product(run: &mut Run) -> Step {
    run.play(&messages_world()?, 1_000);
    run.show("c-product")
}

fn agent_conversation(run: &mut Run) -> Step {
    daily(run)?;
    run.route(WorkspaceRoute::Agents)?;
    run.click("agent-p-nova")?;
    run.click("composer-mode")
}

fn strangers_selected(run: &mut Run) -> Step {
    daily(run)?;
    run.route(WorkspaceRoute::Strangers)?;
    let workspace = run.cx.update(|cx| run.shell.read(cx).workspace().clone());
    let area = run.cx.update(|cx| workspace.read(cx).strangers().clone());
    run.cx.update(|cx| area.update(cx, |a, cx| a.select(Some("req-chenyao".into()), cx)));
    run.cx.run_until_parked();
    Ok(())
}

/// B11: from the product conversation (c-product) the board shows, the
/// contacts tool comparing p-zhou's safety number (the app compares in the tool's row; it
/// has no modal dialog).
fn safety_compare(run: &mut Run) -> Step {
    let mut s = messages_world()?;
    push(
        &mut s,
        900,
        participant(
            "p-zhou",
            &t("name_zhou"),
            SafetyState::Unverified,
            Some("0421 1830 5502 7681 9904"),
        ),
    );
    run.play(&s, 1_000);
    run.show("c-product")?;
    run.click("rail-contacts")?;
    // The row's compare button is below the fold of the tool: start the
    // comparison the way that button does.
    let workspace = run.cx.update(|cx| run.shell.read(cx).workspace().clone());
    let dock = run.cx.update(|cx| workspace.read(cx).contacts_dock().clone());
    run.cx.update(|cx| dock.update(cx, |d, cx| d.start_verify(ParticipantId::new("p-zhou"), cx)));
    run.cx.run_until_parked();
    Ok(())
}

fn conversation_menu(run: &mut Run) -> Step {
    daily_product(run)?;
    run.click("conversation-more")
}

fn settings_board(run: &mut Run, section: SettingsSection) -> Step {
    daily(run)?;
    run.settings(section);
    Ok(())
}

fn tasks(run: &mut Run) -> Step {
    daily(run)?;
    run.route(WorkspaceRoute::Tasks)
}

/// The board world without devices; devices listed at 2 s, offline at 3 s,
/// back with a failed `device.list` at 4 s (desk_settings.rs).
fn device_states(run: &mut Run, ms: u64) -> Step {
    let mut s = board_without(|e| matches!(e, DomainEvent::DeviceUpserted(_)))?;
    push(&mut s, 2_000, DomainEvent::ListingsLoaded(vec![Listing::Devices]));
    push(&mut s, 3_000, DomainEvent::SourceStatusChanged(SourceStatus::Disconnected));
    push(&mut s, 4_000, DomainEvent::SourceStatusChanged(SourceStatus::Connected));
    push(&mut s, 4_000, failure("device.list"));
    run.play(&s, 1_000);
    run.settings(SettingsSection::Devices);
    run.advance(ms);
    Ok(())
}

/// A section over an empty account: loading, offline, failed or empty.
fn section_state(run: &mut Run, route: WorkspaceRoute, state: &str, method: &str) -> Step {
    run.play(&empty_account(state == "empty"), 100);
    run.route(route)?;
    match state {
        "offline" => {
            run.inject(0, DomainEvent::SourceStatusChanged(SourceStatus::Disconnected))?;
            run.advance(100);
        }
        "failed" => {
            run.inject(0, failure(method))?;
            run.advance(100);
        }
        "empty" => run.advance(600),
        _ => {}
    }
    Ok(())
}

fn no_requests() -> Result<Scenario, String> {
    board_without(|e| matches!(e, DomainEvent::ContactRequestUpserted(_)))
}

fn ceremony(run: &mut Run) -> Step {
    run.tick(2_600);
    run.click("account-create-phrase")?;
    run.tick(100);
    Ok(())
}

fn ceremony_ready(run: &mut Run) -> Step {
    ceremony(run)?;
    run.type_into("ceremony-name-input", &t("account_personal"))?;
    run.click("ceremony-continue")?;
    run.tick(300);
    Ok(())
}

fn ceremony_phrase(run: &mut Run) -> Step {
    ceremony_ready(run)?;
    run.click("ceremony-show")?;
    run.tick(300);
    Ok(())
}

fn ceremony_words(run: &mut Run) -> Step {
    ceremony_phrase(run)?;
    run.click("ceremony-hide")?;
    run.tick(300);
    Ok(())
}

fn ceremony_done(run: &mut Run) -> Step {
    ceremony_words(run)?;
    let words = DEMO_POSITIONS.map(|p| demo_word(usize::from(p)));
    for (id, word) in
        ["ceremony-word-1", "ceremony-word-2", "ceremony-word-3"].into_iter().zip(words)
    {
        run.type_into(id, word)?;
    }
    run.click("ceremony-next")?;
    run.tick(100);
    run.type_into("ceremony-passphrase-input", PASSPHRASE)?;
    run.type_into("ceremony-passphrase-repeat", PASSPHRASE)?;
    run.click("ceremony-confirm-submit")?;
    run.tick(300);
    Ok(())
}

/// The born-v3 entry with a locked account that can be recovered.
fn recovery_world(run: &mut Run) -> Step {
    run.tick(2_600);
    run.mock(|m| {
        m.ceremony_mut().recovery_mut().allow("acct-r", &t("account_personal"), PASSPHRASE);
        m.inject(
            0,
            DomainEvent::AccountUpdated(account(
                "acct-r",
                &t("account_personal"),
                LockState::Locked,
            )),
        );
    })?;
    run.tick(300);
    Ok(())
}

fn recovery_intro(run: &mut Run) -> Step {
    recovery_world(run)?;
    run.click("account-recover-acct-r")?;
    run.tick(100);
    Ok(())
}

fn recovery_done(run: &mut Run) -> Step {
    recovery_intro(run)?;
    run.click("recovery-continue")?;
    run.tick(300);
    let grouped = DEMO_ANCHOR
        .chars()
        .collect::<Vec<_>>()
        .chunks(4)
        .map(|g| g.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ");
    run.type_into("recovery-fingerprint-input", &grouped)?;
    run.type_into("recovery-phrase-input", &demo_phrase())?;
    run.type_into("recovery-passphrase-input", PASSPHRASE)?;
    run.click("recovery-submit")?;
    run.tick(300);
    Ok(())
}

fn wizard(run: &mut Run, next: usize, create: bool) -> Step {
    let mut s = board()?;
    push(
        &mut s,
        5,
        DomainEvent::CapabilitiesUpdated(Capabilities {
            born_v3_creation: true,
            ..Default::default()
        }),
    );
    run.play(&s, 1_000);
    run.settings(SettingsSection::Agents);
    run.click("agent-create")?;
    if next > 0 {
        run.type_into("agent-wizard-name", "Nova")?;
    }
    for _ in 0..next {
        run.click("agent-wizard-next")?;
    }
    if create {
        run.click("agent-wizard-create")?;
        run.advance(500);
    }
    Ok(())
}

fn direct_call(run: &mut Run, event: DomainEvent) -> Step {
    run.play(&messages_world()?, 1_000);
    run.show("c-zhou")?;
    run.inject(0, event)?;
    run.advance(100);
    Ok(())
}

/// A16: the local daemon is started and exits at once.
fn daemon_failed(run: &mut Run) -> Step {
    run.tick(1_000);
    run.click("entry-daemon-start")?;
    run.until(20_000, |run| {
        run.cx.update(|cx| {
            matches!(
                run.shell.read(cx).launcher().read(cx).state(),
                ramflux_desktop_ui::launch::LaunchState::Failed(_)
            )
        })
    })?;
    run.tick(200);
    Ok(())
}

macro_rules! capture {
    ($start:expr, $note:expr, [$($landmark:expr),* $(,)?], $steps:expr) => {
        Plan::Capture(Recipe { start: $start, note: $note, landmarks: &[$($landmark),*], steps: $steps, keep: false })
    };
    ($start:expr, $note:expr, [$($landmark:expr),* $(,)?], $steps:expr, keep) => {
        Plan::Capture(Recipe { start: $start, note: $note, landmarks: &[$($landmark),*], steps: $steps, keep: true })
    };
}

const S02: Start = Start::Scenario("s02_daily");
const BORN: Start = Start::Live("s13_born_v3");

/// The plan of `key`, or `None` for a board without a recipe.
pub fn plan(key: &str) -> Option<Plan> {
    Some(match key {
        // Shell and messages.
        "02" | "I24" => capture!(
            S02,
            "board_zh, c-product open",
            ["navigation-rail", "stack-filters", "conversation-header", "composer"],
            daily_product
        ),
        "02L" => capture!(
            S02,
            "empty account (account_personal) before its listings: list loading",
            ["stack-list-state"],
            |run| {
                run.play(&empty_account(false), 100);
                Ok(())
            }
        ),
        "B01" => capture!(
            S02,
            "board_zh c-product, identity popover open",
            ["identity-menu"],
            |run| {
                daily_product(run)?;
                run.click("nav-account")
            },
            keep
        ),
        "B04" | "I34" => capture!(
            S02,
            "board_zh c-product, conversation menu open (the app has a menu, no settings panel)",
            ["conversation-header"],
            conversation_menu,
            keep
        ),
        "B09" => capture!(
            S02,
            "board_zh c-product, command palette open",
            ["palette"],
            |run| {
                daily_product(run)?;
                run.dispatch(Box::new(OpenPalette))
            },
            keep
        ),
        "B10" => capture!(
            S02,
            "board_zh, p-zhou's key changed, conversation with p-zhou open",
            [],
            |run| {
                let mut s = messages_world()?;
                push(
                    &mut s,
                    900,
                    participant(
                        "p-zhou",
                        &t("name_zhou"),
                        SafetyState::KeyChanged,
                        Some("0421 1830 5502 7681 9031"),
                    ),
                );
                run.play(&s, 1_000);
                run.show("c-zhou")
            }
        ),
        "B11" | "I31" => capture!(
            S02,
            "board_zh c-product, contacts tool comparing p-zhou's safety number (the app compares in the tool row, no dialog)",
            ["verify-confirm"],
            safety_compare
        ),
        "B38" => capture!(
            S02,
            "board_zh c-product, source disconnected (offline banner; no queued-messages UI)",
            ["status-strip"],
            |run| {
                let mut s = messages_world()?;
                push(&mut s, 900, DomainEvent::SourceStatusChanged(SourceStatus::Disconnected));
                run.play(&s, 1_000);
                run.show("c-product")
            }
        ),
        "B32" => capture!(
            S02,
            "board_zh c-product (the board's p-zhou and Nova thread with a quote and a reaction is not in the fixture)",
            ["conversation-header"],
            daily_product
        ),
        "B39" | "B40" | "B41" => {
            Plan::Skip("absent: no search states in code (local filter and palette only)")
        }
        // Identity.
        "A01" => capture!(
            Start::Scenario("s01_first_launch"),
            "s01 before the daemon connects: local service step (mock)",
            ["entry-header"],
            |run| {
                run.advance(500);
                Ok(())
            }
        ),
        "A02" => capture!(
            Start::Scenario("s01_first_launch"),
            "s01 at 3 s: invite node waiting for confirmation",
            ["node-confirm"],
            |run| {
                run.advance(3_000);
                Ok(())
            }
        ),
        "A03" => capture!(
            BORN,
            "s13 entry with the locked account (account_personal) (passphrase era; six-digit code not in code)",
            [],
            recovery_world
        ),
        "A04" => capture!(BORN, "s13 create account, name step", ["ceremony-name-input"], ceremony),
        "A05" => capture!(
            BORN,
            "s13 create account (account_personal), ready to save the phrase",
            ["ceremony-show"],
            ceremony_ready
        ),
        "A06" => {
            capture!(BORN, "s13 create account, phrase shown", ["ceremony-hide"], ceremony_phrase)
        }
        "A07" => capture!(
            BORN,
            "s13 create account, confirm three words",
            ["ceremony-word-1"],
            ceremony_words
        ),
        "A09" => capture!(
            BORN,
            "s13 create account done (copy mentions the passphrase)",
            [],
            ceremony_done
        ),
        "A10" => {
            capture!(
                BORN,
                "s13 recover account (account_personal), intro",
                ["recovery-continue"],
                recovery_intro
            )
        }
        "A12" => capture!(BORN, "s13 recover account, done", [], recovery_done),
        "A16" => capture!(
            Start::EntryFailingDaemon,
            "first run: the started local daemon exits at once (this program as a failing stand-in)",
            ["entry-daemon-failed"],
            daemon_failed
        ),
        "A26" => capture!(
            Start::Scenario("s01_first_launch"),
            "s01 node confirmed, link this device path (activation)",
            ["activation-code"],
            |run| {
                run.advance(3_000);
                run.click("node-confirm")?;
                run.advance(200);
                run.click("entry-path-1")
            }
        ),
        // Groups.
        "C01" => {
            capture!(S02, "board_zh new group form over c-product", ["new-group-name"], |run| {
                daily_product(run)?;
                run.workspace(|w, window, cx| w.open_new_group(window, cx))
            })
        }
        "C02" => capture!(
            S02,
            "board_zh groups, c-product with its member panel",
            ["group-profile"],
            |run| {
                daily(run)?;
                run.route(WorkspaceRoute::Groups)?;
                run.workspace(|w, window, cx| w.show_group(ContextId::new("c-product"), window, cx))
            }
        ),
        "C32" => {
            capture!(S02, "empty account, groups loading", ["groups-state"], |run| section_state(
                run,
                WorkspaceRoute::Groups,
                "loading",
                ""
            ))
        }
        "C33" => {
            capture!(S02, "empty account, groups offline", ["groups-state"], |run| section_state(
                run,
                WorkspaceRoute::Groups,
                "offline",
                ""
            ))
        }
        "C34" => {
            capture!(
                S02,
                "empty account, group.list failed",
                ["groups-state"],
                |run| section_state(run, WorkspaceRoute::Groups, "failed", "group.list")
            )
        }
        "C35" => capture!(S02, "empty account, no groups", [], |run| section_state(
            run,
            WorkspaceRoute::Groups,
            "empty",
            ""
        )),
        // Agents.
        "04" | "I27" => capture!(
            S02,
            "board_zh Agents, conversation with Nova, request mode",
            ["agent-roster", "conversation-agent-profile", "composer-receiver"],
            agent_conversation
        ),
        "D01" => capture!(
            S02,
            "board_zh Agents roster (Nova, Atlas, Mira, research agent), nothing selected",
            ["agent-roster"],
            |run| {
                daily(run)?;
                run.route(WorkspaceRoute::Agents)
            }
        ),
        "D33" => capture!(
            S02,
            "new agent wizard, basics (born_v3_creation capability injected)",
            ["agent-wizard-name"],
            |run| wizard(run, 0, false)
        ),
        "D34" => {
            capture!(S02, "new agent wizard, runtime step (no runtime, gated)", [], |run| wizard(
                run, 1, false
            ))
        }
        "D35" | "I28" => {
            capture!(S02, "new agent wizard, capabilities and approvals", [], |run| wizard(
                run, 2, false
            ))
        }
        "D36" => capture!(S02, "new agent wizard, conversations", [], |run| wizard(run, 3, false)),
        "D37" => capture!(S02, "new agent wizard, review", ["agent-wizard-create"], |run| wizard(
            run, 4, false
        )),
        "D38" => capture!(
            S02,
            "new agent wizard, after create (continues into the old identity ceremony)",
            [],
            |run| wizard(run, 4, true)
        ),
        "D40" => capture!(
            S02,
            "board_zh Agents, Nova with three approval cards (pending, allowed, denied)",
            ["moment-m-g-read"],
            |run| {
                daily(run)?;
                run.route(WorkspaceRoute::Agents)?;
                run.click("agent-p-nova")
            }
        ),
        "D41" | "D42" | "D43" => {
            Plan::Skip("absent: the agent roster has no loading, offline or failed state")
        }
        // Tasks.
        "06" => capture!(
            S02,
            "board_zh tasks: Nova's proposal waiting for the user",
            ["tasks-area"],
            tasks
        ),
        "E01" => capture!(S02, "board_zh tasks, All filter", ["tasks-area"], |run| {
            tasks(run)?;
            run.click("tasks-category-all")
        }),
        "E05" => {
            capture!(S02, "board_zh tasks, the reference task selected", ["tasks-detail"], |run| {
                tasks(run)?;
                run.click("task-card-t-reference")
            })
        }
        // Contacts.
        "07" => capture!(S02, "board_zh contacts section", ["contacts-table"], |run| {
            daily(run)?;
            run.route(WorkspaceRoute::Contacts)
        }),
        "F02" => capture!(
            S02,
            "board_zh add contact form in the contacts tool (the app has no dialog)",
            ["contact-add-id"],
            |run| {
                daily_product(run)?;
                run.workspace(|w, window, cx| w.open_add_contact(window, cx))
            }
        ),
        "F04" => {
            capture!(S02, "board_zh contact profile of p-zhou", ["contact-safety-number"], |run| {
                daily(run)?;
                run.route(WorkspaceRoute::Contacts)?;
                run.click("contacts-view-p-zhou")
            })
        }
        "F17" | "I26" => capture!(
            S02,
            "board_zh strangers, chenyao selected",
            ["strangers-area", "stranger-header"],
            strangers_selected
        ),
        "F18" => capture!(
            S02,
            "board_zh without requests, strangers empty",
            ["strangers-empty"],
            |run| {
                run.play(&no_requests()?, 1_000);
                run.route(WorkspaceRoute::Strangers)
            }
        ),
        "F25" | "F26" => Plan::Skip("absent: the strangers area has no loading or offline state"),
        "F27" => capture!(
            S02,
            "board_zh without requests, request read failed, strangers",
            ["strangers-empty"],
            |run| {
                let mut s = no_requests()?;
                push(
                    &mut s,
                    900,
                    DomainEvent::ContactRequestsReadFailed("connection reset".into()),
                );
                run.play(&s, 1_000);
                run.route(WorkspaceRoute::Strangers)
            }
        ),
        "F28" => capture!(S02, "empty account, contacts loading", ["contacts-state"], |run| {
            section_state(run, WorkspaceRoute::Contacts, "loading", "")
        }),
        "F29" => capture!(S02, "empty account, contacts offline", ["contacts-state"], |run| {
            section_state(run, WorkspaceRoute::Contacts, "offline", "")
        }),
        "F30" => capture!(S02, "empty account, contact.list failed", ["contacts-state"], |run| {
            section_state(run, WorkspaceRoute::Contacts, "failed", "contact.list")
        }),
        // Settings.
        "G01" => {
            capture!(S02, "board_zh settings, account", ["account-current"], |run| settings_board(
                run,
                SettingsSection::Account
            ))
        }
        "G25" | "I29" => capture!(
            S02,
            "board_zh settings, account: privacy gated (placeholder, ContractGap::AccountPrivacy)",
            ["account-privacy-unavailable"],
            |run| settings_board(run, SettingsSection::Account)
        ),
        "G02" => {
            capture!(S02, "board_zh settings, backup", ["settings-header"], |run| settings_board(
                run,
                SettingsSection::Backup
            ))
        }
        "G08" | "I30" => capture!(
            S02,
            "board_zh settings, devices (MacBook Pro, iPhone, Chrome · macOS, old laptop)",
            ["settings-header"],
            |run| settings_board(run, SettingsSection::Devices)
        ),
        "G24" => capture!(
            S02,
            "board_zh settings, devices: link gated (placeholder; the approval dialog has no screen)",
            ["settings-link-device"],
            |run| settings_board(run, SettingsSection::Devices)
        ),
        "G09" => {
            capture!(S02, "board_zh settings, nodes", ["settings-header"], |run| settings_board(
                run,
                SettingsSection::Nodes
            ))
        }
        "G11" => capture!(
            S02,
            "board_zh settings, diagnostics (sync rows need the fake daemon)",
            ["settings-header"],
            |run| settings_board(run, SettingsSection::Diagnostics)
        ),
        "G13" | "I12" | "I32" | "I33" => capture!(
            S02,
            "board_zh settings, appearance and language",
            ["appearance-mode"],
            |run| settings_board(run, SettingsSection::Appearance)
        ),
        "G27" => capture!(
            S02,
            "board_zh without devices, device list loading",
            ["settings-devices-state"],
            |run| device_states(run, 0)
        ),
        "G28" => capture!(S02, "board_zh without devices, offline", ["settings-offline"], |run| {
            device_states(run, 2_500)
        }),
        "G29" => capture!(
            S02,
            "board_zh without devices, device.list failed",
            ["settings-last-error"],
            |run| device_states(run, 3_500)
        ),
        "G30" => capture!(
            S02,
            "board_zh settings account, source disconnected",
            ["settings-offline"],
            |run| {
                settings_board(run, SettingsSection::Account)?;
                run.inject(0, DomainEvent::SourceStatusChanged(SourceStatus::Disconnected))?;
                run.advance(100);
                Ok(())
            }
        ),
        "G31" => capture!(
            S02,
            "board_zh settings account, a failed account request",
            ["settings-last-error"],
            |run| {
                settings_board(run, SettingsSection::Account)?;
                run.inject(0, failure("account.passphrase.rotate"))?;
                run.advance(100);
                Ok(())
            }
        ),
        // Docks and calls.
        "H01" => {
            capture!(S02, "board_zh c-product, files tool open", ["dock-panel-objects"], |run| {
                daily_product(run)?;
                run.click("rail-objects")
            })
        }
        "H04" => capture!(S02, "board_zh conversation with Nova, approvals tool open", [], |run| {
            daily(run)?;
            run.show("c-nova")?;
            run.click("rail-approvals")
        }),
        "K01" => capture!(
            S02,
            "board_zh direct conversation with p-zhou, call button",
            ["conversation-call"],
            |run| {
                run.play(&messages_world()?, 1_000);
                run.show("c-zhou")
            }
        ),
        "K06" => capture!(
            S02,
            "board_zh p-zhou, an active outgoing call (injected; duration on the session clock)",
            ["call-bar"],
            |run| direct_call(run, call(CallState::Active, true))
        ),
        "K40" => capture!(
            S02,
            "board_zh p-zhou offline, call button hovered",
            [],
            |run| {
                direct_call(run, DomainEvent::SourceStatusChanged(SourceStatus::Disconnected))?;
                run.hover("conversation-call")
            },
            keep
        ),
        "K41" => capture!(
            S02,
            "board_zh p-zhou, an outgoing call that failed (injected)",
            ["call-failed"],
            |run| direct_call(run, call(CallState::Ended(CallEndReason::Failed), false))
        ),
        _ => return None,
    })
}
