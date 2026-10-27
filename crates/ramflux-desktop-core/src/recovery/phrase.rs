// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The phrase held between the source and the screen.

use std::fmt;

use thiserror::Error;
use zeroize::Zeroizing;

use super::words;

/// The number of words of a recovery phrase.
pub const PHRASE_WORDS: usize = 24;

/// Why a phrase was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PhraseError {
    /// The phrase does not have exactly [`PHRASE_WORDS`] words.
    #[error("a recovery phrase has {PHRASE_WORDS} words")]
    WordCount,
    /// A word is not in the English BIP39 list.
    #[error("a recovery phrase word is not in the word list")]
    UnknownWord,
}

/// A recovery phrase on its way to the screen.
///
/// Deliberately not `Clone`, not `Serialize` and without a `Display`; its
/// `Debug` output is `<redacted>` and the buffer is zeroized on drop. Words
/// are borrowed from it ([`RecoveryPhrase::word`]); a caller that copies a
/// word into a `String` owns wiping that copy.
pub struct RecoveryPhrase {
    /// The words joined by single spaces.
    words: Zeroizing<String>,
}

impl RecoveryPhrase {
    /// Reads a phrase of [`PHRASE_WORDS`] list words separated by white space.
    ///
    /// The checksum is the backend's to check: it generated the phrase.
    ///
    /// # Errors
    ///
    /// Returns [`PhraseError`] when the count is not 24 or a word is not in
    /// the English BIP39 list. `words` is zeroized either way (it is a
    /// [`Zeroizing`] buffer owned by this call).
    pub fn parse(words: Zeroizing<String>) -> Result<Self, PhraseError> {
        let mut count = 0;
        for word in words.split_whitespace() {
            if !words::is_word(word) {
                return Err(PhraseError::UnknownWord);
            }
            count += 1;
        }
        if count != PHRASE_WORDS {
            return Err(PhraseError::WordCount);
        }
        Ok(Self { words })
    }

    /// The word at one-based `number`, borrowed from the phrase.
    pub fn word(&self, number: usize) -> Option<&str> {
        number.checked_sub(1).and_then(|index| self.words().nth(index))
    }

    /// The words in order, borrowed from the phrase.
    pub fn words(&self) -> impl Iterator<Item = &str> {
        self.words.split_whitespace()
    }
}

impl fmt::Debug for RecoveryPhrase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn twenty_four(first: &str) -> Zeroizing<String> {
        let mut words = vec![first.to_owned()];
        words.extend(words::wordlist().iter().take(23).map(|w| (*w).to_owned()));
        Zeroizing::new(words.join(" "))
    }

    #[test]
    fn a_phrase_of_list_words_is_read_and_its_words_are_numbered_from_one() {
        let phrase = RecoveryPhrase::parse(twenty_four("zoo")).expect("phrase");
        assert_eq!(phrase.word(1), Some("zoo"));
        assert_eq!(phrase.word(2), Some(words::wordlist()[0]));
        assert_eq!(phrase.word(24), Some(words::wordlist()[22]));
        assert_eq!(phrase.word(0), None);
        assert_eq!(phrase.word(25), None);
        assert_eq!(phrase.words().count(), PHRASE_WORDS);
    }

    #[test]
    fn a_phrase_with_a_wrong_count_or_an_unknown_word_is_refused() {
        let short = Zeroizing::new("abandon ability".to_owned());
        assert_eq!(RecoveryPhrase::parse(short).err(), Some(PhraseError::WordCount));
        let unknown =
            Zeroizing::new(format!("{} xyzzy", twenty_four("zoo").rsplit_once(' ').unwrap().0));
        assert_eq!(RecoveryPhrase::parse(unknown).err(), Some(PhraseError::UnknownWord));
    }

    #[test]
    fn the_debug_output_never_shows_a_word() {
        let phrase = RecoveryPhrase::parse(twenty_four("zoo")).expect("phrase");
        let shown = format!("{phrase:?} {:?}", Some(&phrase));
        assert!(!shown.contains("zoo") && !shown.contains("abandon"), "{shown}");
    }
}
