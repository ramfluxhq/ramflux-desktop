// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Completion publication precedes notification for every bridge channel.
use ramflux_desktop_core::source::SourceWake;
use std::sync::mpsc;

pub(crate) struct CompletionSender<T> {
    sender: mpsc::Sender<T>,
    wake: SourceWake,
}
impl<T> Clone for CompletionSender<T> {
    fn clone(&self) -> Self {
        Self { sender: self.sender.clone(), wake: self.wake.clone() }
    }
}
impl<T> From<mpsc::Sender<T>> for CompletionSender<T> {
    fn from(sender: mpsc::Sender<T>) -> Self {
        Self { sender, wake: SourceWake::default() }
    }
}
impl<T> CompletionSender<T> {
    pub(crate) fn new(sender: mpsc::Sender<T>, wake: SourceWake) -> Self {
        Self { sender, wake }
    }
    pub(crate) fn send(&self, value: T) -> Result<(), mpsc::SendError<T>> {
        self.sender.send(value)?;
        self.wake.notify();
        Ok(())
    }
}
