// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The typed renderer of message schema v1 in the production window, fed
//! generated updates through the projection test seam. The umbrella test
//! grows leg by leg: Unicode (S10c c03), then delete, unknown and transient.

mod common;
mod sdk_fake;

use common::*;
use ramflux_desktop_core::{ContextId, MomentId, MomentKind, SpanKind};
use ramflux_desktop_test::v1_fixtures::{
    delete_update, entity, forward_of, identity, message_ref, message_update, service_update,
    text_message, unsupported_update, with_journal,
};
use ramflux_desktop_ui::route::Page;
use ramflux_protocol::message::{
    Content, DeleteScope, EntityKind, ServiceAction, content::Value, message_update::State,
};

const TEXT: &str = "A\u{1F600}\u{e9}";

fn moment_id(n: u8) -> MomentId {
    MomentId::new(format!("msg:dm-ana:{}", format!("{n:02x}").repeat(4)))
}

#[gpui_kit::test]
fn typed_renderer_unknown_unicode_delete_and_transient(cx: &mut gpui_kit::TestAppContext) {
    let _locale = locale_guard();
    let (fake, _state, path) = sdk_fake::start("s10c-typed-renderer");
    let (handle, shell) = open_with(cx, sdk_fake::options(&path));
    cx.update(|cx| ramflux_desktop_ui::i18n::select_locale("en", cx));
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the conversation", |cx| {
        with_store(cx, &shell, |s| s.context(&ContextId::new("dm-ana")).is_some())
    });
    show_context(cx, handle, &shell, "dm-ana");

    // Unicode leg: a bold span over scalar offset 1 covers the emoji only,
    // beside a reply and a forward.
    let earlier = message_update(
        identity("dm-ana", &[1; 4], "dev-ana", 100, 1),
        text_message("earlier words", Vec::new()),
    );
    let mut styled = text_message(TEXT, vec![entity(1, 1, EntityKind::EntityBold)]);
    styled.reply_to = Some(message_ref("dm-ana", &[1; 4]));
    styled.forward = Some(forward_of(message_ref("c-else", &[7; 4]), "Kai"));
    let styled = message_update(identity("dm-ana", &[2; 4], "dev-ana", 200, 2), styled);
    assert!(sdk_fake::push_generated(&path, earlier), "bridge running");
    assert!(sdk_fake::push_generated(&path, styled));
    wait(cx, "both messages", |cx| with_store(cx, &shell, |s| s.moment(&moment_id(2)).is_some()));

    // Data layer: the stored span and body.
    with_store(cx, &shell, |s| {
        let MomentKind::Message(p) = &s.moment(&moment_id(2)).expect("styled").kind else {
            panic!("message")
        };
        assert_eq!(p.spans.len(), 1);
        assert_eq!(p.spans[0].kind, SpanKind::Bold);
        assert_eq!((p.spans[0].offset, p.spans[0].length), (1, 1));
        assert_eq!(s.bodies().get(&moment_id(2)).map(|t| t.to_string()), Some(TEXT.to_owned()));
    });
    // Rendered: the bold span covers \u{1F600} only, the reply quotes the
    // earlier message, the forward names its source.
    wait(cx, "the rendered row", |cx| {
        exists_in(cx, handle, "moment-msg:dm-ana:02020202", "message-reply")
    });
    assert!(
        has_label(cx, handle, "bold: \u{1F600}"),
        "A\u{1F600}\u{e9} bold span covers \u{1F600} only"
    );
    assert!(!has_label(cx, handle, "bold: A\u{1F600}\u{e9}"));
    assert!(has_label(cx, handle, "Reply to: earlier words"));
    assert!(has_label(cx, handle, "Forwarded from Kai"));

    // A pin is a visible Service row, in the same stream.
    let mut pin = service_update(
        identity("dm-ana", &[3; 4], "dev-ana", 300, 3),
        ServiceAction::ServicePinChanged,
        &[],
        None,
    );
    if let Some(State::Content(Content { value: Some(Value::Service(service)) })) = &mut pin.state {
        service.target = Some(message_ref("dm-ana", &[2; 4]));
    }
    assert!(sdk_fake::push_generated(&path, pin));
    wait(cx, "the pin row", |cx| with_store(cx, &shell, |s| s.moment(&moment_id(3)).is_some()));
    wait(cx, "the pin row shown", |cx| has_label(cx, handle, "dev-ana changed the pinned message"));

    // Delete leg: a delete for everyone leaves a tombstone, no text anywhere
    // in the store and nothing of the old text on screen.
    let delete = delete_update(
        identity("dm-ana", &[4; 4], "dev-ana", 400, 4),
        vec![message_ref("dm-ana", &[2; 4])],
        DeleteScope::Everyone,
    );
    assert!(sdk_fake::push_generated(&path, delete));
    wait(cx, "the tombstone", |cx| {
        with_store(
            cx,
            &shell,
            |s| matches!(&s.moment(&moment_id(2)).map(|m| &m.kind), Some(MomentKind::Message(p)) if p.tombstone),
        )
    });
    with_store(cx, &shell, |s| {
        let MomentKind::Message(p) = &s.moment(&moment_id(2)).expect("deleted").kind else {
            panic!("message")
        };
        assert!(p.text.is_empty() && p.spans.is_empty() && p.reply_to.is_none());
        assert_eq!(p.delete_scope, Some(ramflux_desktop_core::DeleteScope::Everyone));
        assert_eq!(
            s.bodies().get(&moment_id(2)).map(|t| t.to_string()).unwrap_or_default(),
            "",
            "no body text left in the store"
        );
    });
    wait(cx, "the tombstone shown", |cx| has_label(cx, handle, "Message deleted"));
    assert!(!has_label(cx, handle, "bold: \u{1F600}"), "no span of the old text on screen");
    assert!(!has_label(cx, handle, "Forwarded from Kai"), "no forward header of the old message");

    // Unknown leg: a message of a newer type is a placeholder with no text;
    // a supported reparse replaces it exactly once.
    let newer = unsupported_update(identity("dm-ana", &[6; 4], "dev-ana", 600, 6), 6, 1, 99, 412);
    assert!(sdk_fake::push_generated(&path, newer));
    wait(cx, "the placeholder", |cx| {
        with_store(cx, &shell, |s| {
            matches!(&s.moment(&moment_id(6)).map(|m| &m.kind), Some(MomentKind::Fallback(_)))
        })
    });
    wait(cx, "the placeholder shown", |cx| {
        has_label(cx, handle, "This message is of a newer type this app version cannot show yet.")
    });
    let reparsed = with_journal(
        message_update(
            identity("dm-ana", &[6; 4], "dev-ana", 600, 6),
            text_message("now readable", Vec::new()),
        ),
        7,
    );
    assert!(sdk_fake::push_generated(&path, reparsed.clone()));
    wait(cx, "the reparse", |cx| {
        with_store(cx, &shell, |s| s.bodies().get(&moment_id(6)).is_some())
    });
    assert!(sdk_fake::push_generated(&path, reparsed));
    advance(cx, 200);
    with_store(cx, &shell, |s| {
        assert!(matches!(&s.moment(&moment_id(6)).map(|m| &m.kind), Some(MomentKind::Message(_))));
        assert_eq!(
            s.bodies().get(&moment_id(6)).map(|t| t.to_string()),
            Some("now readable".to_owned())
        );
    });
    wait(cx, "the placeholder gone", |cx| {
        !has_label(cx, handle, "This message is of a newer type this app version cannot show yet.")
    });
    fake.shutdown();
}
