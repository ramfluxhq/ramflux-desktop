// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Steps 1 to 3: attach, add and confirm the node, create the accounts.

use ramflux_desktop_core::{Listing, NodeId, SourceStatus, methods};
use ramflux_desktop_ui::route::{EntryStep, Page};

use super::{Demo, Facts, probe};
use crate::rehearsal::config::{fingerprint_matches, fingerprint_of};
use crate::rehearsal::daemon::DemoDaemon;
use crate::rehearsal::desk::Desk;
use crate::rehearsal::host::Host;
use crate::rehearsal::steps::Notes;

/// Step 1: a desktop per daemon, each on the node step with the daemon's
/// status shown.
pub fn start<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    notes: &mut Notes,
) -> Result<(), String> {
    for ix in 0..2 {
        let options = (demo.options)(&demo.sides[ix].daemon.socket());
        let desk = Desk::open(demo.host, demo.sides[ix].tag, options);
        demo.sides[ix].desk = Some(desk);
        let limit = demo.config.limits.answer;
        demo.wait(ix, limit, "the daemon and its node list", |desk, host| {
            desk.store(host, |s| {
                s.source_status() == SourceStatus::Connected && s.is_listed(Listing::Nodes)
            })
        })?;
        let desk = demo.sides[ix].desk()?;
        let page = desk.page(demo.host);
        if page != Page::Entry(EntryStep::Node) {
            return Err(format!("desktop {} opened on {page:?}, not the node step", desk.tag));
        }
        if !desk.exists(demo.host, "entry-daemon-info") {
            return Err(format!("desktop {} does not show its daemon", desk.tag));
        }
        notes.add(format!("{} on the node step", desk.tag));
        demo.shots(ix, "01_start", notes)?;
    }
    Ok(())
}

/// Step 2: each desktop looks the node up by URL, the shown fingerprint is
/// checked against the expected key, and only then confirmed (a node
/// trusted on first use needs the hold, in the strip, never a modal).
pub fn node<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    notes: &mut Notes,
) -> Result<(), String> {
    for ix in 0..2 {
        let url = demo.config.node_url.clone();
        let desk = demo.sides[ix].desk()?;
        let since = desk.store(demo.host, ramflux_desktop_core::Store::failures_total);
        desk.type_into(demo.host, "node-url", &url)?;
        desk.click(demo.host, None, "node-add")?;
        let limit = demo.config.limits.answer;
        let failed = |s: &ramflux_desktop_core::Store| {
            s.failures_since(since).iter().rev().find(|f| f.method == methods::NODE_ADD).cloned()
        };
        demo.wait(ix, limit, "the node's signed record", |desk, host| {
            desk.store(host, |s| s.nodes().any(|n| !n.confirmed) || failed(s).is_some())
        })?;
        if let Some(failure) = demo.read(ix, failed)? {
            let shown = demo.sides[ix].desk()?.label(demo.host, "entry-node-add-failure");
            demo.shots(ix, "02_node_lookup_failed", notes)?;
            return Err(format!(
                "desktop {}: the node lookup failed: {} (shown: {})",
                demo.sides[ix].tag,
                failure.message,
                shown.unwrap_or_else(|| "nothing".to_owned())
            ));
        }
        let node = demo
            .read(ix, |s| s.nodes().find(|n| !n.confirmed).cloned())?
            .ok_or("the discovered node went away")?;
        let shown = node.fingerprint.clone().unwrap_or_default();
        let card = format!("node-{}", node.id);
        let tag = demo.sides[ix].tag;
        if !demo.sides[ix].desk()?.exists_in(demo.host, &card, "node-confirm") {
            return Err(format!("desktop {tag} does not show the node card"));
        }
        if !fingerprint_matches(&demo.config.node_key, &shown) {
            demo.shots(ix, "02_node_key_mismatch", notes)?;
            return Err(format!(
                "desktop {tag}: shown fingerprint {shown:?} does not match the expected key; \
                 not confirmed"
            ));
        }
        notes.add(format!("{tag}: fingerprint {} matches", short(&shown)));
        demo.shots(ix, "02_node_card", notes)?;
        let desk = demo.sides[ix].desk()?;
        desk.click(demo.host, Some(&card), "node-confirm")?;
        if node.pin_source.is_trust_on_first_use() {
            if !desk.strip_open(demo.host) || !desk.exists(demo.host, "entry-confirm-footer") {
                return Err(format!("desktop {tag}: no hold strip for a first-use pin"));
            }
            demo.shots(ix, "02_node_hold", notes)?;
            demo.sides[ix].desk()?.hold_confirm(demo.host)?;
            notes.add(format!("{tag}: trusted on first use, confirmed with the hold"));
        }
        let id = node.id.clone();
        demo.wait(ix, limit, "the confirmed node", |desk, host| {
            desk.store(host, |s| s.node(&id).is_some_and(|n| n.confirmed))
        })?;
        cross_check_key(demo, ix, &node.id, &shown, notes)?;
        demo.wait(ix, limit, "the account step", |desk, host| {
            desk.page(host) == Page::Entry(EntryStep::Account)
        })?;
        demo.shots(ix, "02_node_confirmed", notes)?;
        demo.node = Some(node.id);
    }
    Ok(())
}

/// The key the daemon pinned must be the one whose fingerprint was shown.
fn cross_check_key<H: Host, D: DemoDaemon>(
    demo: &Demo<'_, H, D>,
    ix: usize,
    node: &NodeId,
    shown: &str,
    notes: &mut Notes,
) -> Result<(), String> {
    let side = &demo.sides[ix];
    match side.daemon.node_public_key(node.as_str())? {
        Some(key) if fingerprint_of(&key) == shown => {
            notes.add(format!("{}: the daemon pinned that key", side.tag));
            Ok(())
        }
        Some(_) => Err(format!("{}: the daemon pinned another key than the one shown", side.tag)),
        None => Err(format!("{}: the daemon does not list the confirmed node", side.tag)),
    }
}

/// Step 3: each desktop creates its account on the node and opens the
/// workspace; the reference CLI's account status agrees.
pub fn accounts<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    notes: &mut Notes,
) -> Result<(), String> {
    let node = demo.node.clone().ok_or("no confirmed node")?;
    for ix in 0..2 {
        let name = demo.sides[ix].label.clone();
        let secret = demo.sides[ix].passphrase.clone();
        create_with_phrase(demo, ix, &name, &secret)?;
        let limit = demo.config.limits.answer;
        demo.wait(ix, limit, "the new account's workspace", |desk, host| {
            desk.page(host) == Page::Workspace
                && desk.store(host, |s| {
                    probe::account(s).is_some_and(|a| {
                        a.label == name && a.self_participant_id.is_some() && a.device_id.is_some()
                    }) && probe::unlocked(s)
                })
        })?;
        let account = demo.read(ix, |s| probe::account(s).cloned())?.ok_or("no account")?;
        let principal = account.self_participant_id.map(|p| p.to_string()).unwrap_or_default();
        let status = demo.sides[ix].daemon.account_status(account.id.as_str())?;
        if status["node_id"] != node.as_str() || status["principal_id"] != principal.as_str() {
            return Err(format!("{name}: the daemon's account status disagrees: {status}"));
        }
        let text = |field: &str| status[field].as_str().unwrap_or_default().to_owned();
        demo.sides[ix].facts = Some(Facts {
            account: account.id.to_string(),
            principal: principal.clone(),
            device: account.device_id.unwrap_or_default(),
            commitment: text("principal_commitment"),
            target: text("target_delivery_id"),
        });
        notes.add(format!("{name} = {} on {node}", short(&principal)));
        if let Some(facts) = &demo.sides[ix].facts {
            notes.add(format!(
                "{name}: account {}, principal {}, device {}, target {}",
                facts.account, facts.principal, facts.device, facts.target
            ));
        }
        demo.shots(ix, "03_account", notes)?;
    }
    Ok(())
}

/// The first characters of a long id, for notes.
pub fn short(id: &str) -> String {
    let cut: String = id.chars().take(18).collect();
    if cut.len() < id.len() { format!("{cut}...") } else { cut }
}

/// Creates the account of desktop `ix` through the ceremony screens, as a
/// person does: the name, the 24 words read from the screen and written
/// down, three of them typed back, the passphrase twice, then the result
/// closed. The words come from the painted list (they exist nowhere else).
fn create_with_phrase<H: Host, D: DemoDaemon>(
    demo: &mut Demo<'_, H, D>,
    ix: usize,
    name: &str,
    passphrase: &str,
) -> Result<(), String> {
    use ramflux_desktop_core::CeremonyPhase;
    use ramflux_desktop_ui::i18n::tr_args;

    let limit = demo.config.limits.answer;
    let phase = |desk: &Desk, host: &mut H| {
        desk.entry_store(host, |s| s.ceremony().map(|c| c.phase.clone()))
    };
    let desk = demo.sides[ix].desk()?;
    desk.activate(demo.host)?;
    demo.wait(ix, limit, "creation with a recovery phrase offered", |desk, host| {
        desk.exists(host, "account-create-phrase")
    })?;
    let desk = demo.sides[ix].desk()?;
    desk.click(demo.host, None, "account-create-phrase")?;
    desk.type_into(demo.host, "ceremony-name-input", name)?;
    desk.click(demo.host, None, "ceremony-continue")?;
    demo.wait(ix, limit, "the ceremony", |desk, host| {
        phase(desk, host) == Some(CeremonyPhase::Prepared)
    })?;
    demo.sides[ix].desk()?.click(demo.host, None, "ceremony-show")?;
    demo.wait(ix, limit, "the 24 words", |desk, host| {
        desk.exists(host, "ceremony-word-cell-24")
            && phase(desk, host) == Some(CeremonyPhase::PhraseReady)
    })?;
    // Written down from the screen: the label of each cell is "Word N: word".
    let desk = demo.sides[ix].desk()?;
    let mut words = Vec::new();
    for number in 1..=24 {
        let id = format!("ceremony-word-cell-{number}");
        let label = desk.label(demo.host, &id).ok_or("a word cell has no label")?;
        let prefix =
            tr_args("ceremony.show.cell", &[("number", &number.to_string()), ("word", "")]);
        let word = label.strip_prefix(prefix.as_ref()).ok_or("a word cell is covered")?;
        words.push(word.to_owned());
    }
    desk.click(demo.host, None, "ceremony-hide")?;
    demo.wait(ix, limit, "the words asked", |desk, host| {
        matches!(phase(desk, host), Some(CeremonyPhase::Challenged { .. }))
    })?;
    let Some(CeremonyPhase::Challenged { positions }) = phase(demo.sides[ix].desk()?, demo.host)
    else {
        return Err("no words were asked".to_owned());
    };
    let desk = demo.sides[ix].desk()?;
    for (id, position) in
        ["ceremony-word-1", "ceremony-word-2", "ceremony-word-3"].into_iter().zip(positions)
    {
        let word = words.get(usize::from(position) - 1).ok_or("a position is out of range")?;
        desk.type_into(demo.host, id, word)?;
    }
    desk.click(demo.host, None, "ceremony-next")?;
    desk.type_into(demo.host, "ceremony-passphrase-input", passphrase)?;
    desk.type_into(demo.host, "ceremony-passphrase-repeat", passphrase)?;
    desk.click(demo.host, None, "ceremony-confirm-submit")?;
    demo.wait(ix, limit, "the result", |desk, host| {
        matches!(phase(desk, host), Some(CeremonyPhase::Created(_)))
            && desk.exists(host, "ceremony-done-close")
    })?;
    demo.sides[ix].desk()?.click(demo.host, None, "ceremony-done-close")
}
