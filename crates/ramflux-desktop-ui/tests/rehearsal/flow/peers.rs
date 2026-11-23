// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The node's command-line peers with one desktop: Alice's desktop (side
//! 0) and the two test accounts the node keeps (`peers.json`: names,
//! accounts, principals, devices and daemon sockets), driven with the
//! reference CLI on their own daemons.
//!
//! 20. The desktop adds each peer from the contact card the peer's daemon
//!     exports (typed into Add contact, as a person pastes it); each peer
//!     accepts the desktop's request and adds the desktop's card back (the
//!     card pins the desktop's home node, which files need). The trust state
//!     the desktop shows is noted as the daemon reports it.
//! 21. Direct messages both ways with the first peer, live on the desktop.
//! 22. An image and a file each way with the first peer: the desktop sends
//!     with the attach button, the peer imports with `rf attachment import`;
//!     the peer sends with `rf dm send --attach`, the desktop downloads with
//!     the Download button; bytes compared both ways.
//! 23. A group with both peers made in the desktop's New group form;
//!     messages both ways (peers with `rf group send`).
//! 24. Settings › Account and › Devices show the account's real data.
//!
//! Peers are shared test accounts: nothing is removed from them, every name
//! and text carries the run id, and their daemons are never stopped.

use std::path::{Path, PathBuf};
use std::process::Command;

use ramflux_desktop_core::{ContactLink, ContextId, ParticipantId};
use ramflux_desktop_ui::i18n::tr;
use serde_json::Value;

use super::setup::short;
use super::{Demo, files, probe};
use crate::rehearsal::daemon::DemoDaemon;
use crate::rehearsal::host::Host;
use crate::rehearsal::steps::Notes;

/// The file that lists the node's test peers (switches the peer steps on).
pub const PEERS_ENV: &str = "RAMFLUX_DEMO_PEERS";

/// One command-line peer.
#[derive(Debug, Clone)]
pub struct Peer {
    /// Its name in `peers.json`.
    pub name: String,
    /// Its local account id.
    pub account: String,
    /// Its principal (the participant id the desktop shows).
    pub principal: String,
    /// Its device.
    pub device: String,
    socket: PathBuf,
    rf: PathBuf,
}

impl Peer {
    /// The peers listed in the file `PEERS_ENV` names, with the `rf` binary
    /// of the live daemons; `Ok(None)` when the variable is not set.
    ///
    /// # Errors
    ///
    /// Why the file could not be read.
    pub fn from_env() -> Result<Option<Vec<Self>>, String> {
        let Some(path) = std::env::var_os(PEERS_ENV) else { return Ok(None) };
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", Path::new(&path).display()))?;
        let listed: Value = serde_json::from_str(&text).map_err(|e| format!("peers file: {e}"))?;
        let rf = ramflux_desktop_test::live_daemon::rf_binary()?;
        let peers = listed["peers"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| Self {
                name: p["name"].as_str().unwrap_or_default().to_owned(),
                account: p["account_id"].as_str().unwrap_or_default().to_owned(),
                principal: p["principal_id"].as_str().unwrap_or_default().to_owned(),
                device: p["device_id"].as_str().unwrap_or_default().to_owned(),
                socket: PathBuf::from(p["daemon_socket"].as_str().unwrap_or_default()),
                rf: rf.clone(),
            })
            .collect::<Vec<_>>();
        if peers.len() < 2 {
            return Err("the peers file lists fewer than two peers".to_owned());
        }
        Ok(Some(peers))
    }

    /// Runs `rf` on this peer's daemon and parses its JSON answer.
    ///
    /// # Errors
    ///
    /// The CLI's failure, with its error text.
    pub fn rf(&self, args: &[&str]) -> Result<Value, String> {
        let out = Command::new(&self.rf)
            .arg("--socket")
            .arg(&self.socket)
            .args(args)
            .output()
            .map_err(|e| format!("cannot run rf: {e}"))?;
        if !out.status.success() {
            let verb = args.iter().take(2).copied().collect::<Vec<_>>().join(" ");
            return Err(format!(
                "peer {} rf {verb} failed: {}",
                self.name,
                String::from_utf8_lossy(&out.stderr)
                    .trim()
                    .lines()
                    .take(4)
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
        }
        if out.stdout.iter().all(u8::is_ascii_whitespace) {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&out.stdout).map_err(|e| format!("rf answered no JSON: {e}"))
    }

    fn account_flag(&self) -> String {
        format!("--account={}", self.account)
    }
}

/// What the peer steps share.
#[derive(Debug, Default)]
pub struct PeerRun {
    /// The peers (the first is the one messages and files go to).
    pub peers: Vec<Peer>,
    /// The direct conversation with the first peer.
    pub conversation: Option<ContextId>,
}

/// Step 20: both peers added from their cards, each answering back.
pub fn contacts<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    run: &mut PeerRun,
    notes: &mut Notes,
) -> Result<(), String> {
    let me = demo.sides[0].facts()?.clone();
    let my_card = super::talk::shown_card(demo, 0, notes)?;
    for peer in run.peers.clone() {
        let answer = peer.rf(&["contact", "card", &peer.account_flag()])?;
        let card = answer["card"].as_str().ok_or("the peer exported no card")?.to_owned();
        let desk = demo.sides[0].desk()?;
        if !desk.exists(demo.host, "contact-add-id") {
            desk.click(demo.host, None, "rail-contacts")?;
        }
        desk.type_into(demo.host, "contact-add-id", &card)?;
        let kind = desk.label(demo.host, "contact-add-kind").unwrap_or_default();
        desk.click(demo.host, None, "contact-add")?;
        let participant = ParticipantId::new(&peer.principal);
        let listed = demo.wait(0, demo.config.limits.answer, "the peer listed", |desk, host| {
            desk.store(host, |s| s.participant(&participant).is_some())
        });
        if let Err(error) = listed {
            let note = demo.sides[0].desk()?.label(demo.host, "contact-add-note");
            return Err(note.map_or(error.clone(), |note| format!("{error}: {note}")));
        }
        notes.add(format!(
            "desktop added peer {} from the card its daemon exported ({} bytes); the form said: {kind}",
            peer.name,
            card.len()
        ));
        // The peer answers the desktop's request and pins its card.
        let accepted = peer.rf(&[
            "contact",
            "accept",
            &peer.account_flag(),
            &format!("--principal={}", me.principal),
        ]);
        notes.add(format!(
            "peer {}: contact accept {}",
            peer.name,
            accepted.as_ref().map_or_else(|e| format!("refused: {e}"), |_| "done".to_owned())
        ));
        let pinned =
            peer.rf(&["contact", "add", &peer.account_flag(), &format!("--card={my_card}")]);
        notes.add(format!(
            "peer {}: contact add --card (the desktop's card) {}",
            peer.name,
            pinned.as_ref().map_or_else(|e| format!("refused: {e}"), |_| "done".to_owned())
        ));
        let added = demo.wait(
            0,
            demo.config.limits.answer,
            "the peer accepted and reachable",
            |desk, host| {
                desk.store(host, |s| {
                    s.participant(&participant).is_some_and(|p| {
                        p.link_state() == ContactLink::Accepted && p.reachable == Some(true)
                    })
                })
            },
        )?;
        notes.add(format!("desktop: peer {} accepted and reachable after {added:?}", peer.name));
        let shown = demo.read(0, |s| {
            s.participant(&participant).map(|p| (p.safety, p.root_source, p.reachable))
        })?;
        notes.add(format!(
            "desktop shows peer {}: (safety, root, reachable) = {shown:?}",
            peer.name
        ));
        let row = format!("contacts-{}", peer.principal);
        let status = demo.sides[0].desk()?.label_in(demo.host, &row, "contact-root");
        let safety = demo.sides[0].desk()?.label_in(demo.host, &row, "contact-safety");
        notes.add(format!("status on the row {row}: {safety:?}"));
        notes.add(format!("trust line on the row {row}: {status:?}"));
    }
    demo.shots(0, "20_peer_contacts", notes)?;
    Ok(())
}

/// Step 21: direct messages both ways with the first peer.
pub fn messages<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    run: &mut PeerRun,
    notes: &mut Notes,
) -> Result<(), String> {
    let peer = run.peers.first().cloned().ok_or("no peer")?;
    let id = demo.config.run_id.clone();
    let opener = format!("Hello {} from the desktop ({id})", peer.name);
    let row = format!("contacts-{}", peer.principal);
    let desk = demo.sides[0].desk()?;
    if !desk.exists(demo.host, &row) {
        desk.click(demo.host, None, "rail-contacts")?;
    }
    desk.click(demo.host, Some(&row), "message")?;
    let desk = demo.sides[0].desk()?;
    if desk.draft_open(demo.host) {
        desk.type_into(demo.host, "composer-input", &opener)?;
        desk.click(demo.host, None, "composer-send")?;
        demo.wait(0, demo.config.limits.answer, "the started conversation", |desk, host| {
            !desk.draft_open(host) && desk.current_context(host).is_some()
        })?;
    } else {
        desk.type_into(demo.host, "composer-input", &opener)?;
        desk.click(demo.host, None, "composer-send")?;
    }
    let context =
        demo.sides[0].desk()?.current_context(demo.host).ok_or("no conversation shown")?;
    run.conversation = Some(context.clone());
    let me = demo.sides[0].facts()?.principal.clone();
    let shown = context.clone();
    let opener_text = opener.clone();
    demo.wait(0, demo.config.limits.answer, "the opener accepted", |desk, host| {
        desk.store(host, |s| {
            probe::receipt(s, &shown, &opener_text, &me)
                .is_some_and(|r| r != ramflux_desktop_core::Receipt::Sending)
        })
    })?;
    notes.add(format!("desktop sent {opener:?} in conversation {}", short(context.as_str())));
    let seen = wait_cli(demo, || {
        let page =
            peer.rf(&["dm", "read", &peer.account_flag(), &format!("--conversation={context}")])?;
        Ok(page_texts(&page).iter().any(|t| t == &opener))
    })?;
    notes.add(format!("peer {} read it with rf dm read ({seen:?})", peer.name));
    let reply = format!("Reply from {} ({id})", peer.name);
    peer.rf(&[
        "conversation",
        "send",
        &peer.account_flag(),
        &format!("--conversation={context}"),
        &format!("--text={reply}"),
        &format!("--operation-id=rehearsal-{id}-reply"),
    ])?;
    let waited =
        demo.wait(0, demo.config.limits.answer, "the peer's reply, live", |desk, host| {
            desk.store(host, |s| probe::message(s, &context, &reply).is_some())
        })?;
    notes.add(format!(
        "peer {} replied with rf conversation send; the desktop showed it from its event stream in {waited:?}",
        peer.name
    ));
    demo.shots(0, "21_peer_messages", notes)?;
    Ok(())
}

/// Step 22: an image and a file each way with the first peer.
pub fn files_both_ways<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    run: &mut PeerRun,
    notes: &mut Notes,
) -> Result<(), String> {
    let peer = run.peers.first().cloned().ok_or("no peer")?;
    let context = run.conversation.clone().ok_or("no conversation with the peer")?;
    let id = demo.config.run_id.clone();
    let mut failures = Vec::new();
    // Desktop to peer.
    let (image, image_bytes) = files::image_file(&demo.work, &format!("desk-{id}.png"))?;
    let (file, file_bytes) = files::text_file(&demo.work, &format!("desk-{id}.txt"), 600 * 1024)?;
    for (path, bytes) in [(&image, &image_bytes), (&file, &file_bytes)] {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        if let Err(error) = files::send_file(demo, 0, &context, path, notes) {
            failures.push(error);
            continue;
        }
        match peer_import(demo, &peer, &context, &name) {
            Ok(got) if &got == bytes => {
                notes.add(format!(
                    "peer {} imported {name}: {} bytes identical",
                    peer.name,
                    got.len()
                ));
            }
            Ok(got) => failures.push(format!(
                "peer {} imported {name} with different bytes ({} vs {})",
                peer.name,
                got.len(),
                bytes.len()
            )),
            Err(error) => failures.push(error),
        }
    }
    demo.shots(0, "22_desktop_sent_files", notes)?;
    // Peer to desktop.
    let (image, image_bytes) = files::image_file(&demo.work, &format!("{}-{id}.png", peer.name))?;
    let (file, file_bytes) =
        files::text_file(&demo.work, &format!("{}-{id}.txt", peer.name), 300 * 1024)?;
    for (n, (path, bytes)) in [(&image, &image_bytes), (&file, &file_bytes)].into_iter().enumerate()
    {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        let message = format!("rehearsal-{id}-file-{n}");
        let sent = peer.rf(&[
            "dm",
            "send",
            &peer.account_flag(),
            &format!("--conversation={context}"),
            &format!("--message={message}"),
            &format!("--envelope={message}"),
            &format!("--source-principal={}", peer.principal),
            &format!("--sender={}", peer.device),
            "--target=unused",
            &format!("--body=file from {}", peer.name),
            &format!("--attach={}", path.display()),
        ]);
        match sent {
            Ok(answer) => notes.add(format!(
                "peer {} sent {name} with rf dm send --attach ({})",
                peer.name, answer["delivery_state"]
            )),
            Err(error) => {
                failures.push(error);
                continue;
            }
        }
        match files::receive_file(demo, 0, &context, &name, notes) {
            Ok(got) if &got == bytes => {
                notes.add(format!("desktop: {name} identical to what peer {} sent", peer.name));
            }
            Ok(got) => failures.push(format!(
                "desktop downloaded {name} with different bytes ({} vs {})",
                got.len(),
                bytes.len()
            )),
            Err(error) => failures.push(error),
        }
    }
    demo.shots(0, "22_desktop_received_files", notes)?;
    if failures.is_empty() { Ok(()) } else { Err(failures.join("; ")) }
}

/// The peer finds the desktop's message carrying `name` and imports the
/// file with `rf attachment import`.
fn peer_import<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    peer: &Peer,
    context: &ContextId,
    name: &str,
) -> Result<Vec<u8>, String> {
    let mut found = None;
    wait_cli(demo, || {
        let page =
            peer.rf(&["dm", "read", &peer.account_flag(), &format!("--conversation={context}")])?;
        found = media_message(&page, name);
        Ok(found.is_some())
    })
    .map_err(|e| format!("peer {} never listed {name}: {e}", peer.name))?;
    let (message, index) = found.ok_or("no message")?;
    let out = demo.work.join(format!("peer-{}-{name}", peer.name));
    peer.rf(&[
        "attachment",
        "import",
        &peer.account_flag(),
        &format!("--message={message}"),
        &format!("--index={index}"),
        &format!("--output={}", out.display()),
    ])?;
    std::fs::read(&out).map_err(|e| format!("cannot read {}: {e}", out.display()))
}

/// The message of a read-model page carrying a file named `name`, and the
/// file's index.
fn media_message(page: &Value, name: &str) -> Option<(String, usize)> {
    page["messages"].as_array()?.iter().find_map(|row| {
        let media = row["content"]["message"]["media"].as_array()?;
        let index = media.iter().position(|m| {
            m["fileName"].as_str().or_else(|| m["file_name"].as_str()) == Some(name)
        })?;
        Some((row["id"].as_str()?.to_owned(), index))
    })
}

/// The texts of a read-model page.
fn page_texts(page: &Value) -> Vec<String> {
    page["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["text"].as_str().map(str::to_owned))
        .collect()
}

/// Repeats `check` (a CLI read) while the desktop keeps running, until it
/// holds or the answer limit passes.
fn wait_cli<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    mut check: impl FnMut() -> Result<bool, String>,
) -> Result<std::time::Duration, String> {
    let start = std::time::Instant::now();
    let mut last = String::new();
    while start.elapsed() < demo.config.limits.answer {
        match check() {
            Ok(true) => return Ok(start.elapsed()),
            Ok(false) => {}
            Err(error) => last = error,
        }
        let pause = std::time::Instant::now();
        while pause.elapsed() < std::time::Duration::from_secs(2) {
            demo.host.tick();
        }
    }
    Err(format!("timed out after {:?} {last}", demo.config.limits.answer))
}

/// Step 23: a group with both peers from the desktop's New group form.
pub fn group<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    run: &mut PeerRun,
    notes: &mut Notes,
) -> Result<(), String> {
    let id = demo.config.run_id.clone();
    let name = format!("Peers group {id}");
    let desk = demo.sides[0].desk()?;
    desk.click(demo.host, None, "stack-new")?;
    desk.click_label(demo.host, &tr("stack.new_group"))?;
    let Some(Some(context)) = desk.group_form(demo.host) else {
        return Err("New group… opened no form".to_owned());
    };
    desk.type_into(demo.host, "new-group-name", &name)?;
    for peer in &run.peers {
        let member = format!("new-group-member-{}", peer.principal);
        demo.sides[0]
            .desk()?
            .click(demo.host, None, &member)
            .map_err(|e| format!("{e}: peer {} is not offered as a member", peer.name))?;
    }
    demo.shots(0, "23_new_group", notes)?;
    demo.sides[0].desk()?.click(demo.host, None, "new-group-create")?;
    let created = demo.wait(0, demo.config.limits.answer, "the group shown", |desk, host| {
        desk.current_context(host).as_ref() == Some(&context)
    });
    if let Err(error) = created {
        let status = demo.sides[0].desk()?.label(demo.host, "new-group-status");
        return Err(status.map_or(error.clone(), |s| format!("{error}: {s}")));
    }
    notes.add(format!(
        "desktop made {name:?} with both peers; conversation {}",
        short(context.as_str())
    ));
    let opener = format!("Group hello from the desktop ({id})");
    let desk = demo.sides[0].desk()?;
    desk.type_into(demo.host, "composer-input", &opener)?;
    desk.click(demo.host, None, "composer-send")?;
    let me = demo.sides[0].facts()?.principal.clone();
    let (shown, text) = (context.clone(), opener.clone());
    demo.wait(0, demo.config.limits.answer, "the group message accepted", |desk, host| {
        desk.store(host, |s| {
            probe::receipt(s, &shown, &text, &me)
                .is_some_and(|r| r != ramflux_desktop_core::Receipt::Sending)
        })
    })?;
    let mut failures = Vec::new();
    for (n, peer) in run.peers.clone().into_iter().enumerate() {
        let mut group_id = None;
        let listed = wait_cli(demo, || {
            let groups = peer.rf(&["group", "list", &peer.account_flag()])?;
            group_id = groups["groups"].as_array().into_iter().flatten().find_map(|g| {
                let conversation = g["conversation_id"].as_str();
                let ids = g["conversation_ids"].as_array();
                let matches = conversation == Some(context.as_str())
                    || ids.is_some_and(|ids| ids.iter().any(|c| c == context.as_str()))
                    || g["id"] == context.as_str()
                    || g["group_id"] == context.as_str();
                matches.then(|| {
                    g["group_id"].as_str().or_else(|| g["id"].as_str()).map(str::to_owned)
                })?
            });
            Ok(group_id.is_some())
        });
        let Some(group) = group_id.filter(|_| listed.is_ok()) else {
            failures.push(format!(
                "peer {} does not list the group: {}",
                peer.name,
                listed.err().unwrap_or_default()
            ));
            continue;
        };
        let read = wait_cli(demo, || {
            let page = peer.rf(&[
                "group",
                "read",
                &peer.account_flag(),
                &format!("--group={group}"),
                &format!("--conversation={context}"),
            ])?;
            Ok(page_texts(&page).iter().any(|t| t == &opener))
        });
        notes.add(format!("peer {} read the desktop's group message: {read:?}", peer.name));
        let reply = format!("Group reply from {} ({id})", peer.name);
        let sent = peer.rf(&[
            "group",
            "send",
            &peer.account_flag(),
            &format!("--group={group}"),
            &format!("--conversation={context}"),
            &format!("--message=rehearsal-{id}-group-{n}"),
            &format!("--sender={}", peer.device),
            &format!("--body={reply}"),
        ]);
        if let Err(error) = sent {
            failures.push(error);
            continue;
        }
        let seen =
            demo.wait(0, demo.config.limits.answer, "the peer's group reply", |desk, host| {
                desk.store(host, |s| probe::message(s, &context, &reply).is_some())
            });
        match seen {
            Ok(after) => notes
                .add(format!("desktop showed peer {}'s group reply after {after:?}", peer.name)),
            Err(error) => failures.push(error),
        }
    }
    demo.shots(0, "23_peer_group", notes)?;
    if failures.is_empty() { Ok(()) } else { Err(failures.join("; ")) }
}

/// Step 24: Settings › Account and › Devices with the account's data.
pub fn settings<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    notes: &mut Notes,
) -> Result<(), String> {
    use ramflux_desktop_ui::settings::SettingsSection;
    let facts = demo.sides[0].facts()?.clone();
    let desk = demo.sides[0].desk()?;
    desk.open_settings(demo.host, SettingsSection::Account);
    let account = demo.wait(0, demo.config.limits.answer, "the account page", |desk, host| {
        desk.exists(host, "rotate-current") || desk.exists(host, "backup-passphrase")
    });
    let listed = demo.read(0, |s| s.accounts().map(|a| a.id.to_string()).collect::<Vec<_>>())?;
    notes.add(format!("account page drawn: {}; accounts listed {listed:?}", account.is_ok()));
    demo.shots(0, "24_settings_account", notes)?;
    let desk = demo.sides[0].desk()?;
    desk.open_settings(demo.host, SettingsSection::Devices);
    let row = format!("device-{}", facts.device);
    let devices =
        demo.wait(0, demo.config.limits.answer, "this device on the devices page", |desk, host| {
            desk.exists(host, &row)
        });
    let count = demo.read(0, |s| s.devices().count())?;
    notes.add(format!(
        "devices page: this device drawn {}; {count} device(s) listed",
        devices.is_ok()
    ));
    demo.shots(0, "24_settings_devices", notes)?;
    demo.sides[0].desk()?.close_settings(demo.host);
    account.and(devices).map(drop)
}

/// Step 25: history paged on scroll. The first peer writes more messages
/// than one read-model page holds; the desktop quits and opens again, so it
/// starts from the newest page; scrolling the conversation up reads the
/// older pages (`message.read` with `before_order_key`) until the first of
/// those messages is shown.
pub fn history<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    run: &mut PeerRun,
    notes: &mut Notes,
) -> Result<(), String> {
    const COUNT: usize = 120;
    let peer = run.peers.first().cloned().ok_or("no peer")?;
    let context = run.conversation.clone().ok_or("no conversation with the peer")?;
    let id = demo.config.run_id.clone();
    let text = |n: usize| format!("History {n:03} from {} ({id})", peer.name);
    for n in 1..=COUNT {
        peer.rf(&[
            "conversation",
            "send",
            &peer.account_flag(),
            &format!("--conversation={context}"),
            &format!("--text={}", text(n)),
            &format!("--operation-id=rehearsal-{id}-history-{n}"),
        ])?;
    }
    let last = text(COUNT);
    let shown = context.clone();
    demo.wait(0, demo.config.limits.answer, "the last history message, live", |desk, host| {
        desk.store(host, |s| probe::message(s, &shown, &last).is_some())
    })?;
    notes.add(format!("peer {} sent {COUNT} messages; the desktop showed them live", peer.name));
    // Open the desktop again: it starts from the newest page.
    let desk = demo.sides[0].desk.take().ok_or("the desktop is not open")?;
    desk.close(demo.host)?;
    let closed = std::time::Instant::now();
    while closed.elapsed() < std::time::Duration::from_secs(2) {
        demo.host.tick();
    }
    let options = (demo.options)(&demo.sides[0].daemon.socket());
    demo.sides[0].desk =
        Some(crate::rehearsal::desk::Desk::open(demo.host, demo.sides[0].tag, options));
    demo.wait(0, demo.config.limits.answer, "the reopened desktop", |desk, host| {
        let page = desk.page(host);
        page == ramflux_desktop_ui::route::Page::Workspace
            || matches!(page, ramflux_desktop_ui::route::Page::Entry(_))
    })?;
    if demo.sides[0].desk()?.page(demo.host) != ramflux_desktop_ui::route::Page::Workspace {
        let secret = demo.sides[0].passphrase.clone();
        let desk = demo.sides[0].desk()?;
        desk.type_into(demo.host, "passphrase", &secret)?;
        desk.click(demo.host, None, "unlock")?;
        notes.add("the reopened desktop showed the account locked; unlocked");
    }
    let shown = context.clone();
    demo.wait(0, demo.config.limits.answer, "the conversation", |desk, host| {
        desk.page(host) == ramflux_desktop_ui::route::Page::Workspace
            && desk.store(host, |s| s.context(&shown).is_some())
    })?;
    demo.sides[0].desk()?.show_context(demo.host, &context)?;
    let last = text(COUNT);
    let shown = context.clone();
    demo.wait(0, demo.config.limits.answer, "the newest page", |desk, host| {
        desk.store(host, |s| probe::message(s, &shown, &last).is_some())
    })?;
    let count = |s: &ramflux_desktop_core::Store| {
        s.context_moments(&context)
            .filter(|m| matches!(m.kind, ramflux_desktop_core::MomentKind::Message(_)))
            .count()
    };
    let first_page = demo.read(0, count)?;
    notes.add(format!("after reopening, {first_page} messages loaded (the newest page)"));
    demo.shots(0, "25_history_newest", notes)?;
    let first = text(1);
    let start = std::time::Instant::now();
    let mut scrolls = 0;
    while start.elapsed() < demo.config.limits.answer {
        if demo.read(0, |s| probe::message(s, &context, &first).is_some())? {
            break;
        }
        demo.sides[0].desk()?.scroll_conversation_to_top(demo.host)?;
        scrolls += 1;
        let pause = std::time::Instant::now();
        while pause.elapsed() < std::time::Duration::from_millis(300) {
            demo.host.tick();
        }
    }
    let loaded = demo.read(0, count)?;
    let reached = demo.read(0, |s| probe::message(s, &context, &first).is_some())?;
    notes.add(format!(
        "{scrolls} scrolls to the top: {loaded} messages loaded; the first history message shown: {reached}"
    ));
    demo.shots(0, "25_history_older", notes)?;
    if !reached || loaded <= first_page {
        return Err(format!("older history was not paged in ({first_page} then {loaded})"));
    }
    Ok(())
}
