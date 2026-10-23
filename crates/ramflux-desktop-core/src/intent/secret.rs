// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! One-shot secrets handed from the UI to the source.

use std::fmt;

use zeroize::Zeroize;

/// A one-shot secret (passphrase, PIN, activation code) handed from the UI to the source.
///
/// It is deliberately not `Clone`, not `Serialize`, and its `Debug` output is
/// `<redacted>`, so it cannot leak through logs, scenario files, or the event
/// inspector. The source reads it with [`SecretInput::expose`]; the buffer is
/// zeroized when the handle is dropped.
pub struct SecretInput(String);

impl SecretInput {
    /// Wraps a secret.
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    /// Borrows the secret. Do not copy it into longer-lived storage.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether the secret is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Drop for SecretInput {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for SecretInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}
