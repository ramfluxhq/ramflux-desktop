// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Rendering follows visibility; logical expiry has its own monotonic alarm.
use super::{ChangeSet, StoreEvent, StoreHandle, TICK};
use gpui_kit::Context;

impl StoreHandle {
    /// Enables time-based rendering and scenario playback. Live completions
    /// and logical deadlines are driven independently of this clock.
    pub fn start_clock(&mut self, cx: &mut Context<Self>) {
        self.clock_enabled = true;
        self.refresh_clocks(cx);
    }

    /// Stops rendering ticks and scenario playback.
    pub fn stop_clock(&mut self) {
        self.clock_enabled = false;
        self.cancel_render_clock();
    }

    /// Whether time-based rendering currently needs a ticker.
    pub fn render_clock_running(&self) -> bool {
        self.clock.is_some()
    }

    /// Follows the platform's presented-frame visibility, including occlusion.
    pub fn set_render_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.render_visible != visible {
            self.render_visible = visible;
            self.refresh_clocks(cx);
            let mut changes = ChangeSet::default();
            changes.mark_clock();
            cx.emit(StoreEvent::Changed(std::rc::Rc::new(changes)));
        }
    }

    fn cancel_render_clock(&mut self) {
        self.clock_generation = self.clock_generation.wrapping_add(1);
        // This may run in the ticker's own callback. It exits on its generation
        // check; detaching lets that callback finish safely.
        if let Some(task) = self.clock.take() {
            task.detach();
        }
        self.last_tick = None;
    }

    pub(super) fn refresh_clocks(&mut self, cx: &mut Context<Self>) {
        // Scenario playback is a logical simulation, not a live render clock.
        let scenario = self.live.is_none() && !self.sessions.is_empty();
        let timed = self
            .active
            .as_ref()
            .and_then(|id| self.sessions.get(id))
            .is_some_and(super::AccountSession::needs_render_time)
            || self.live.as_ref().is_some_and(|l| l.daemon().needs_render_time());
        let needed = self.clock_enabled && self.render_visible && (scenario || timed);
        if !needed && self.clock.is_some() {
            self.cancel_render_clock();
        }
        if needed && self.clock.is_none() {
            self.last_tick = Some(cx.background_executor().now());
            let generation = self.clock_generation;
            let interval = if scenario { TICK } else { std::time::Duration::from_secs(1) };
            self.clock = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(interval).await;
                    let keep = this
                        .update(cx, |handle, cx| {
                            if handle.clock_generation != generation || handle.clock.is_none() {
                                return false;
                            }
                            let now = cx.background_executor().now();
                            let elapsed = handle
                                .last_tick
                                .map_or(interval, |last| now.saturating_duration_since(last));
                            handle.last_tick = Some(now);
                            handle.advance(
                                u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
                                cx,
                            );
                            handle.clock_generation == generation && handle.clock.is_some()
                        })
                        .unwrap_or(false);
                    if !keep {
                        break;
                    }
                }
            }));
        }
        self.arm_logical_deadline(cx);
    }

    fn arm_logical_deadline(&mut self, cx: &mut Context<Self>) {
        let due = self
            .sessions
            .values()
            .chain(self.live.iter().map(|l| l.daemon()))
            .filter(|s| s.is_live())
            .filter_map(|s| s.logical_deadline().map(|due| (due, due.saturating_since(s.now()))))
            .min_by_key(|(due, _)| *due);
        let timestamp = due.map(|(due, _)| due);
        if timestamp == self.logical_due {
            return;
        }
        self.logical_clock = None;
        self.logical_due = timestamp;
        if let Some((due, delay)) = due {
            self.logical_clock = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(std::time::Duration::from_millis(delay)).await;
                let _ = this.update(cx, |handle, cx| {
                    if handle.logical_due != Some(due) {
                        return;
                    }
                    handle.logical_due = None;
                    if let Some(task) = handle.logical_clock.take() {
                        task.detach();
                    }
                    for session in handle.sessions.values_mut() {
                        session.reached_deadline(due);
                    }
                    if let Some(link) = handle.live.as_mut() {
                        link.daemon_mut().reached_deadline(due);
                    }
                    handle.advance(0, cx);
                    let mut changes = ChangeSet::default();
                    changes.mark_clock();
                    cx.emit(StoreEvent::Changed(std::rc::Rc::new(changes)));
                });
            }));
        }
    }
}
