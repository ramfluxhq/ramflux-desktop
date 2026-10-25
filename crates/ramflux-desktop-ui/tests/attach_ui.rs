// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The composer's attach button in the production window on a fake daemon:
//! the native file dialog picks a file, which is checked against what one
//! message carries and staged; Send sends it (its message shows it as
//! transferring until the daemon answered), removing it cancels it, a file
//! that is too large is never sent, and a failed send shows the reason on
//! the message's attachment.

mod common;
mod sdk_fake;

use std::path::PathBuf;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use ramflux_desktop_core::{ContextId, Direction, Lifecycle, MomentKind, Receipt, TransferState};
use ramflux_desktop_ui::route::Page;
use ramflux_desktop_ui::workspace::Staged;
use sdk_fake::wait;

type Handle = gpui_kit::WindowHandle<gpui_kit::base::Root>;
type ShellEntity = gpui_kit::Entity<ramflux_desktop_ui::Shell>;

struct Scratch(PathBuf);

impl Scratch {
    fn file(name: &str, len: usize) -> Self {
        let dir = std::env::temp_dir().join(format!("rf-f10-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join(format!("{name}.txt"));
        std::fs::write(&path, vec![b'a'; len]).expect("file");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().expect("dir"));
    }
}

fn open_ana(
    cx: &mut TestAppContext,
    name: &str,
) -> (Handle, ShellEntity, sdk_fake::Shared, ramflux_desktop_test::fake_bus::FakeBus) {
    let (fake, state, path) = sdk_fake::start(name);
    let (handle, shell) = open_with(cx, sdk_fake::options(&path));
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the conversation", |cx| {
        with_store(cx, &shell, |s| s.context(&ContextId::new("dm-ana")).is_some())
    });
    show_context(cx, handle, &shell, "dm-ana");
    (handle, shell, state, fake)
}

/// Clicks the attach button and answers the file dialog with `path`.
fn pick(cx: &mut TestAppContext, handle: Handle, path: &std::path::Path) {
    click(cx, handle, "composer-attach");
    assert!(cx.did_prompt_for_paths(), "the native file dialog opened");
    let path = path.to_path_buf();
    cx.simulate_path_prompt_response(move |options| {
        assert!(options.files && !options.directories && !options.multiple);
        Some(vec![path])
    });
    cx.run_until_parked();
}

fn staged(cx: &mut TestAppContext, shell: &ShellEntity) -> Option<Staged> {
    let workspace = workspace(cx, shell);
    cx.update(|cx| workspace.read(cx).composer().read(cx).staged().cloned())
}

fn label(cx: &mut TestAppContext, handle: Handle, id: &str) -> Option<String> {
    let id = id.to_owned();
    in_window(cx, handle, move |window, _| {
        window.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
}

fn submits(fake: &ramflux_desktop_test::fake_bus::FakeBus) -> Vec<serde_json::Value> {
    fake.seen().into_iter().filter(|s| s.method == "message.submit").map(|s| s.body).collect()
}

/// Whether the outgoing attachment message's row shows its attachment.
fn row_shows_attachment(cx: &mut TestAppContext, handle: Handle, shell: &ShellEntity) -> bool {
    let id = with_store(cx, shell, |s| {
        s.moments()
            .find(|m| {
                matches!(&m.kind, MomentKind::Message(p) if p.attachment.as_ref()
                    .and_then(|o| s.object(o))
                    .is_some_and(|o| o.direction == Direction::Outbound))
            })
            .map(|m| m.id.to_string())
    })
    .expect("outgoing message");
    exists_in(cx, handle, &format!("moment-{id}"), "message-attachment")
}

/// The outbound attachment message of `dm-ana`, with its object.
fn outgoing(
    cx: &mut TestAppContext,
    shell: &ShellEntity,
) -> Option<(Lifecycle, Receipt, ramflux_desktop_core::Object)> {
    with_store(cx, shell, |s| {
        s.moments().find_map(|m| match &m.kind {
            MomentKind::Message(p) if m.context_id.as_str() == "dm-ana" => {
                let object = s.object(p.attachment.as_ref()?)?;
                (object.direction == Direction::Outbound)
                    .then(|| (m.lifecycle, p.receipt, object.clone()))
            }
            _ => None,
        })
    })
}

#[gpui_kit::test]
fn a_picked_file_is_staged_then_sent_and_shown_while_it_travels(cx: &mut TestAppContext) {
    let (handle, shell, state, fake) = open_ana(cx, "ui-attach-ok");
    // The submission waits until the test has seen the message on its way
    // (a fixed delay raced on loaded runners).
    let held = state.lock().expect("state").submit_held.clone();
    held.store(true, std::sync::atomic::Ordering::SeqCst);
    let file = Scratch::file("report", 8192);
    pick(cx, handle, &file.0);
    assert!(matches!(staged(cx, &shell), Some(Staged::Ready { size: 8192, .. })));
    let chip = label(cx, handle, "composer-attachment").expect("chip");
    assert!(chip.starts_with("report.txt: "), "{chip}");
    assert!(submits(&fake).is_empty(), "nothing is sent before Send");
    click(cx, handle, "composer-send");
    wait(cx, "the message on its way", |cx| outgoing(cx, &shell).is_some());
    let (lifecycle, receipt, object) = outgoing(cx, &shell).expect("message");
    assert_eq!((lifecycle, receipt), (Lifecycle::InProgress, Receipt::Sending));
    assert_eq!(object.transfer, TransferState::Transferring { transferred: 0 }, "no bytes claimed");
    assert_eq!(object.name.as_deref(), Some("report.txt"));
    assert!(row_shows_attachment(cx, handle, &shell), "the attachment row is shown");
    assert_eq!(staged(cx, &shell), None, "the chip leaves once the message shows it");
    held.store(false, std::sync::atomic::Ordering::SeqCst);
    wait(cx, "the sent attachment", |cx| {
        outgoing(cx, &shell).is_some_and(|(l, r, o)| {
            l == Lifecycle::Done && r == Receipt::Sent && o.transfer == TransferState::Complete
        })
    });
    let sent = submits(&fake);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["attachments"][0]["name"], "report.txt");
    assert_eq!(sent[0]["conversation_id"], "dm-ana");
    fake.shutdown();
}

#[gpui_kit::test]
fn a_file_over_the_limit_is_refused_in_the_composer(cx: &mut TestAppContext) {
    let (handle, shell, _, fake) = open_ana(cx, "ui-attach-big");
    let file = Scratch::file("huge", ramflux_desktop_sdk::ATTACHMENT_LIMIT + 1);
    pick(cx, handle, &file.0);
    assert!(matches!(staged(cx, &shell), Some(Staged::TooLarge { .. })));
    let detail = label(cx, handle, "composer-attachment").expect("chip");
    let limit = ramflux_desktop_ui::i18n::size(Some(ramflux_desktop_sdk::ATTACHMENT_LIMIT as u64));
    assert!(detail.contains(&limit), "the limit is named: {detail}");
    click(cx, handle, "composer-send");
    advance(cx, 300);
    assert!(submits(&fake).is_empty(), "a file that does not fit is never sent");
    click(cx, handle, "composer-attachment-remove");
    assert_eq!(staged(cx, &shell), None);
    fake.shutdown();
}

#[gpui_kit::test]
fn removing_a_staged_file_cancels_it(cx: &mut TestAppContext) {
    let (handle, shell, _, fake) = open_ana(cx, "ui-attach-cancel");
    let file = Scratch::file("draft", 64);
    pick(cx, handle, &file.0);
    assert!(exists(cx, handle, "composer-attachment"));
    click(cx, handle, "composer-attachment-remove");
    assert_eq!(staged(cx, &shell), None);
    assert!(!exists(cx, handle, "composer-attachment"));
    type_into(cx, handle, "composer-input", "just text");
    click(cx, handle, "composer-send");
    wait(cx, "the text", |_| !submits(&fake).is_empty());
    advance(cx, 200);
    let sent = submits(&fake);
    assert_eq!(sent.len(), 1, "only the text went out");
    assert!(sent[0].get("attachments").is_none(), "{}", sent[0]);
    // A cancelled dialog stages nothing.
    click(cx, handle, "composer-attach");
    cx.simulate_path_prompt_response(|_| None);
    cx.run_until_parked();
    assert_eq!(staged(cx, &shell), None);
    fake.shutdown();
}

#[gpui_kit::test]
fn a_failed_send_shows_the_reason_on_the_attachment(cx: &mut TestAppContext) {
    let (handle, shell, state, fake) = open_ana(cx, "ui-attach-fail");
    state.lock().expect("state").fail_submit =
        Some("local bus error: relay object upload failed".to_owned());
    let file = Scratch::file("broken", 128);
    pick(cx, handle, &file.0);
    click(cx, handle, "composer-send");
    wait(cx, "the failed attachment", |cx| {
        outgoing(cx, &shell).is_some_and(|(l, _, o)| {
            l == Lifecycle::Failed && matches!(o.transfer, TransferState::Failed { .. })
        })
    });
    let (_, _, object) = outgoing(cx, &shell).expect("message");
    let TransferState::Failed { reason } = object.transfer else { unreachable!() };
    assert!(reason.contains("relay object upload failed"), "{reason}");
    assert!(row_shows_attachment(cx, handle, &shell));
    let failures = with_store(cx, &shell, |s| s.failures().len());
    assert_eq!(failures, 1, "the status strip has it too");
    fake.shutdown();
}
