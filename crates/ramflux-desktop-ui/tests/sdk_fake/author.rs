// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! An AI participant played through the fake daemon: scenario surface
//! events written as the author's protocol events, applied by the daemon
//! and announced on the event stream.

use ramflux_desktop_core::{DomainEvent, SurfaceEvent};
use ramflux_desktop_sdk::surface::author::Author;
use ramflux_desktop_test::fake_bus::FakeBus;
use ramflux_desktop_test::fake_surfaces::surface_ref;

use super::Shared;

/// The group conversation of the fixture, and its bot member.
pub const GROUP: &str = "thread-team";
/// The bot member that plays the AI participant.
pub const BOT: &str = "principal-bo";

/// The bot as an author in the group.
pub struct Bot {
    pub author: Author,
}

impl Bot {
    pub fn new() -> Self {
        Self { author: Author::default() }
    }

    /// Sends `event`; announces it when `publish`. Returns the surface's
    /// reference and whether the daemon accepted it.
    pub fn send(
        &mut self,
        fake: &FakeBus,
        state: &Shared,
        event: &SurfaceEvent,
        publish: bool,
    ) -> (String, bool) {
        let surfaces = state.lock().expect("state").surfaces.clone();
        let accepted = self
            .author
            .send(GROUP, event, |bytes| surfaces.author_event(BOT, bytes).is_ok())
            .expect("authored");
        let reference = surface_ref(GROUP, BOT, event.surface_id().as_str());
        if publish {
            fake.publish("acct-main", "surface.changed", surfaces.meta(&reference));
        }
        (reference, accepted)
    }

    /// Announces a surface's current metadata.
    pub fn announce(fake: &FakeBus, state: &Shared, reference: &str) {
        let meta = state.lock().expect("state").surfaces.meta(reference);
        fake.publish("acct-main", "surface.changed", meta);
    }
}

/// A scenario's surface events, timeline then responses.
pub fn scenario_surfaces(name: &str) -> Vec<SurfaceEvent> {
    let scenario = ramflux_desktop_test::load_fixture(name).expect("fixture");
    scenario
        .timeline
        .iter()
        .chain(scenario.responses.iter().flat_map(|r| &r.emit))
        .filter_map(|(_, e)| match e {
            DomainEvent::Surface(event) => Some(event.clone()),
            _ => None,
        })
        .collect()
}
