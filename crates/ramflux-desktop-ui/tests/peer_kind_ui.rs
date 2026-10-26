// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! UI integration tests of verified peer kinds in the production window on
//! the fake daemon (see `sdk_fake`, `fake_profiles`): the kind mark on
//! contacts, in the conversation header and in the members dock, verified
//! by a root-signed profile or only claimed; the warning on a name that
//! could pass for another principal's, with the id beside it, on the
//! contact, in the header and on the author of a message; a verification
//! that went stale, with Verify again; the safe default for a principal
//! nothing was reported about; and an authenticated rename as a line in
//! the conversation.

mod common;
mod sdk_fake;

use common::*;
use gpui_kit::TestAppContext;
use gpui_kit::test::TestWindowExt as _;
use ramflux_desktop_core::{
    ContextId, KindClaim, MomentKind, ParticipantId, PeerKind, SafetyState, SyncMode,
};
use ramflux_desktop_ui::i18n::tr_args;
use ramflux_desktop_ui::identity::kind_text;
use ramflux_desktop_ui::route::Page;
use sdk_fake::wait;

const ACCOUNT: &str = "acct-main";

type Handle = gpui_kit::WindowHandle<gpui_kit::base::Root>;

struct World {
    fake: ramflux_desktop_test::fake_bus::FakeBus,
    state: sdk_fake::Shared,
    handle: Handle,
    shell: gpui_kit::Entity<ramflux_desktop_ui::Shell>,
}

/// The fake daemon with public profiles: Ana verified as a person whose
/// name looks like Cy's, Cy verified as a person whose verification went
/// stale (the fixture's `verification_stale`), Bo an AI participant by its
/// card alone, Zed nothing at all.
fn open_profiles(cx: &mut TestAppContext, name: &str) -> World {
    let (fake, state, path) = sdk_fake::start(name);
    fake.enable_stream();
    {
        let profiles = state.lock().expect("state").profiles.clone();
        profiles.verified("principal-ana", "Ana", "human");
        profiles.warn("principal-ana", false, Some("principal-cy"));
        profiles.verified("principal-cy", "Cy", "human");
        profiles.asserted("principal-bo", Some("Orbit"), Some("headless_ai"));
    }
    let (handle, shell) = open_sized(cx, sdk_fake::options(&path), 1_100.);
    wait(cx, "workspace", |cx| page(cx, &shell) == Page::Workspace);
    wait(cx, "the stream", |cx| {
        with_store(cx, &shell, |s| s.sync_status().is_some_and(|x| x.mode == SyncMode::Stream))
    });
    wait(cx, "the profiles", |cx| {
        with_store(cx, &shell, |s| {
            s.participant(&ParticipantId::new("principal-ana"))
                .is_some_and(|p| p.kind_claim == KindClaim::Verified(PeerKind::Human))
        })
    });
    World { fake, state, handle, shell }
}

fn label_in(
    cx: &mut TestAppContext,
    world: &World,
    scope: &str,
    id: &'static str,
) -> Option<String> {
    let scope = scope.to_owned();
    in_window(cx, world.handle, move |window, _| {
        window.within(scope).try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
}

fn label_of(cx: &mut TestAppContext, world: &World, id: &'static str) -> Option<String> {
    in_window(cx, world.handle, move |window, _| {
        window.try_find(id).and_then(|e| e.label().map(str::to_owned))
    })
}

#[gpui_kit::test]
fn contacts_say_how_far_a_kind_is_proven_and_warn_on_a_confusable_name(cx: &mut TestAppContext) {
    let world = open_profiles(cx, "ui-kind-contacts");
    click(cx, world.handle, "rail-contacts");
    wait(cx, "the contacts", |cx| exists(cx, world.handle, "contacts-principal-ana"));
    // A root-signed profile: the kind, verified; the mark belongs to the
    // kind, not to the name.
    let verified_person = kind_text(KindClaim::Verified(PeerKind::Human)).expect("a kind");
    assert_eq!(
        label_in(cx, &world, "contacts-principal-ana", "contact-kind").as_deref(),
        Some(verified_person.as_ref())
    );
    // A card's claim: the kind, claimed.
    let claimed_ai = kind_text(KindClaim::Asserted(PeerKind::HeadlessAi)).expect("a kind");
    assert_eq!(
        label_in(cx, &world, "contacts-principal-bo", "contact-kind").as_deref(),
        Some(claimed_ai.as_ref())
    );
    // Ana's name looks like Cy's: the warning carries Ana's own id.
    let warning = label_in(cx, &world, "contacts-principal-ana", "contact-name-warning");
    let warning = warning.expect("a name warning");
    assert!(warning.contains("principal-ana"), "{warning}");
    assert!(
        warning.contains(
            tr_args(
                "identity.name.confusable",
                &[("other", &ParticipantId::new("principal-cy").short())]
            )
            .as_ref()
        ),
        "{warning}"
    );
    assert!(label_in(cx, &world, "contacts-principal-cy", "contact-name-warning").is_none());
    // A stale verification: said so, with Verify again.
    assert_eq!(
        label_in(cx, &world, "contacts-principal-cy", "contact-safety").as_deref(),
        None,
        "the safety line is text, not a labeled control"
    );
    let cy = with_store(cx, &world.shell, |s| {
        s.participant(&ParticipantId::new("principal-cy")).cloned()
    });
    let cy = cy.expect("cy");
    assert_eq!(cy.safety, SafetyState::VerificationStale);
    assert!(cy.safety.needs_attention(), "sticky until verified again");
    assert_eq!(
        label_in(cx, &world, "contacts-principal-cy", "verify").as_deref(),
        Some(tr_args("dock.contacts.verify_again_for", &[("name", "Cy")]).as_ref())
    );
    // Nothing reported about Zed: the safe default, no mark, no warning.
    let zed = with_store(cx, &world.shell, |s| {
        s.participant(&ParticipantId::new("principal-zed")).cloned()
    });
    let zed = zed.expect("zed");
    assert_eq!((zed.kind_claim, zed.safety), (KindClaim::None, SafetyState::Unverified));
    assert!(!zed.named());
    assert!(label_in(cx, &world, "contacts-principal-zed", "contact-kind").is_none());
    assert!(label_in(cx, &world, "contacts-principal-zed", "contact-name-warning").is_none());
    world.fake.shutdown();
}

#[gpui_kit::test]
fn the_header_and_the_author_carry_the_name_warning_with_the_id(cx: &mut TestAppContext) {
    let world = open_profiles(cx, "ui-kind-header");
    show_context(cx, world.handle, &world.shell, "dm-ana");
    wait(cx, "the header mark", |cx| exists(cx, world.handle, "header-kind"));
    let verified_person = kind_text(KindClaim::Verified(PeerKind::Human)).expect("a kind");
    assert_eq!(label_of(cx, &world, "header-kind").as_deref(), Some(verified_person.as_ref()));
    let warning = label_of(cx, &world, "header-name-warning").expect("the header warning");
    assert!(warning.contains("principal-ana"), "{warning}");
    // Ana's message rows name her id next to her name.
    wait(cx, "a message from Ana", |cx| exists(cx, world.handle, "author-name-warning"));
    let author = label_of(cx, &world, "author-name-warning").expect("the author warning");
    assert!(author.contains("principal-ana"), "{author}");
    world.fake.shutdown();
}

#[gpui_kit::test]
fn an_authenticated_rename_is_a_line_in_the_conversation(cx: &mut TestAppContext) {
    let world = open_profiles(cx, "ui-kind-rename");
    show_context(cx, world.handle, &world.shell, "dm-ana");
    let event = world.state.lock().expect("state").profiles.rename("principal-ana", "Ana Reyes");
    world.fake.publish(ACCOUNT, "contact.changed", event.expect("a verified profile"));
    let dm = ContextId::new("dm-ana");
    wait(cx, "the name change", |cx| {
        with_store(cx, &world.shell, |s| {
            ramflux_desktop_core::selectors::timeline(s, &dm)
                .iter()
                .any(|m| matches!(&m.kind, MomentKind::NameChange(p) if p.new_name == "Ana Reyes"))
        })
    });
    wait(cx, "the line", |cx| exists(cx, world.handle, "name-change"));
    let line = label_of(cx, &world, "name-change").expect("the line");
    let text = tr_args("moment.name_change.line", &[("old", "Ana"), ("new", "Ana Reyes")]);
    assert!(line.contains(text.as_ref()), "{line}");
    assert!(line.contains("principal-ana"), "the principal id: {line}");
    // The rows are read again: the new name reaches the contact.
    wait(cx, "the new name", |cx| {
        with_store(cx, &world.shell, |s| {
            s.participant(&ParticipantId::new("principal-ana"))
                .is_some_and(|p| p.display == "Ana Reyes")
        })
    });
    let ana = with_store(cx, &world.shell, |s| {
        s.participant(&ParticipantId::new("principal-ana")).cloned()
    });
    assert_eq!(
        ana.map(|p| p.kind_claim),
        Some(KindClaim::Verified(PeerKind::Human)),
        "a rename keeps the claim"
    );
    world.fake.shutdown();
}

#[gpui_kit::test]
fn members_carry_their_kind_mark(cx: &mut TestAppContext) {
    let world = open_profiles(cx, "ui-kind-members");
    show_context(cx, world.handle, &world.shell, "thread-team");
    click(cx, world.handle, "rail-members");
    assert!(exists(cx, world.handle, "dock-content-members"));
    let seen = with_store(cx, &world.shell, |s| {
        let context = s.context(&ContextId::new("thread-team")).cloned();
        let members: Vec<String> = context
            .iter()
            .flat_map(|c| ramflux_desktop_core::selectors::participants(s, &c.id))
            .map(|p| format!("{}:{:?}", p.id, p.kind_claim))
            .collect();
        format!("{context:?} members {members:?}")
    });
    wait(cx, &format!("the members ({seen})"), |cx| {
        exists(cx, world.handle, "member-principal-bo")
    });
    let claimed_ai = kind_text(KindClaim::Asserted(PeerKind::HeadlessAi)).expect("a kind");
    assert_eq!(
        label_in(cx, &world, "member-principal-bo", "member-kind").as_deref(),
        Some(claimed_ai.as_ref())
    );
    assert!(label_in(cx, &world, "member-principal-bo", "member-name-warning").is_none());
    world.fake.shutdown();
}
