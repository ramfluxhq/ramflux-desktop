// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! One sample of every MLS group event, intent and UI event.
#![allow(dead_code)]

use ramflux_desktop_core::mls_group::*;
use ramflux_desktop_core::*;

pub fn author() -> Author {
    Author { anchor: "anc-1".to_owned(), display: None, ai_flag: true, is_me: false }
}

pub fn summary() -> MlsGroupSummary {
    MlsGroupSummary {
        group_id: "g-1".to_owned(),
        name: "Design".to_owned(),
        tier: 1000,
        members: 12,
        leaves: 0,
        agents: 1,
        approval: true,
        my_role: MlsRole::Owner,
        host_node: "node.example".to_owned(),
        push: PushSetting::All,
        catch_up: Some(CatchUp { state: "running".to_owned(), done: 1, total: 2 }),
        privacy_delay: PrivacyDelay::Waiting(Timestamp(5)),
        quota_post: Some(Quota { limit: 20, remaining: 1, first_day: true, resets_at: None }),
        quota_commit: None,
        last_log_seq: 9,
        warnings: vec![Warning {
            code: "host_ip_visible".to_owned(),
            params: serde_json::json!({}),
            cleared: false,
        }],
        last_activity_at: Some(Timestamp(7)),
        unread: 2,
        unread_mentions: 1,
        read_log_seq: 7,
        last_post: Some(LastPost {
            log_seq: 9,
            kind: "text".to_owned(),
            author: Some(author()),
            snippet: Some("hi".to_owned()),
            appended_at: Some(Timestamp(7)),
        }),
    }
}

pub fn post() -> Post {
    Post {
        log_seq: Some(3),
        post_id: "p-1".to_owned(),
        author: Some(author()),
        appended_at: Some(Timestamp(4)),
        kind: PostKind::Text,
        body: PostBody::Text("hello".to_owned()),
        reply_to: None,
        mentions: vec![],
        state: PostState::Sent,
        send: None,
    }
}

pub fn invite() -> Invite {
    Invite {
        invite_id: "i-1".to_owned(),
        link: InviteLink::new("rfinv-x"),
        max_uses: 10,
        uses_left: Some(10),
        expires_at: Timestamp(99),
        revoked: false,
    }
}

pub fn member() -> MemberRow {
    MemberRow {
        anchor: "anc-1".to_owned(),
        display: None,
        role: MlsRole::Member,
        ai_flag: false,
        devices: 2,
        is_me: false,
    }
}

pub fn join_request(id: &str, anchor: &str) -> JoinRequest {
    JoinRequest {
        request_id: OpaqueId::new(id),
        anchor: OpaqueId::new(anchor),
        display: Some("Ana".to_owned()),
        ai_flag: false,
        requested_at: Some(Timestamp(100)),
        expires_at: Some(Timestamp(86_400_100)),
    }
}

pub fn events() -> Vec<DomainEvent> {
    vec![
        DomainEvent::MlsStarted(MlsStart::Join { attempt: 1 }),
        DomainEvent::MlsGroupsListed { rows: vec![summary()], cursor: None, first_page: true },
        DomainEvent::MlsFailed {
            method: "mls_group.info".to_owned(),
            group_id: Some("g-1".to_owned()),
            invite_id: None,
            code: "not_member".to_owned(),
        },
        DomainEvent::MlsGroupInfoRead { summary: Box::new(summary()), issued: 0 },
        DomainEvent::MlsMembersPage {
            group_id: "g-1".to_owned(),
            rows: vec![member()],
            cursor: Some("p2".to_owned()),
            failed: None,
        },
        DomainEvent::MlsHistoryPage {
            group_id: "g-1".to_owned(),
            direction: HistoryDirection::Before(5),
            posts: vec![post()],
            floor_log_seq: Some(1),
            last_log_seq: 9,
            more: true,
        },
        DomainEvent::MlsGroupEvent {
            seq: 4,
            group_id: "g-1".to_owned(),
            log_seq: Some(3),
            kind: GroupEventKind::Post(Box::new(post())),
        },
        DomainEvent::MlsCreated {
            group_id: "g-1".to_owned(),
            name: "Design".to_owned(),
            approval: true,
            invite: invite(),
        },
        DomainEvent::MlsCreateFailed { code: "tier_unavailable".to_owned() },
        DomainEvent::MlsInviteCreated { group_id: "g-1".to_owned(), invite: invite() },
        DomainEvent::MlsInvitesListed { group_id: "g-1".to_owned(), invites: vec![invite()] },
        DomainEvent::MlsInviteRevoked { group_id: "g-1".to_owned(), invite_id: "i-1".to_owned() },
        DomainEvent::MlsJoinOutcome {
            attempt: 2,
            outcome: JoinOutcome::Refused(JoinRefusal::QueueFull),
        },
        DomainEvent::MlsJoinPendingListed {
            records: vec![JoinRecord {
                request_id: "r-1".to_owned(),
                reason: PendingReason::AwaitingAdmin,
                expires_at: None,
                group_id: Some("g-1".to_owned()),
                attempt: 0,
            }],
        },
        DomainEvent::MlsSendOutcome {
            group_id: "g-1".to_owned(),
            local_id: "l-1".to_owned(),
            post_id: Some("p-1".to_owned()),
            state: SendState::Queued(QueueReason::CatchingUp),
        },
        DomainEvent::MlsDeleteOutcome {
            group_id: "g-1".to_owned(),
            post_id: "p-1".to_owned(),
            result: DeleteResult::Committed { log_seq: 8 },
        },
        DomainEvent::MlsLeft { group_id: "g-1".to_owned(), result: LeaveResult::Committed },
        DomainEvent::MlsRefreshed { group_id: "g-1".to_owned(), last_log_seq: 9 },
        DomainEvent::MlsMarkedRead {
            group_id: "g-1".to_owned(),
            read_log_seq: 9,
            unread: 0,
            unread_mentions: 0,
        },
        DomainEvent::MlsUnavailable {
            method: "mls_group.send".to_owned(),
            group_id: Some("g-1".to_owned()),
            attempt: None,
            local_id: Some("l-1".to_owned()),
        },
        DomainEvent::MlsRequestsPage {
            group_id: "g-1".to_owned(),
            page_gen: 1,
            first_page: true,
            rows: vec![join_request("r-1", "anc-r1")],
            cursor: Some("c2".to_owned()),
            failed: None,
        },
        DomainEvent::MlsAdminOutcome {
            epoch: 0,
            action_id: 1,
            group_id: "g-1".to_owned(),
            kind: AdminKind::Approve,
            target: AdminTarget::Request(OpaqueId::new("r-1")),
            result: AdminResult::Committed { log_seq: 301 },
        },
        DomainEvent::MlsDeclineOutcome {
            epoch: 0,
            group_id: "g-1".to_owned(),
            request_id: OpaqueId::new("r-1"),
            result: DeclineResult::Declined,
        },
        DomainEvent::MlsPushOutcome {
            epoch: 0,
            group_id: "g-1".to_owned(),
            result: PushResult::Set(PushSetting::None),
        },
        DomainEvent::MlsSkipOutcome {
            epoch: 0,
            group_id: "g-1".to_owned(),
            from_seq: 2,
            result: SkipResult::Rejoining,
        },
    ]
}

pub fn intents() -> Vec<IntentKind> {
    vec![
        IntentKind::MlsGroupList { cursor: None },
        IntentKind::MlsGroupInfo { group_id: "g-1".to_owned() },
        IntentKind::MlsGroupMembers { group_id: "g-1".to_owned(), cursor: None },
        IntentKind::MlsGroupHistory {
            group_id: "g-1".to_owned(),
            direction: HistoryDirection::Newest,
        },
        IntentKind::MlsGroupRefresh { group_id: "g-1".to_owned() },
        IntentKind::MlsGroupMarkRead { group_id: "g-1".to_owned(), log_seq: 3 },
        IntentKind::MlsGroupCreate { name: "Design".to_owned(), approval: true },
        IntentKind::MlsInviteCreate { group_id: "g-1".to_owned(), max_uses: 10, ttl_s: 86_400 },
        IntentKind::MlsInviteList { group_id: "g-1".to_owned() },
        IntentKind::MlsInviteRevoke { group_id: "g-1".to_owned(), invite_id: "i-1".to_owned() },
        IntentKind::MlsGroupJoin { link: InviteLink::new("rfinv-x"), attempt: 1 },
        IntentKind::MlsJoinPending,
        IntentKind::MlsGroupSend {
            group_id: "g-1".to_owned(),
            text: "hi".to_owned(),
            reply_to: None,
            mentions: vec![],
            local_id: "l-1".to_owned(),
            retry_of: None,
        },
        IntentKind::MlsGroupDelete { group_id: "g-1".to_owned(), post_id: "p-1".to_owned() },
        IntentKind::MlsGroupLeave { group_id: "g-1".to_owned() },
        IntentKind::MlsRequestsList { group_id: "g-1".to_owned(), cursor: None },
        IntentKind::MlsRequestApprove {
            group_id: "g-1".to_owned(),
            request_id: OpaqueId::new("r-1"),
        },
        IntentKind::MlsRequestDecline {
            group_id: "g-1".to_owned(),
            request_id: OpaqueId::new("r-1"),
        },
        IntentKind::MlsRoleSet {
            group_id: "g-1".to_owned(),
            anchor: OpaqueId::new("anc-1"),
            admin: true,
        },
        IntentKind::MlsMemberRemove { group_id: "g-1".to_owned(), anchor: OpaqueId::new("anc-1") },
        IntentKind::MlsMemberBan { group_id: "g-1".to_owned(), anchor: OpaqueId::new("anc-1") },
        IntentKind::MlsSettingsSet {
            group_id: "g-1".to_owned(),
            field: SettingsField::Name("Team".to_owned()),
        },
        IntentKind::MlsPushSet { group_id: "g-1".to_owned(), push: PushSetting::All },
        IntentKind::MlsCatchUpSkip { group_id: "g-1".to_owned() },
        IntentKind::MlsVoid { group_id: "g-1".to_owned(), log_seq: 250 },
    ]
}

pub fn ui_events() -> Vec<UiEventKind> {
    vec![
        UiEventKind::MlsListRequested { cursor: None },
        UiEventKind::MlsInfoRequested { group_id: "g-1".to_owned() },
        UiEventKind::MlsMembersRequested { group_id: "g-1".to_owned(), cursor: None },
        UiEventKind::MlsHistoryRequested {
            group_id: "g-1".to_owned(),
            direction: HistoryDirection::After(3),
        },
        UiEventKind::MlsRefreshPressed { group_id: "g-1".to_owned() },
        UiEventKind::MlsMarkReadRequested { group_id: "g-1".to_owned(), log_seq: 3 },
        UiEventKind::MlsCreatePressed { name: "Design".to_owned(), approval: false },
        UiEventKind::MlsInviteCreatePressed {
            group_id: "g-1".to_owned(),
            max_uses: 1,
            ttl_s: 604_800,
        },
        UiEventKind::MlsInviteListRequested { group_id: "g-1".to_owned() },
        UiEventKind::MlsInviteRevokePressed {
            group_id: "g-1".to_owned(),
            invite_id: "i-1".to_owned(),
        },
        UiEventKind::MlsJoinPressed { link: InviteLink::new("rfinv-x"), attempt: 1 },
        UiEventKind::MlsJoinPendingRequested,
        UiEventKind::MlsPanelRequested {
            panel: MlsPanel::ConfirmLeave { group_id: "g-1".to_owned() },
        },
        UiEventKind::MlsRetryPressed {
            context: ContextId::new("mls:g-1"),
            local_id: "l-1".to_owned(),
        },
        UiEventKind::MlsDeletePressed { group_id: "g-1".to_owned(), post_id: "p-1".to_owned() },
        UiEventKind::MlsLeavePressed { group_id: "g-1".to_owned() },
        UiEventKind::MlsRequestsRequested { group_id: "g-1".to_owned(), cursor: None },
        UiEventKind::MlsApprovePressed {
            group_id: "g-1".to_owned(),
            request_id: OpaqueId::new("r-1"),
        },
        UiEventKind::MlsDeclinePressed {
            group_id: "g-1".to_owned(),
            request_id: OpaqueId::new("r-1"),
        },
        UiEventKind::MlsRolePressed {
            group_id: "g-1".to_owned(),
            anchor: OpaqueId::new("anc-1"),
            admin: false,
        },
        UiEventKind::MlsRemovePressed {
            group_id: "g-1".to_owned(),
            anchor: OpaqueId::new("anc-1"),
        },
        UiEventKind::MlsBanPressed { group_id: "g-1".to_owned(), anchor: OpaqueId::new("anc-1") },
        UiEventKind::MlsSettingsPressed {
            group_id: "g-1".to_owned(),
            field: SettingsField::Approval(false),
        },
        UiEventKind::MlsPushPressed { group_id: "g-1".to_owned(), push: PushSetting::None },
        UiEventKind::MlsSkipPressed { group_id: "g-1".to_owned() },
        UiEventKind::MlsVoidPressed { group_id: "g-1".to_owned(), log_seq: 250 },
    ]
}
