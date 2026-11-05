// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Trace coverage is compared against frames actually received by the fake daemon.
use super::*;
use crate::bus::{
    BusRequest, client::BusClient, deadline::Deadlines, jobs::JobLink, stream, subscription,
};
use ramflux_desktop_test::fake_bus::{FakeBus, Reply, socket_path};
use serde_json::json;
use std::time::Duration;
use tokio::sync::mpsc;

#[tokio::test]
async fn trace_accounts_for_ordinary_stream_and_job_sends() {
    let path = socket_path("trace-sends");
    let fake = FakeBus::start(&path, |_, _, _| Reply::Ok(json!({})));
    fake.enable_stream();
    let deadlines = Deadlines::standard();
    let client = BusClient::new(path.clone(), Duration::from_secs(2), deadlines);
    client
        .call(&BusRequest::new(
            None,
            "daemon",
            "daemon.status",
            json!({"secret":"never-record-this"}),
        ))
        .await
        .expect("read");
    let mut keyed = BusRequest::new(None, "node", "node.probe", json!({}));
    keyed.operation_id = Some("never-record-operation-key".into());
    client.call(&keyed).await.expect("keyed request");
    let (out, mut events) = mpsc::unbounded_channel();
    let socket = path.clone();
    let stream = tokio::spawn(async move {
        stream::run(
            &socket,
            stream::Subscription {
                account: "acct-main".into(),
                topics: vec!["account.state"],
                since_seq: Some(0),
            },
            Duration::from_secs(2),
            deadlines,
            out,
        )
        .await;
    });
    assert!(matches!(events.recv().await, Some(stream::Streamed::Opened { .. })));
    let (out, mut pushed) = mpsc::unbounded_channel();
    let socket = path.clone();
    let push = tokio::spawn(async move {
        subscription::run(&socket, "acct-main".into(), Duration::from_secs(2), deadlines, out)
            .await;
    });
    assert_eq!(pushed.recv().await, Some(subscription::Pushed::Opened));
    let (out, mut job_events) = mpsc::unbounded_channel();
    let jobs = JobLink::new(path.clone(), Duration::from_secs(2), deadlines, out);
    jobs.ensure().await.expect("job subscription");
    assert!(matches!(job_events.recv().await, Some(crate::bus::jobs::JobStreamed::Opened { .. })));
    let _ = jobs
        .call(&BusRequest::new(
            None,
            "account",
            "account.backup.cancel",
            json!({"job_id":"missing"}),
        ))
        .await;
    // Attended setup and its data request go through the same send hook too.
    crate::bus::attended::call(
        &path,
        Duration::from_secs(2),
        deadlines,
        &BusRequest::new(
            Some("acct-main"),
            "account",
            "account.export_agent_credentials",
            json!({}),
        ),
    )
    .await
    .expect("attended request");
    client.call(&BusRequest::new(None, "daemon", "daemon.stop", json!({}))).await.expect("control");
    let entries = after(&path, 0);
    let seen = fake.seen();
    assert_eq!(entries.len(), seen.len(), "every actual frame appears exactly once");
    assert_eq!(entries.len(), 9);
    for method in [
        "daemon.status",
        "node.probe",
        "bus.subscribe",
        "subscription.open",
        "account.backup.cancel",
        "account.export_agent_credentials",
    ] {
        assert_eq!(
            entries.iter().filter(|(_, e)| e.method == method).count(),
            seen.iter().filter(|s| s.method == method).count(),
            "{method}"
        );
    }
    for (connection, frame, count) in [
        ("ordinary", "control", 1),
        ("ordinary", "data", 2),
        ("stream", "subscription", 1),
        ("push", "subscription", 1),
        ("job", "subscription", 1),
        ("job", "data", 1),
        ("attended", "setup", 1),
        ("attended", "data", 1),
    ] {
        assert_eq!(
            entries
                .iter()
                .filter(|(_, e)| e.connection_class == connection && e.frame_class == frame)
                .count(),
            count,
            "{connection}/{frame}"
        );
    }
    assert!(entries.iter().any(|(_, e)| e.method == "node.probe" && e.keyed));
    assert!(entries.windows(2).all(|w| w[0].1.monotonic_ms <= w[1].1.monotonic_ms));
    let json =
        serde_json::to_string(&entries.iter().map(|(_, e)| e).collect::<Vec<_>>()).expect("export");
    assert!(!json.contains("never-record"));
    for (_, entry) in entries {
        let json = serde_json::to_value(entry).expect("metadata");
        assert_eq!(json.as_object().expect("entry").len(), 6);
    }
    stream.abort();
    push.abort();
    jobs.close().await;
}
