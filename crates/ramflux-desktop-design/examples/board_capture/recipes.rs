// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Board key -> how to reach the screen on the mock source.
//!
//! A recipe names the fixture it starts from, the steps that reach the
//! board's state, and landmarks (element ids) the board's screen must show.
//! Dark proofs and the English boards reuse the recipe of the screen they
//! prove; the theme and locale come from the board (or the matrix).
//!
//! Fixture content is the mock's demo data (mostly English names and
//! messages), not the copy drawn on the boards; the chrome copy is the
//! application's own zh-CN or en catalogue. Copy differences inside content
//! areas are therefore expected and are classed `fixture` when reviewed.

use ramflux_desktop_core::{
    Account, AccountId, AccountKind, Call, CallDirection, CallEndReason, CallId, CallState,
    Capabilities, ContactRequest, ContextId, DomainEvent, EffectFailure, FailureCode, Listing,
    LockState, Networking, ParticipantId, RequestDirection, RequestState, SecureSessions,
    SourceStatus, Timestamp,
};
use ramflux_desktop_test::mock_ceremony::{DEMO_POSITIONS, demo_word};
use ramflux_desktop_test::mock_recovery::{DEMO_ANCHOR, demo_phrase};
use ramflux_desktop_test::{Scenario, load_fixture};
use ramflux_desktop_ui::actions::OpenPalette;
use ramflux_desktop_ui::settings::SettingsSection;
use ramflux_desktop_ui::workspace::WorkspaceRoute;

use crate::harness::{Run, Start, Step};

const PASSPHRASE: &str = "river stone lantern";
/// The epoch of the default fixtures (2026-10-01T00:00Z).
const EPOCH_MS: i64 = 1_790_812_800_000;

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

fn scenario(name: &str) -> Result<Scenario, String> {
    load_fixture(name).map_err(|e| format!("fixture {name}: {e}"))
}

fn push(scenario: &mut Scenario, at: u64, event: DomainEvent) {
    scenario.timeline.push((at, event));
    scenario.timeline.sort_by_key(|(t, _)| *t);
}

/// s02 with a pending stranger, so the strangers boards have content.
fn with_stranger() -> Result<Scenario, String> {
    let mut scenario = scenario("s02_daily")?;
    let peer = ParticipantId::new("guzhou@node.example.org");
    let stranger = ramflux_desktop_core::Participant {
        id: peer.clone(),
        kind: ramflux_desktop_core::ParticipantKind::Human,
        display: "Gu Zhou".to_owned(),
        safety: ramflux_desktop_core::SafetyState::Unverified,
        contact: false,
        safety_number: Some("73051186442090573312".into()),
        link: ramflux_desktop_core::ContactLink::None,
        reachable: None,
        kind_claim: ramflux_desktop_core::KindClaim::None,
        name_warnings: ramflux_desktop_core::NameWarnings::default(),
        root_source: None,
    };
    let request = ContactRequest {
        id: "req-guzhou".into(),
        peer,
        direction: RequestDirection::Incoming,
        state: RequestState::Pending,
        note: None,
        safety_number: Some("73051186442090573312".into()),
        held_messages: 1,
        preview: Some("Hello, could you share last week's review notes?".into()),
        expires_at: None,
    };
    push(&mut scenario, 9, DomainEvent::ParticipantUpserted(stranger));
    push(&mut scenario, 10, DomainEvent::ContactRequestUpserted(request));
    Ok(scenario)
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
        (0, DomainEvent::AccountUpdated(account("a-empty", "Empty", LockState::Unlocked))),
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
    let started = Timestamp(EPOCH_MS + 3_000);
    DomainEvent::CallUpserted(Call {
        id: CallId::new("call-board"),
        context_id: ContextId::new("c-lena"),
        peer: ParticipantId::new("p-lena"),
        caller: ParticipantId::new("p-me"),
        direction: CallDirection::Outgoing,
        state,
        version: 1,
        started_at: started,
        answered_at: answered.then_some(Timestamp(EPOCH_MS + 5_000)),
        ended_at: state.is_ended().then_some(Timestamp(EPOCH_MS + 9_000)),
        order: None,
    })
}

// ---- shared step sequences ----

fn daily(run: &mut Run) -> Step {
    run.play(&with_stranger()?, 3_000);
    Ok(())
}

fn daily_team(run: &mut Run) -> Step {
    daily(run)?;
    run.show("c-team")
}

fn agent_conversation(run: &mut Run) -> Step {
    daily(run)?;
    run.route(WorkspaceRoute::Agents)?;
    run.click("agent-p-orbit")?;
    run.click("composer-mode")
}

fn strangers_selected(run: &mut Run) -> Step {
    daily(run)?;
    run.route(WorkspaceRoute::Strangers)?;
    let workspace = run.cx.update(|cx| run.shell.read(cx).workspace().clone());
    let area = run.cx.update(|cx| workspace.read(cx).strangers().clone());
    run.cx.update(|cx| area.update(cx, |a, cx| a.select(Some("req-guzhou".into()), cx)));
    run.cx.run_until_parked();
    Ok(())
}

fn safety_compare(run: &mut Run) -> Step {
    run.play(&scenario("s04_safety")?, 1_000);
    run.route(WorkspaceRoute::Contacts)?;
    let workspace = run.cx.update(|cx| run.shell.read(cx).workspace().clone());
    let profile = run.cx.update(|cx| workspace.read(cx).contact_profile().clone());
    run.cx.update(|cx| {
        profile.update(cx, |p, cx| p.set_participant(Some(ParticipantId::new("p-mira")), cx))
    });
    run.cx.run_until_parked();
    run.click("contact-compare")
}

fn conversation_menu(run: &mut Run) -> Step {
    daily_team(run)?;
    run.click("conversation-more")
}

fn settings_in(run: &mut Run, fixture: &str, ms: u64, section: SettingsSection) -> Step {
    run.play(&scenario(fixture)?, ms);
    run.settings(section);
    Ok(())
}

fn appearance(run: &mut Run) -> Step {
    settings_in(run, "s02_daily", 3_000, SettingsSection::Appearance)
}

fn devices(run: &mut Run) -> Step {
    settings_in(run, "s03_intervention", 1_000, SettingsSection::Devices)
}

fn account_settings(run: &mut Run) -> Step {
    settings_in(run, "s02_daily", 3_000, SettingsSection::Account)
}

fn tasks(run: &mut Run) -> Step {
    run.play(&scenario("s10_ai_participant")?, 200);
    run.show("c-launch")?;
    run.type_into("composer-input", "@Orbit can you check the launch checklist?")?;
    run.click("composer-send")?;
    run.advance(1_100);
    run.route(WorkspaceRoute::Tasks)
}

/// s09 without devices; devices listed at 2 s, offline at 3 s, back and a
/// failed `device.list` at 4 s (desk_settings.rs).
fn device_states(run: &mut Run, ms: u64) -> Step {
    let mut s = scenario("s09_multi_account_day")?;
    s.timeline.retain(|(_, e)| !matches!(e, DomainEvent::DeviceUpserted(_)));
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

fn ceremony(run: &mut Run) -> Step {
    run.tick(2_600);
    run.click("account-create-phrase")?;
    run.tick(100);
    Ok(())
}

fn ceremony_ready(run: &mut Run) -> Step {
    ceremony(run)?;
    run.type_into("ceremony-name-input", "Alex")?;
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
        m.ceremony_mut().recovery_mut().allow("acct-r", "Alex", PASSPHRASE);
        m.inject(0, DomainEvent::AccountUpdated(account("acct-r", "Alex", LockState::Locked)));
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
    let mut s = with_stranger()?;
    push(
        &mut s,
        5,
        DomainEvent::CapabilitiesUpdated(Capabilities {
            born_v3_creation: true,
            ..Default::default()
        }),
    );
    run.play(&s, 3_000);
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
    daily(run)?;
    run.show("c-lena")?;
    run.inject(0, event)?;
    run.advance(100);
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
            "s02_daily + stranger, Design team open",
            ["navigation-rail", "stack-filters", "conversation-header", "composer"],
            daily_team
        ),
        "02L" => capture!(
            S02,
            "empty account before its listings: list loading",
            ["stack-list-state"],
            |run| {
                run.play(&empty_account(false), 100);
                Ok(())
            }
        ),
        "B01" => capture!(
            Start::Scenario("s09_multi_account_day"),
            "s09 two accounts, identity popover open",
            ["identity-menu"],
            |run| {
                run.advance(2_000);
                run.click("nav-account")
            },
            keep
        ),
        "B04" | "I34" => capture!(
            S02,
            "s02 Design team, conversation menu open (no settings panel in code)",
            ["conversation-header"],
            conversation_menu,
            keep
        ),
        "B09" => capture!(
            S02,
            "s02 Design team, command palette open",
            ["palette"],
            |run| {
                daily_team(run)?;
                run.dispatch(Box::new(OpenPalette))
            },
            keep
        ),
        "B10" => capture!(
            Start::Scenario("s04_safety"),
            "s04 key change of Mira, conversation open",
            ["safety-banner-p-mira"],
            |run| {
                run.advance(1_000);
                run.show("c-mira")
            }
        ),
        "B11" | "I31" => capture!(
            Start::Scenario("s04_safety"),
            "s04 contact profile of Mira, safety number compare",
            ["contact-compare-cancel"],
            safety_compare
        ),
        "B38" => capture!(
            S02,
            "s02 Design team, source disconnected (offline banner; no queued-messages UI)",
            ["status-strip"],
            |run| {
                let mut s = with_stranger()?;
                push(&mut s, 3_100, DomainEvent::SourceStatusChanged(SourceStatus::Disconnected));
                run.play(&s, 3_200);
                run.show("c-team")
            }
        ),
        "B32" => capture!(
            Start::Scenario("s10_ai_participant"),
            "s10 launch conversation with a human and an agent",
            ["conversation-header"],
            |run| {
                run.advance(1_500);
                run.show("c-launch")
            }
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
            "s13 entry with a locked recoverable account (passphrase era; six-digit code not in code)",
            [],
            recovery_world
        ),
        "A04" => capture!(BORN, "s13 create account, name step", ["ceremony-name-input"], ceremony),
        "A05" => capture!(
            BORN,
            "s13 create account, ready to save the phrase",
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
            capture!(BORN, "s13 recover account, intro", ["recovery-continue"], recovery_intro)
        }
        "A12" => capture!(BORN, "s13 recover account, done", [], recovery_done),
        "A16" => Plan::Skip(
            "needs a daemon process that fails to start (first_run_shots stand-in); no mock recipe",
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
        "C01" => capture!(S02, "s02 new group form", ["new-group-name"], |run| {
            daily_team(run)?;
            run.workspace(|w, window, cx| w.open_new_group(window, cx))
        }),
        "C02" => capture!(
            Start::Scenario("s05_group_governance"),
            "s05 group directory, first group with its member panel",
            ["group-profile"],
            |run| {
                run.advance(1_000);
                run.route(WorkspaceRoute::Groups)?;
                let workspace = run.cx.update(|cx| run.shell.read(cx).workspace().clone());
                let ids = run.cx.update(|cx| {
                    let groups = workspace.read(cx).group_directory().clone();
                    groups.read(cx).entry_ids(cx)
                });
                let first = ids.into_iter().next().ok_or("no group in s05")?;
                run.workspace(move |w, window, cx| w.show_group(first, window, cx))
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
            "s02 Agents, conversation with Orbit, request mode",
            ["agent-roster", "conversation-agent-profile", "composer-receiver"],
            agent_conversation
        ),
        "D01" => capture!(S02, "s02 Agents roster, nothing selected", ["agent-roster"], |run| {
            daily(run)?;
            run.route(WorkspaceRoute::Agents)
        }),
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
            Start::Scenario("s03_intervention"),
            "s03 ops conversation with an approval moment",
            ["moment-m-approve"],
            |run| {
                run.advance(1_000);
                run.show("c-ops")
            }
        ),
        "D41" | "D42" | "D43" => {
            Plan::Skip("absent: the agent roster has no loading, offline or failed state")
        }
        // Tasks.
        "06" => capture!(
            Start::Scenario("s10_ai_participant"),
            "s10 task proposal waiting for the user",
            ["tasks-area"],
            tasks
        ),
        "E01" => capture!(
            Start::Scenario("s10_ai_participant"),
            "s10 tasks, All filter",
            ["tasks-area"],
            |run| {
                tasks(run)?;
                run.click("tasks-category-all")
            }
        ),
        "E05" => capture!(
            Start::Scenario("s10_ai_participant"),
            "s10 tasks, launch task selected",
            ["tasks-detail"],
            |run| {
                tasks(run)?;
                run.click("task-card-t-launch")
            }
        ),
        // Contacts.
        "07" => capture!(S02, "s02 + stranger, contacts section", ["contacts-table"], |run| {
            daily(run)?;
            run.route(WorkspaceRoute::Contacts)
        }),
        "F02" => {
            capture!(S02, "s02 add contact form in the contacts tool", ["contact-add-id"], |run| {
                daily_team(run)?;
                run.workspace(|w, window, cx| w.open_add_contact(window, cx))
            })
        }
        "F04" => capture!(S02, "s02 contact profile of Lena", ["contact-safety-number"], |run| {
            daily(run)?;
            run.route(WorkspaceRoute::Contacts)?;
            run.click("contacts-view-p-lena")
        }),
        "F17" | "I26" => capture!(
            S02,
            "s02 strangers, Gu Zhou selected",
            ["strangers-area", "stranger-header"],
            strangers_selected
        ),
        "F18" => {
            capture!(S02, "s02 without requests, strangers empty", ["strangers-empty"], |run| {
                run.advance(3_000);
                run.route(WorkspaceRoute::Strangers)
            })
        }
        "F25" | "F26" => Plan::Skip("absent: the strangers area has no loading or offline state"),
        "F27" => capture!(
            S02,
            "s02 contact requests read failed, strangers",
            ["strangers-empty"],
            |run| {
                run.advance(3_000);
                run.inject(0, DomainEvent::ContactRequestsReadFailed("connection reset".into()))?;
                run.advance(100);
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
        "G01" => capture!(S02, "s02 settings, account", ["account-current"], account_settings),
        "G25" | "I29" => capture!(
            S02,
            "s02 settings, account: privacy gated (placeholder, ContractGap::AccountPrivacy)",
            ["account-privacy-unavailable"],
            account_settings
        ),
        "G02" => capture!(
            Start::Scenario("s09_multi_account_day"),
            "s09 settings, backup",
            ["settings-header"],
            |run| settings_in(run, "s09_multi_account_day", 1_000, SettingsSection::Backup)
        ),
        "G08" | "I30" => capture!(
            Start::Scenario("s03_intervention"),
            "s03 settings, devices",
            ["settings-header"],
            devices
        ),
        "G24" => capture!(
            Start::Scenario("s03_intervention"),
            "s03 settings, devices: link gated (placeholder; the approval dialog has no screen)",
            ["settings-link-device"],
            devices
        ),
        "G09" => capture!(
            Start::Scenario("s09_multi_account_day"),
            "s09 settings, nodes",
            ["settings-header"],
            |run| settings_in(run, "s09_multi_account_day", 2_000, SettingsSection::Nodes)
        ),
        "G11" => capture!(
            S02,
            "s02 settings, diagnostics (sync rows need the fake daemon)",
            ["settings-header"],
            |run| settings_in(run, "s02_daily", 3_000, SettingsSection::Diagnostics)
        ),
        "G13" | "I12" | "I32" | "I33" => {
            capture!(S02, "s02 settings, appearance and language", ["appearance-mode"], appearance)
        }
        "G27" => capture!(
            Start::Scenario("s09_multi_account_day"),
            "s09 without devices, device list loading",
            ["settings-devices-state"],
            |run| device_states(run, 0)
        ),
        "G28" => capture!(
            Start::Scenario("s09_multi_account_day"),
            "s09 without devices, offline",
            ["settings-offline"],
            |run| device_states(run, 2_500)
        ),
        "G29" => capture!(
            Start::Scenario("s09_multi_account_day"),
            "s09 without devices, device.list failed",
            ["settings-last-error"],
            |run| device_states(run, 3_500)
        ),
        "G30" => capture!(
            S02,
            "s02 settings account, source disconnected",
            ["settings-offline"],
            |run| {
                account_settings(run)?;
                run.inject(0, DomainEvent::SourceStatusChanged(SourceStatus::Disconnected))?;
                run.advance(100);
                Ok(())
            }
        ),
        "G31" => capture!(
            S02,
            "s02 settings account, a failed account request",
            ["settings-last-error"],
            |run| {
                account_settings(run)?;
                run.inject(0, failure("account.passphrase.rotate"))?;
                run.advance(100);
                Ok(())
            }
        ),
        // Docks and calls.
        "H01" => capture!(S02, "s02 Design team, files tool open", ["dock-panel-objects"], |run| {
            daily_team(run)?;
            run.click("rail-objects")
        }),
        "H04" => capture!(
            Start::Scenario("s03_intervention"),
            "s03 ops conversation, approvals tool open",
            [],
            |run| {
                run.advance(1_000);
                run.show("c-ops")?;
                run.click("rail-approvals")
            }
        ),
        "K01" => capture!(
            S02,
            "s02 direct conversation with Lena, call button",
            ["conversation-call"],
            |run| {
                daily(run)?;
                run.show("c-lena")
            }
        ),
        "K06" => {
            capture!(S02, "s02 Lena, an active outgoing call (injected)", ["call-bar"], |run| {
                direct_call(run, call(CallState::Active, true))
            })
        }
        "K40" => capture!(
            S02,
            "s02 Lena offline, call button hovered",
            [],
            |run| {
                direct_call(run, DomainEvent::SourceStatusChanged(SourceStatus::Disconnected))?;
                run.hover("conversation-call")
            },
            keep
        ),
        "K41" => capture!(
            S02,
            "s02 Lena, an outgoing call that failed (injected)",
            ["call-failed"],
            |run| direct_call(run, call(CallState::Ended(CallEndReason::Failed), false))
        ),
        _ => return None,
    })
}
