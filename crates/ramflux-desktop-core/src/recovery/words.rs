// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The English BIP39 word list and the reading of a typed word.
//!
//! The list (`bip39_english.txt`) is the canonical one from the BIP39
//! specification (MIT licensed, 2048 words, sha256
//! `2f5eed53a4727b4bf8880d8f3f199efc90e58503646d9ff8eff3a2ed3b24dbda` over
//! the words, one per line). Its words are unique by their first four
//! letters, which is what makes a four-letter prefix enough.
//!
//! A typed word is read the way the backend reads it: Unicode NFKD, ASCII
//! lowercase, white space split; exactly one token of at most 128 bytes; the
//! token is the word or, when it has exactly four letters, a prefix of it.
//! The screen resolves a word to the full list word before it is sent, so a
//! typo is found here and does not spend the one attempt.

use std::sync::LazyLock;

use unicode_normalization::UnicodeNormalization as _;
use zeroize::Zeroizing;

/// The longest typed word the backend reads.
pub const MAX_INPUT_BYTES: usize = 128;

/// The letters a prefix needs to name a word on its own.
pub const PREFIX_LETTERS: usize = 4;

/// How many candidates [`suggestions`] returns at most.
pub const MAX_SUGGESTIONS: usize = 6;

static WORDS: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| include_str!("bip39_english.txt").lines().collect());

/// The 2048 words in list order (sorted).
pub fn wordlist() -> &'static [&'static str] {
    &WORDS
}

/// Whether `word` is exactly a list word.
pub fn is_word(word: &str) -> bool {
    WORDS.binary_search(&word).is_ok()
}

/// What a typed word came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WordInput {
    /// Nothing (or only white space) was typed.
    Empty,
    /// More than one token, or more than [`MAX_INPUT_BYTES`]: the backend
    /// refuses it.
    NotOneWord,
    /// The token is a list word.
    Word(&'static str),
    /// The token has four letters and starts exactly one list word.
    Prefix(&'static str),
    /// The token starts a list word but does not name one yet.
    Incomplete,
    /// No list word starts with the token (or a four-letter token names none).
    Unknown,
}

impl WordInput {
    /// The full list word this input stands for, when it names one.
    pub fn resolved(&self) -> Option<&'static str> {
        match self {
            Self::Word(word) | Self::Prefix(word) => Some(word),
            _ => None,
        }
    }
}

/// Normalizes one typed word as the backend does: NFKD, ASCII lowercase,
/// surrounding white space dropped. `None` when it is empty, has more than one
/// token or is longer than [`MAX_INPUT_BYTES`].
fn normalize(input: &str) -> Result<Option<Zeroizing<String>>, ()> {
    if input.len() > MAX_INPUT_BYTES {
        return Err(());
    }
    let mut normalized = Zeroizing::new(input.nfkd().collect::<String>());
    normalized.make_ascii_lowercase();
    let mut tokens = normalized.split_whitespace();
    let Some(first) = tokens.next() else { return Ok(None) };
    if tokens.next().is_some() {
        return Err(());
    }
    Ok(Some(Zeroizing::new(first.to_owned())))
}

/// Reads one typed word.
pub fn resolve_word(input: &str) -> WordInput {
    let token = match normalize(input) {
        Ok(Some(token)) => token,
        Ok(None) => return WordInput::Empty,
        Err(()) => return WordInput::NotOneWord,
    };
    if let Ok(index) = WORDS.binary_search(&token.as_str()) {
        return WordInput::Word(WORDS[index]);
    }
    let mut starting = WORDS.iter().filter(|word| word.starts_with(token.as_str()));
    match (starting.next(), starting.next()) {
        (None, _) => WordInput::Unknown,
        (Some(word), None) if token.len() == PREFIX_LETTERS => WordInput::Prefix(word),
        // Four letters name one word; they cannot start several with this
        // list. Fewer, or more than four that are not the word yet, are
        // still being typed.
        (Some(_), _) if token.len() == PREFIX_LETTERS => WordInput::Unknown,
        _ => WordInput::Incomplete,
    }
}

/// Up to [`MAX_SUGGESTIONS`] list words starting with what is typed, once at
/// least two letters are in. Nothing for an input the backend would not read
/// as one word.
pub fn suggestions(input: &str) -> Vec<&'static str> {
    let Ok(Some(token)) = normalize(input) else { return Vec::new() };
    if token.chars().count() < 2 {
        return Vec::new();
    }
    WORDS
        .iter()
        .filter(|word| word.starts_with(token.as_str()))
        .take(MAX_SUGGESTIONS)
        .copied()
        .collect()
}

/// The longest typed phrase read at all (24 words of at most eight letters
/// with generous spacing); longer input is refused unread.
pub const MAX_PHRASE_BYTES: usize = 1_024;

/// What a typed phrase has so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PhraseCheck {
    /// How many tokens are typed.
    pub tokens: usize,
    /// How many of them name a list word (a four-letter prefix does).
    pub resolved: usize,
    /// The one-based position of the first token that names no word and
    /// cannot become one by typing on. A token still being typed (the last
    /// one, with no space after it) is never reported.
    pub first_unknown: Option<usize>,
}

impl PhraseCheck {
    /// Whether the phrase has exactly [`crate::PHRASE_WORDS`] tokens and
    /// every one names a list word.
    pub fn complete(&self) -> bool {
        self.tokens == super::PHRASE_WORDS && self.resolved == self.tokens
    }
}

/// Reads what is typed as a phrase, without keeping any of it.
pub fn check_phrase(typed: &str) -> PhraseCheck {
    let mut check = PhraseCheck::default();
    if typed.len() > MAX_PHRASE_BYTES {
        check.tokens = super::PHRASE_WORDS + 1;
        return check;
    }
    let still_typing = !typed.is_empty() && !typed.ends_with(char::is_whitespace);
    let total = typed.split_whitespace().count();
    for (index, token) in typed.split_whitespace().enumerate() {
        check.tokens += 1;
        match resolve_word(token) {
            WordInput::Word(_) | WordInput::Prefix(_) => check.resolved += 1,
            WordInput::Incomplete if still_typing && index + 1 == total => {}
            WordInput::Unknown if still_typing && index + 1 == total => {}
            _ => {
                check.first_unknown.get_or_insert(index + 1);
            }
        }
    }
    check
}

/// Reads a typed phrase into the canonical form the daemon takes: the
/// list words, in order, joined by single spaces. A four-letter prefix is
/// completed. The result is a zeroizing buffer; nothing else copies it.
///
/// # Errors
///
/// [`super::PhraseError::WordCount`] unless there are exactly 24 tokens,
/// [`super::PhraseError::UnknownWord`] when a token names no list word.
pub fn normalize_phrase(typed: &str) -> Result<Zeroizing<String>, super::PhraseError> {
    if typed.len() > MAX_PHRASE_BYTES || typed.split_whitespace().count() != super::PHRASE_WORDS {
        return Err(super::PhraseError::WordCount);
    }
    let mut phrase = Zeroizing::new(String::with_capacity(super::PHRASE_WORDS * 9));
    for token in typed.split_whitespace() {
        let word = resolve_word(token).resolved().ok_or(super::PhraseError::UnknownWord)?;
        if !phrase.is_empty() {
            phrase.push(' ');
        }
        phrase.push_str(word);
    }
    Ok(phrase)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_the_canonical_one() {
        assert_eq!(wordlist().len(), 2048);
        assert_eq!((wordlist()[0], wordlist()[2047]), ("abandon", "zoo"));
        assert!(wordlist().windows(2).all(|pair| pair[0] < pair[1]), "sorted and unique");
    }

    #[test]
    fn the_first_four_letters_name_a_word_uniquely() {
        let prefixes: std::collections::BTreeSet<&str> =
            wordlist().iter().map(|word| &word[..word.len().min(PREFIX_LETTERS)]).collect();
        assert_eq!(prefixes.len(), 2048);
    }

    #[test]
    fn a_word_a_prefix_and_odd_spellings_resolve_to_the_list_word() {
        assert_eq!(resolve_word("abandon"), WordInput::Word("abandon"));
        assert_eq!(resolve_word("  ABANdon\t"), WordInput::Word("abandon"));
        assert_eq!(resolve_word("aban"), WordInput::Prefix("abandon"));
        assert_eq!(resolve_word("ABAN"), WordInput::Prefix("abandon"));
        // Full-width letters are the same letters after NFKD.
        assert_eq!(resolve_word("\u{ff41}\u{ff42}\u{ff41}\u{ff4e}"), WordInput::Prefix("abandon"));
        // A four-letter list word is itself, not only a prefix.
        assert_eq!(resolve_word("able"), WordInput::Word("able"));
        assert_eq!(resolve_word("add"), WordInput::Word("add"));
    }

    #[test]
    fn what_is_not_yet_a_word_is_told_apart_from_what_never_will_be() {
        assert_eq!(resolve_word(""), WordInput::Empty);
        assert_eq!(resolve_word("   "), WordInput::Empty);
        assert_eq!(resolve_word("ab"), WordInput::Incomplete);
        assert_eq!(resolve_word("aba"), WordInput::Incomplete);
        assert_eq!(resolve_word("abxx"), WordInput::Unknown);
        assert_eq!(resolve_word("abandons"), WordInput::Unknown);
        assert_eq!(resolve_word("abando"), WordInput::Incomplete);
        assert_eq!(resolve_word("zzzz"), WordInput::Unknown);
        assert_eq!(resolve_word("two words"), WordInput::NotOneWord);
        assert_eq!(resolve_word(&"a".repeat(MAX_INPUT_BYTES + 1)), WordInput::NotOneWord);
        // Three letters name a word only when they are the word.
        assert_eq!(resolve_word("ice"), WordInput::Word("ice"));
    }

    #[test]
    fn every_list_word_and_its_four_letter_prefix_resolve_to_it() {
        for word in wordlist() {
            assert_eq!(resolve_word(word), WordInput::Word(word));
            if word.len() > PREFIX_LETTERS {
                assert_eq!(resolve_word(&word[..PREFIX_LETTERS]), WordInput::Prefix(word));
            }
        }
    }

    #[test]
    fn suggestions_start_at_two_letters_and_are_capped() {
        assert!(suggestions("a").is_empty());
        let few = suggestions("ab");
        assert_eq!(few.len(), MAX_SUGGESTIONS);
        assert!(few.iter().all(|word| word.starts_with("ab")));
        assert_eq!(suggestions("zoo"), ["zoo"]);
        assert_eq!(suggestions("zzzz"), Vec::<&str>::new());
        assert!(suggestions("two words").is_empty());
    }

    #[test]
    fn a_typed_phrase_is_checked_while_it_is_typed_and_read_in_canonical_form() {
        let words = wordlist();
        let typed = |n: usize| words.iter().take(n).copied().collect::<Vec<_>>().join(" ");
        let none = check_phrase("");
        assert_eq!((none.tokens, none.resolved, none.first_unknown), (0, 0, None));
        let part = check_phrase(&typed(10));
        assert_eq!((part.tokens, part.resolved), (10, 10));
        assert!(!part.complete());
        assert!(check_phrase(&typed(24)).complete());
        assert!(!check_phrase(&typed(25)).complete(), "one too many");
        // The word still being typed is not an error yet; a finished wrong one is.
        assert_eq!(check_phrase("abandon zz").first_unknown, None);
        assert_eq!(check_phrase("abandon zzzz ").first_unknown, Some(2));
        assert_eq!(check_phrase("abandon notaword extra").first_unknown, Some(2));
        // Prefixes complete, case and spacing do not matter.
        let canonical = normalize_phrase(&typed(24)).expect("phrase");
        let sloppy = typed(24)
            .split(' ')
            .map(|w| format!("  {}", w.chars().take(4).collect::<String>().to_uppercase()))
            .collect::<String>();
        assert_eq!(&*normalize_phrase(&sloppy).expect("prefixes"), &*canonical);
        assert_eq!(normalize_phrase(&typed(23)).err(), Some(super::super::PhraseError::WordCount));
        let wrong = format!("{} xyzzy", typed(23));
        assert_eq!(normalize_phrase(&wrong).err(), Some(super::super::PhraseError::UnknownWord));
        let long = "abandon ".repeat(200);
        assert_eq!(normalize_phrase(&long).err(), Some(super::super::PhraseError::WordCount));
    }
}
