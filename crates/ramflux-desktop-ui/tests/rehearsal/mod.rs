// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The demo rehearsal harness: two production window roots, each on its own
//! daemon, walked through the tier ② demo with their own controls.
//!
//! - `config`: node URL, expected node key, run id, screenshot folder, limits.
//! - `host`: the GPUI context (offscreen with a renderer, or the test context).
//! - `desk`: one desktop window, driven by clicks, typing and holds.
//! - `daemon`: the daemon behind a desktop and the reference CLI beside it.
//! - `steps`: the timed step table with prerequisites.
//! - `flow`: the seven steps (and the live-only steps, see `flow::run_live`).
//! - `tap`: a frame log between a desktop and its daemon (live run).
//!
//! Used by the live run (`demo_rehearsal`, against a real node) and by the
//! fake-daemon tests (`demo_rehearsal_fake`).

#![allow(dead_code)]

pub mod config;
pub mod daemon;
pub mod desk;
pub mod flow;
pub mod host;
pub mod steps;
pub mod tap;
