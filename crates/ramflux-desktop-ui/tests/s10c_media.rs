// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Media rows of message schema v1 (S10c c04): the thumbnail policy (local
//! opt-in, then codec, byte size, width, height, then decode), and the
//! revoked or unavailable state an object refusal gives a row.

mod common;
mod sdk_fake;

use common::*;
use ramflux_desktop_core::{
    AccountId, ContextId, DomainEvent, MediaState, MediaView, Moment, MomentId, MomentKind,
};
use ramflux_desktop_sdk::Scope;
use ramflux_desktop_sdk::bus::BusError;
use ramflux_desktop_sdk::translate::Translator;
use ramflux_desktop_sdk::translate::v1::candidate::{apply, media_failed};
use ramflux_desktop_sdk::translate::v1::media::REVOKED_REASON;
use ramflux_desktop_test::v1_fixtures::{
    identity, media_ref, message_update, text_message, thumbnail,
};
use ramflux_desktop_ui::route::Page;
use ramflux_protocol::message::{MediaRef, MessageUpdate};

fn translator(inline: bool) -> Translator {
    let mut translator =
        Translator::new(Scope::Account(AccountId::new("acct-main")), "/tmp/s10c-media.sock");
    translator.set_inline_thumbnails(inline);
    translator
}

fn update(n: u8, media: Vec<MediaRef>) -> MessageUpdate {
    let mut message = text_message("see attached", Vec::new());
    message.media = media;
    message_update(identity("c-media", &[n; 4], "dev-ana", u64::from(n), u64::from(n)), message)
}

fn moment_id(n: u8) -> MomentId {
    MomentId::new(format!("msg:c-media:{}", format!("{n:02x}").repeat(4)))
}

fn rows(events: Vec<DomainEvent>) -> Vec<MediaView> {
    let mut moments = events.into_iter().filter_map(|event| match event {
        DomainEvent::MomentAdded(moment) => Some(*moment),
        DomainEvent::MomentUpdated { kind: Some(kind), moment_id, .. } => Some(Moment {
            id: moment_id,
            context_id: ContextId::new("c-media"),
            origin: ramflux_desktop_core::ParticipantId::new("x"),
            at: ramflux_desktop_core::Timestamp(0),
            kind,
            lifecycle: ramflux_desktop_core::Lifecycle::Done,
            order: None,
        }),
        _ => None,
    });
    let moment = moments.next().expect("one moment");
    match moment.kind {
        MomentKind::Message(payload) => payload.media,
        other => panic!("not a message: {other:?}"),
    }
}

/// The media rows of a message with one thumbnail, and how many thumbnails
/// were decoded and kept.
fn with_thumbnail(
    inline: bool,
    thumb: ramflux_protocol::message::Thumbnail,
) -> (Vec<MediaView>, usize, usize) {
    let mut translator = translator(inline);
    let events = apply(
        &mut translator,
        &[update(1, vec![media_ref("o-1", "cat.png", "image/png", 2048, Some(thumb))])],
    )
    .events;
    let kept = translator.cached_thumbnail_bytes(&moment_id(1));
    (rows(events), translator.thumbnail_decodes(), kept)
}

fn png(width: u32, height: u32, len: usize) -> ramflux_protocol::message::Thumbnail {
    thumbnail("image/png", width, height, len)
}

#[test]
fn thumbnail_off_by_default_gives_a_file_row_and_decodes_nothing() {
    let (rows, decodes, kept) = with_thumbnail(false, png(64, 64, 100));
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].name.as_str(), rows[0].size), ("cat.png", 2048));
    assert!(rows[0].thumbnail.is_none(), "opt-in off: no thumbnail");
    assert_eq!((decodes, kept), (0, 0), "decode counter is 0 with the opt-in off");
}

#[test]
fn unknown_codec_gives_a_file_row() {
    let (rows, decodes, _) = with_thumbnail(true, thumbnail("image/gif", 64, 64, 100));
    assert!(rows[0].thumbnail.is_none());
    assert_eq!(decodes, 0, "an unknown codec is never decoded");
}

#[test]
fn thumbnail_of_16384_bytes_is_shown() {
    let (rows, decodes, kept) = with_thumbnail(true, png(64, 64, 16_384));
    assert_eq!(rows[0].thumbnail.as_ref().map(|t| t.bytes.len()), Some(16_384));
    assert_eq!((decodes, kept), (1, 16_384));
}

#[test]
fn thumbnail_of_16385_bytes_is_a_file_row_with_no_decode_and_no_bytes() {
    let (rows, decodes, kept) = with_thumbnail(true, png(64, 64, 16_385));
    assert!(rows[0].thumbnail.is_none(), "no thumbnail bytes in MediaView");
    assert_eq!(decodes, 0, "decode counter is 0 for 16,385 B");
    assert_eq!(kept, 0, "no thumbnail bytes kept");
}

fn refused_side(width: u32, height: u32) {
    let (rows, decodes, kept) = with_thumbnail(true, png(width, height, 100));
    assert!(rows[0].thumbnail.is_none(), "{width}x{height} is a file row");
    assert_eq!((decodes, kept), (0, 0), "{width}x{height} is checked before any decode");
}

#[test]
fn thumbnail_width_0_gives_a_file_row() {
    refused_side(0, 64);
}

#[test]
fn thumbnail_width_513_gives_a_file_row() {
    refused_side(513, 64);
}

#[test]
fn thumbnail_height_0_gives_a_file_row() {
    refused_side(64, 0);
}

#[test]
fn thumbnail_height_513_gives_a_file_row() {
    refused_side(64, 513);
}

#[test]
fn thumbnail_of_512_by_512_is_shown() {
    let (rows, decodes, _) = with_thumbnail(true, png(512, 512, 100));
    let shown = rows[0].thumbnail.as_ref().expect("thumbnail");
    assert_eq!((shown.width, shown.height, decodes), (512, 512, 1));
}

#[test]
fn bytes_that_are_not_the_declared_codec_give_a_file_row() {
    let mut wrong = png(64, 64, 100);
    wrong.encoded_image = vec![0; 100];
    let (rows, decodes, kept) = with_thumbnail(true, wrong);
    assert!(rows[0].thumbnail.is_none());
    assert_eq!((decodes, kept), (1, 0), "decoded once, nothing kept");
}

#[test]
fn v1_media_rows_thumbnail_policy_and_revoked() {
    let mut translator = translator(true);
    let first = media_ref("o-1", "cat.png", "image/png", 2048, Some(png(64, 64, 100)));
    let second = media_ref("o-2", "notes.pdf", "application/pdf", 10, None);
    let added = apply(&mut translator, &[update(1, vec![first, second])]).events;
    let shown = rows(added);
    assert_eq!(shown.len(), 2);
    assert!(shown[0].thumbnail.is_some() && shown[0].state == MediaState::Available);
    assert!(shown[1].thumbnail.is_none() && shown[1].state == MediaState::Available);
    assert!(translator.cached_thumbnail_bytes(&moment_id(1)) > 0);

    // The typed revoke refusal revokes the row and keeps no thumbnail bytes.
    let revoked = BusError::Daemon {
        retry_after_ms: None,
        code: "Refused".to_owned(),
        message: "access revoked".to_owned(),
        reason_code: Some(REVOKED_REASON.to_owned()),
        details: None,
        cause: None,
    };
    let after = rows(media_failed(&mut translator, "o-1", &revoked).events);
    assert_eq!(after[0].state, MediaState::Revoked);
    assert!(after[0].thumbnail.is_none(), "no thumbnail kept");
    assert_eq!(after[1].state, MediaState::Available, "the other object is untouched");
    assert_eq!(translator.cached_thumbnail_bytes(&moment_id(1)), 0, "no cached object bytes");

    // Any other failure is only unavailable.
    let after = rows(media_failed(&mut translator, "o-2", &BusError::Timeout).events);
    assert_eq!(after[1].state, MediaState::Unavailable);
    assert_eq!(after[0].state, MediaState::Revoked, "a revoked row stays revoked");
}

const DM: &str = "dm-ana";

fn dm_id(n: u8) -> MomentId {
    MomentId::new(format!("msg:{DM}:{}", format!("{n:02x}").repeat(4)))
}

fn open_with_media(
    cx: &mut gpui_kit::TestAppContext,
    inline: bool,
) -> (
    ramflux_desktop_test::fake_bus::FakeBus,
    std::path::PathBuf,
    gpui_kit::WindowHandle<gpui_kit::base::Root>,
    gpui_kit::Entity<ramflux_desktop_ui::Shell>,
) {
    let (fake, _state, path) =
        sdk_fake::start(if inline { "s10c-media-on" } else { "s10c-media-off" });
    let mut options = sdk_fake::options(&path);
    if let ramflux_desktop_ui::shell::SourceChoice::Rfd(config) = &mut options.source {
        config.inline_thumbnails = inline;
    }
    let (handle, shell) = open_with(cx, options);
    cx.update(|cx| ramflux_desktop_ui::i18n::select_locale("en", cx));
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the conversation", |cx| {
        with_store(cx, &shell, |s| s.context(&ContextId::new(DM)).is_some())
    });
    show_context(cx, handle, &shell, DM);
    let mut message = text_message("see attached", Vec::new());
    message.media = vec![
        media_ref("o-1", "cat.png", "image/png", 2048, Some(png(64, 64, 100))),
        media_ref("o-2", "notes.pdf", "application/pdf", 10, None),
    ];
    let update = message_update(identity(DM, &[5; 4], "dev-ana", 500, 5), message);
    assert!(sdk_fake::push_generated(&path, update), "bridge running");
    wait(cx, "the message", |cx| with_store(cx, &shell, |s| s.moment(&dm_id(5)).is_some()));
    (fake, path, handle, shell)
}

#[gpui_kit::test]
fn media_rows_render_file_thumbnail_and_revoked_state(cx: &mut gpui_kit::TestAppContext) {
    let _locale = locale_guard();
    let (fake, path, handle, shell) = open_with_media(cx, true);
    let row = "moment-msg:dm-ana:05050505";
    wait(cx, "the media rows", |cx| exists_in(cx, handle, row, "message-media-1"));
    assert!(has_label(cx, handle, "cat.png \u{b7} 2.0 KiB"), "file facts");
    assert!(has_label(cx, handle, "notes.pdf \u{b7} 10 B"));
    assert!(exists_in(cx, handle, row, "message-media-thumbnail-0"), "opt-in on: thumbnail");
    assert!(!exists_in(cx, handle, row, "message-media-thumbnail-1"));

    let revoked = BusError::Daemon {
        retry_after_ms: None,
        code: "Refused".to_owned(),
        message: "access revoked".to_owned(),
        reason_code: Some(REVOKED_REASON.to_owned()),
        details: None,
        cause: None,
    };
    assert!(sdk_fake::push_media_failure(&path, "o-1", revoked));
    wait(cx, "the revoked row", |cx| {
        has_label(cx, handle, "cat.png \u{b7} 2.0 KiB \u{b7} Access revoked")
    });
    assert!(!exists_in(cx, handle, row, "message-media-thumbnail-0"), "no thumbnail kept");
    with_store(cx, &shell, |s| {
        let MomentKind::Message(p) = &s.moment(&dm_id(5)).expect("message").kind else {
            panic!("message")
        };
        assert_eq!(p.media[0].state, MediaState::Revoked);
        assert!(p.media[0].thumbnail.is_none());
    });
    fake.shutdown();
}

#[gpui_kit::test]
fn media_rows_show_files_only_while_thumbnails_are_off(cx: &mut gpui_kit::TestAppContext) {
    let _locale = locale_guard();
    let (fake, _path, handle, _shell) = open_with_media(cx, false);
    let row = "moment-msg:dm-ana:05050505";
    wait(cx, "the media rows", |cx| exists_in(cx, handle, row, "message-media-0"));
    assert!(has_label(cx, handle, "cat.png \u{b7} 2.0 KiB"), "a file row");
    assert!(!exists_in(cx, handle, row, "message-media-thumbnail-0"), "opt-in off: no thumbnail");
    fake.shutdown();
}
