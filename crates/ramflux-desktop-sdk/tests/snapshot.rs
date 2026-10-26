// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Snapshot mapping against a fake daemon replaying recorded and SDK-shaped
//! frames: daemon scope (entry page) and account scope (workspace).

mod common;

use common::*;
use ramflux_desktop_core::{
    AccountId, ContextId, ContextKind, Direction, DomainEvent, GrantId, GrantState, Lifecycle,
    ListState, Listing, LockState, MomentId, MomentKind, NodeId, ObjectId, ParticipantId,
    ParticipantKind, PinSource, Receipt, Role, SafetyState, Source as _, SourceStatus,
    StandingApprovalId, TransferState, methods, selectors,
};

#[test]
fn an_empty_daemon_reaches_the_node_step_with_its_real_status() {
    let path = socket_path("empty");
    let recorded = frames();
    let fake = FakeBus::start(&path, move |method, _, _| {
        Reply::Ok(recorded[&format!("{method}#recorded")].clone())
    });
    let hub = hub(&path);
    let mut daemon = Fed::new(hub.daemon_source());
    daemon.until("connected and listed", |s, _| {
        s.source_status() == SourceStatus::Connected
            && s.is_listed(Listing::Nodes)
            && s.is_listed(Listing::Accounts)
    });
    let info = daemon.store.daemon().cloned().expect("daemon info");
    assert_eq!(info.socket_path, "/tmp/ramflux-f2/rfd.sock");
    assert_eq!(info.open_accounts, 0);
    assert_eq!(daemon.store.nodes().count(), 0, "an empty node list");
    assert_eq!(
        ListState::of(daemon.store.is_listed(Listing::Nodes), true),
        ListState::Empty,
        "an answered empty node list is empty, not loading"
    );
    assert_eq!(daemon.store.accounts().count(), 0);
    assert!(daemon.store.unsupported_reason(methods::PRIVACY_REVEAL).is_some());
    assert!(daemon.store.unsupported_reason(methods::TASK_START).is_none());
    assert!(daemon.store.unsupported_reason(methods::NODE_ADD).is_none());
    assert_eq!(daemon.names()[0], "CapabilitiesUpdated", "capabilities come first");
    let seqs: Vec<u64> = daemon.envelopes.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, (1..=seqs.len() as u64).collect::<Vec<_>>(), "gapless seq from 1");
    fake.shutdown();
}

#[test]
fn the_daemon_scope_maps_nodes_and_accounts() {
    let path = socket_path("daemon");
    let fake = FakeBus::start(&path, |method, _, _| fixture(method));
    let hub = hub(&path);
    let mut daemon = Fed::new(hub.daemon_source());
    daemon.until("account node", |s, _| {
        s.account(&AccountId::new("acct-main")).is_some_and(|a| a.node_id.is_some())
    });
    let node = daemon.store.node(&NodeId::new("n-harbor")).cloned().expect("node");
    assert!(node.confirmed && node.is_default && node.enabled && !node.builtin);
    assert_eq!(node.pin_source, PinSource::Invite);
    assert_eq!(node.capabilities, ["gateway", "relay", "prekey"]);
    assert!(node.endpoints.contains(&"harbor.example.org:7443".to_owned()));
    assert!(node.fingerprint.as_deref().is_some_and(|f| !f.is_empty()));
    assert_eq!(node.expires_at.map(|t| t.millis()), Some(1_890_000_000_000));
    let account = daemon.store.account(&AccountId::new("acct-main")).cloned().expect("account");
    assert_eq!(account.lock_state, LockState::Unlocked);
    assert_eq!(account.node_id, Some(NodeId::new("n-harbor")));
    assert_eq!(account.label, "Me", "the label the daemon keeps");
    // Locked accounts are listed too (`account_details`), for the switcher.
    let work = daemon.store.account(&AccountId::new("acct-work")).cloned().expect("locked");
    assert_eq!((work.lock_state, work.label.as_str()), (LockState::Locked, "Work"));
    assert_eq!(work.kind, ramflux_desktop_core::AccountKind::HeadlessAi);
    let old = daemon.store.account(&AccountId::new("acct-old")).cloned().expect("locked");
    assert_eq!(
        (old.lock_state, old.label.as_str()),
        (LockState::Locked, "acct-old"),
        "no label: the id"
    );
    fake.shutdown();
}

#[test]
fn the_account_scope_maps_the_directories_messages_and_grants() {
    let path = socket_path("account");
    let fake = FakeBus::start(&path, |method, _, _| fixture(method));
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.source.focus(Some(&ContextId::new("dm-ana")));
    account.until("messages of the focused conversation", |s, _| {
        s.moment(&MomentId::new("msg:dm-ana:m2")).is_some()
    });
    account.until("every account listing", |s, _| {
        [Listing::Contexts, Listing::Contacts, Listing::Approvals, Listing::Objects]
            .into_iter()
            .all(|l| s.is_listed(l))
    });
    let store = &account.store;
    let me = store.account(&AccountId::new("acct-main")).cloned().expect("account");
    assert_eq!(
        me.self_participant_id,
        Some(ParticipantId::new("principal-me")),
        "self is this account's principal, not a device"
    );
    assert_eq!(me.device_id.as_deref(), Some("device-me"));
    assert_eq!(me.label, "Me", "the label the account's own identity declares");

    let dm = store.context(&ContextId::new("dm-ana")).cloned().expect("dm");
    assert_eq!(dm.kind, ContextKind::Direct);
    assert_eq!(dm.title, "dm-ana", "no title is reported: the id is shown, nothing invented");
    assert_eq!(
        dm.participants,
        [ParticipantId::new("principal-ana"), ParticipantId::new("principal-me")]
    );
    assert_eq!(dm.unread, 1);
    assert!(!dm.pinned, "the daemon reports no pins");
    assert!(store.context(&ContextId::new("dm-old")).is_none(), "archived conversations stay out");
    let group = store.context(&ContextId::new("thread-team")).cloned().expect("group conversation");
    assert_eq!(group.kind, ContextKind::Group);
    assert_eq!(group.title, "Team");
    assert!(group.muted);
    assert_eq!(group.disappearing_secs, Some(3600));
    assert_eq!(group.role_of(&ParticipantId::new("principal-me")), Role::Owner);
    assert_eq!(group.role_of(&ParticipantId::new("principal-bo")), Role::Member);
    assert_eq!(group.epoch, Some(3));
    assert!(store.context(&ContextId::new("g-team")).is_none(), "a group id is not a conversation");

    let inbound = store.moment(&MomentId::new("msg:dm-ana:m1")).cloned().expect("m1");
    let outbound = store.moment(&MomentId::new("msg:dm-ana:m2")).cloned().expect("m2");
    assert_eq!(inbound.origin, ParticipantId::new("principal-ana"), "sender device to principal");
    assert_eq!(outbound.origin, ParticipantId::new("principal-me"));
    match (&inbound.kind, &outbound.kind) {
        (MomentKind::Message(_), MomentKind::Message(b)) => {
            assert_eq!(store.body(&inbound.id), Some("hi"), "bodies live in the body cache");
            assert_eq!(store.body(&outbound.id), Some("ok"));
            assert_eq!(b.receipt, Receipt::Delivered, "own message carries its receipt");
        }
        other => panic!("messages expected, got {other:?}"),
    }
    assert_eq!(inbound.at.millis(), 1_790_000_000_000);

    let ana = store.participant(&ParticipantId::new("principal-ana")).cloned().expect("ana");
    assert_eq!((ana.display.as_str(), ana.kind), ("principal-ana", ParticipantKind::Unknown));
    assert!(ana.contact, "the accepted link");
    let bo = store.participant(&ParticipantId::new("principal-bo")).cloned().expect("bo");
    assert_eq!((bo.kind, bo.safety), (ParticipantKind::Agent, SafetyState::Verified));
    assert_eq!(bo.safety_number.as_deref(), Some("12345 67890"));
    let cy = store.participant(&ParticipantId::new("principal-cy")).cloned().expect("cy");
    assert_eq!(cy.safety, SafetyState::VerificationStale, "a verified device set that changed");
    let own = store.participant(&ParticipantId::new("principal-me")).cloned().expect("me");
    assert_eq!((own.display.as_str(), own.kind), ("Me", ParticipantKind::Human));
    assert!(
        store.participant(&ParticipantId::new("device-ana")).is_none(),
        "senders are not participants"
    );

    let grant = store.grant(&GrantId::new("appr-1")).cloned().expect("approval");
    assert_eq!(grant.state, GrantState::Pending);
    assert_eq!(grant.scope, "files/delete");
    assert_eq!(grant.args_hash, "h-123");
    let intervention = store.moment(&MomentId::new("approval:appr-1")).cloned().expect("moment");
    assert_eq!(
        intervention.context_id,
        ContextId::new("dm-ana"),
        "in the conversation it came from"
    );
    assert_eq!(intervention.lifecycle, Lifecycle::NeedsMe);
    assert!(matches!(&intervention.kind, MomentKind::Intervention(p) if p.grant_id == grant.id));
    assert!(
        selectors::attention(store).iter().any(|a| a.moment_id == intervention.id),
        "the approval is in needs me"
    );
    assert!(store.grant(&GrantId::new("appr-2")).is_some(), "an approval without a conversation");
    assert!(store.moment(&MomentId::new("approval:appr-2")).is_none(), "stays in the dock only");
    assert!(store.standing_approval(&StandingApprovalId::new("sa-1")).is_some());
    assert_eq!(store.devices().count(), 2);

    let notes = store.object(&ObjectId::new("attachment:m1:0")).cloned().expect("object");
    assert_eq!((notes.name.as_deref(), notes.size), (Some("notes.txt"), Some(12)));
    assert_eq!((notes.direction, &notes.transfer), (Direction::Inbound, &TransferState::Complete));
    let sent = store.object(&ObjectId::new("attachment:m2:0")).cloned().expect("sent object");
    assert_eq!((sent.name, sent.size), (None, None), "unknown name and size stay unknown");
    assert_eq!(sent.direction, Direction::Outbound);
    assert_eq!(sent.transfer, TransferState::Failed { reason: "paused".to_owned() });
    assert!(store.object(&ObjectId::new("legacy-object")).is_none(), "no observed direction");

    let servers = &store.integrations().mcp_servers;
    let files = servers.iter().find(|s| s.name == "files").expect("files");
    assert_eq!(
        (files.tools.as_slice(), files.healthy),
        (["read".to_owned(), "delete".to_owned()].as_slice(), None)
    );
    assert_eq!(servers.iter().find(|s| s.name == "search").and_then(|s| s.healthy), Some(true));
    assert!(store.rejections().is_empty(), "{:?}", store.rejections());
    assert!(
        account
            .envelopes
            .iter()
            .any(|e| matches!(e.event, DomainEvent::SourceStatusChanged(SourceStatus::Connected)))
    );
    let reads: Vec<String> = fake.seen().into_iter().map(|s| s.method).collect();
    assert!(!reads.iter().any(|m| m == "conversation.list"), "the directory replaces it");
    fake.shutdown();
}
