// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Snapshot mapping against a fake daemon replaying recorded and SDK-shaped
//! frames: daemon scope (entry page) and account scope (workspace).

mod common;

use common::*;
use ramflux_desktop_core::{
    AccountId, ContextId, ContextKind, DomainEvent, GrantId, GrantState, LockState, MomentId,
    MomentKind, NodeId, ParticipantId, PinSource, Receipt, Role, SourceStatus, StandingApprovalId,
    methods,
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
    daemon.until("connected", |s, _| s.source_status() == SourceStatus::Connected);
    let info = daemon.store.daemon().cloned().expect("daemon info");
    assert_eq!(info.socket_path, "/tmp/ramflux-f2/rfd.sock");
    assert_eq!(info.open_accounts, 0);
    assert_eq!(daemon.store.nodes().count(), 0, "an empty node list");
    assert_eq!(daemon.store.accounts().count(), 0);
    assert!(daemon.store.unsupported_reason(methods::TASK_START).is_some());
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
    fake.shutdown();
}

#[test]
fn the_account_scope_maps_conversations_groups_messages_and_grants() {
    let path = socket_path("account");
    let fake = FakeBus::start(&path, |method, _, _| fixture(method));
    let hub = hub(&path);
    let mut account = Fed::new(hub.account_source(AccountId::new("acct-main")));
    account.until("messages of the pinned conversation", |s, _| {
        s.moment(&MomentId::new("msg:dm-ana:m2")).is_some()
    });
    let store = &account.store;
    let me = store.account(&AccountId::new("acct-main")).cloned().expect("account");
    assert_eq!(
        me.self_participant_id,
        Some(ParticipantId::new("device-me")),
        "self from device.list"
    );
    assert_eq!(me.device_id.as_deref(), Some("device-me"));
    let dm = store.context(&ContextId::new("dm-ana")).cloned().expect("dm");
    assert_eq!(dm.kind, ContextKind::Direct);
    assert!(dm.pinned, "pin_order from the daemon pins it");
    assert!(store.context(&ContextId::new("dm-old")).is_none(), "archived conversations stay out");
    let group = store.context(&ContextId::new("g-team")).cloned().expect("group");
    assert_eq!(group.kind, ContextKind::Group);
    assert_eq!(group.role_of(&ParticipantId::new("device-me")), Role::Owner);
    assert_eq!(group.epoch, Some(3));
    let inbound = store.moment(&MomentId::new("msg:dm-ana:m1")).cloned().expect("m1");
    let outbound = store.moment(&MomentId::new("msg:dm-ana:m2")).cloned().expect("m2");
    match (&inbound.kind, &outbound.kind) {
        (MomentKind::Message(a), MomentKind::Message(b)) => {
            assert_eq!(a.text, "hi");
            assert_eq!(b.text, "ok");
            assert_eq!(b.receipt, Receipt::Delivered, "own message carries its receipt");
        }
        other => panic!("messages expected, got {other:?}"),
    }
    assert_eq!(inbound.at.millis(), 1_790_000_000_000);
    let grant = store.grant(&GrantId::new("appr-1")).cloned().expect("approval");
    assert_eq!(grant.state, GrantState::Pending);
    assert_eq!(grant.scope, "files/delete");
    assert_eq!(grant.args_hash, "h-123");
    assert!(store.standing_approval(&StandingApprovalId::new("sa-1")).is_some());
    assert_eq!(store.devices().count(), 2);
    assert_eq!(store.integrations().mcp_servers[0].tools, ["read", "delete"]);
    assert!(store.participant(&ParticipantId::new("principal-ana")).is_some_and(|p| p.contact));
    assert!(store.rejections().is_empty(), "{:?}", store.rejections());
    assert!(
        account
            .envelopes
            .iter()
            .any(|e| matches!(e.event, DomainEvent::SourceStatusChanged(SourceStatus::Connected)))
    );
    fake.shutdown();
}
