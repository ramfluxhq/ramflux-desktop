// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Reducer arms for local settings, nodes, devices, revocations and audit.

mod common;

use common::*;
use ramflux_desktop_core::*;

fn node(id: &str, is_default: bool) -> Node {
    Node {
        id: NodeId::new(id),
        endpoints: Vec::new(),
        fingerprint: None,
        pin_source: PinSource::Invite,
        health: NodeHealth::Unknown,
        capabilities: Vec::new(),
        expires_at: None,
        confirmed: true,
        builtin: false,
        enabled: true,
        is_default,
        probe: None,
    }
}

fn outcome(ok: bool) -> ProbeOutcome {
    ProbeOutcome { ok, latency_ms: ok.then_some(12), detail: String::new() }
}

#[test]
fn context_settings_change_pin_mute_and_timer_in_place() {
    let mut feed = Feed::new();
    feed.ok(1, DomainEvent::ContextUpserted(context("c-1", false)));
    let changes = feed.ok(
        2,
        DomainEvent::ContextSettingsChanged {
            context_id: ContextId::new("c-1"),
            pinned: Some(true),
            muted: Some(true),
            disappearing: Some(Disappearing::After(86_400)),
        },
    );
    assert!(changes.pin_order);
    let context = feed.store.context(&ContextId::new("c-1")).expect("context");
    assert!(context.pinned && context.muted);
    assert_eq!(context.disappearing_secs, Some(86_400));
    assert_eq!(feed.store.pin_order(), [ContextId::new("c-1")]);

    feed.ok(
        3,
        DomainEvent::ContextSettingsChanged {
            context_id: ContextId::new("c-1"),
            pinned: Some(false),
            muted: None,
            disappearing: Some(Disappearing::Off),
        },
    );
    let context = feed.store.context(&ContextId::new("c-1")).expect("context");
    assert!(!context.pinned && context.muted, "unset fields stay unchanged");
    assert_eq!(context.disappearing_secs, None);
    assert!(feed.store.pin_order().is_empty());
}

#[test]
fn one_default_node_and_probe_sets_health() {
    let mut feed = Feed::new();
    feed.ok(1, DomainEvent::NodeUpserted(node("n-a", true)));
    feed.ok(1, DomainEvent::NodeUpserted(node("n-b", false)));
    let changes = feed.ok(
        2,
        DomainEvent::NodeSettingsChanged {
            node_id: NodeId::new("n-b"),
            enabled: None,
            is_default: Some(true),
        },
    );
    assert_eq!(changes.nodes.len(), 2, "the previous default changed too");
    let defaults: Vec<_> =
        feed.store.nodes().filter(|n| n.is_default).map(|n| n.id.clone()).collect();
    assert_eq!(defaults, [NodeId::new("n-b")]);

    let probe = |gateway, relay| DomainEvent::NodeProbed {
        node_id: NodeId::new("n-a"),
        probe: NodeProbe { at: Timestamp(3), gateway: outcome(gateway), relay: outcome(relay) },
    };
    for (gateway, relay, health) in [
        (true, true, NodeHealth::Healthy),
        (true, false, NodeHealth::Degraded),
        (false, false, NodeHealth::Unreachable),
    ] {
        feed.ok(3, probe(gateway, relay));
        assert_eq!(feed.store.node(&NodeId::new("n-a")).map(|n| n.health), Some(health));
    }
    feed.ok(4, DomainEvent::NodeRemoved { node_id: NodeId::new("n-a") });
    assert!(feed.store.node(&NodeId::new("n-a")).is_none());
    assert!(matches!(
        feed.push(5, DomainEvent::NodeRemoved { node_id: NodeId::new("n-a") }),
        Err(ApplyError::UnknownNode(_))
    ));
}

#[test]
fn only_approved_grants_revoke_and_audit_is_appended() {
    let mut feed = Feed::new();
    feed.ok(1, DomainEvent::GrantRequested(grant("g-1", Risk::High)));
    assert!(matches!(
        feed.push(2, DomainEvent::GrantRevoked { grant_id: GrantId::new("g-1") }),
        Err(ApplyError::GrantNotApproved(_))
    ));
    assert!(matches!(
        feed.push(
            3,
            DomainEvent::GrantResolved {
                grant_id: GrantId::new("g-1"),
                state: GrantState::Revoked
            }
        ),
        Err(ApplyError::InvalidGrantResolution(_))
    ));
    feed.ok(
        4,
        DomainEvent::GrantResolved { grant_id: GrantId::new("g-1"), state: GrantState::Approved },
    );
    feed.ok(5, DomainEvent::GrantRevoked { grant_id: GrantId::new("g-1") });
    assert_eq!(feed.store.grant(&GrantId::new("g-1")).map(|g| g.state), Some(GrantState::Revoked));

    let changes = feed.ok(
        6,
        DomainEvent::AuditAppended(AuditEntry {
            at: Timestamp(6),
            action: methods::GRANT_REVOKE.to_owned(),
            subject: "g-1".to_owned(),
            proposed_by: Actor::Human,
            human_confirmed: true,
        }),
    );
    assert!(changes.audit);
    assert_eq!(feed.store.audit().len(), 1);
}

#[test]
fn lock_changes_need_a_known_account_and_devices_upsert() {
    let mut feed = Feed::new();
    assert!(matches!(
        feed.push(
            1,
            DomainEvent::AccountLockChanged {
                account_id: AccountId::new("a-x"),
                lock_state: LockState::Locked,
            }
        ),
        Err(ApplyError::UnknownAccount(_))
    ));
    let device = Device {
        id: DeviceId::new("dev-1"),
        account_id: AccountId::new("a-1"),
        label: "Laptop".to_owned(),
        this_device: false,
        revoked: true,
        last_seen: None,
    };
    let changes = feed.ok(2, DomainEvent::DeviceUpserted(device));
    assert!(changes.devices.contains(&DeviceId::new("dev-1")));
    assert!(feed.store.device(&DeviceId::new("dev-1")).is_some_and(|d| d.revoked));
}
