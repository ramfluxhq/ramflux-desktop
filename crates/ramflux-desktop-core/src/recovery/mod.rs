// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The recovery phrase of a born-v3 account, and the words it is made of.
//!
//! The phrase is the one secret of the creation ceremony that travels from
//! the source to the screen. It never enters an event, the store, a
//! scenario or a log:
//!
//! - [`RecoveryPhrase`] is the phrase as the source hands it out
//!   ([`crate::Source::take_recovery_phrase`]). It is neither `Clone` nor
//!   `Serialize`, its `Debug` output is redacted and its buffer is zeroized
//!   when it is dropped.
//! - [`words`] holds the English BIP39 word list and reads a typed word the
//!   same way the backend does (NFKD, ASCII lowercase, one whitespace-free
//!   token, an exact word or a four-letter prefix), so the confirmation
//!   screen can complete and check a word before the one attempt is spent.

mod phrase;
pub mod words;

pub use phrase::{PHRASE_WORDS, PhraseError, RecoveryPhrase};
pub use words::{WordInput, resolve_word, suggestions};
