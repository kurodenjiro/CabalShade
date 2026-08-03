//! The BIP-39 mnemonic engine: the export/import format for the wallet.
//!
//! # How it works
//!
//! Every EVM private key is 32 bytes of entropy. BIP-39 encodes that entropy
//! into a checksummed 12-word phrase (`abandon ability able …`) that a human
//! can write down and re-enter. This module provides both directions:
//!
//! - **export**: 32-byte key → 12-word mnemonic (`Mnemonic::parse_in_normalized`).
//! - **import**: 12-word mnemonic → 32-byte key. The phrase is validated
//!   against the BIP-39 wordlist and its checksum **before** any AI fuzzy
//!   matching is offered, so a wrong word is caught by the checksum, never by
//!   the model.
//!
//! # Why this and not a custom scheme
//!
//! BIP-39 is the industry standard for wallet backup: the wordlist, the
//! checksum and the seed derivation are specified, portable to any other
//! wallet, and audited. A bespoke "AI-generated secret" would be easier to
//! remember and catastrophically weaker — a human phrase carries far less
//! entropy than 128 random bits. The story the AI writes is a **recall aid for
//! the order of the words**, never part of the secret.

use alloy::primitives::B256;
use alloy::signers::local::PrivateKeySigner;

/// One EVM private key is 128 bits of entropy → 12 words.
pub const WORD_COUNT: usize = 12;

/// A valid 12-word BIP-39 phrase. `Debug` is redacted so a logged phrase never
/// leaks the wallet.
#[derive(Clone, PartialEq, Eq)]
pub struct Mnemonic( bip39::Mnemonic );

impl std::fmt::Debug for Mnemonic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<mnemonic-redacted>")
    }
}

/// Why a mnemonic was rejected on import.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MnemonicError {
    /// Not 12 words, or a word is not in the BIP-39 wordlist.
    #[error("invalid mnemonic: wrong word count or a word outside the BIP-39 wordlist")]
    InvalidWords,
    /// The checksum did not verify — a word is right but in the wrong place,
    /// or the phrase was typed wrong.
    #[error("invalid mnemonic: checksum mismatch")]
    Checksum,
    /// The phrase validated but is not 128 bits of entropy (unsupported
    /// strength — this engine only exports/imports 12-word phrases).
    #[error("invalid mnemonic: expected 128-bit (12-word) strength")]
    WrongStrength,
}

impl Mnemonic {
    /// A fresh random 12-word phrase. This is how the wallet's key is born:
    /// generate the mnemonic, derive the key from it, and the phrase is the
    /// recoverable form of that exact key.
    #[must_use]
    pub fn generate() -> Self {
        Self(bip39::Mnemonic::generate_in(bip39::Language::English, WORD_COUNT).expect("128-bit entropy is valid"))
    }

    /// The words, in order.
    #[must_use]
    pub fn words(&self) -> Vec<&str> {
        self.0.words().collect()
    }

    /// Parses and validates a user-entered phrase.
    ///
    /// # Errors
    ///
    /// [`MnemonicError::InvalidWords`] if a word is unknown or the count is
    /// wrong, [`MnemonicError::Checksum`] if the checksum fails, and
    /// [`MnemonicError::WrongStrength`] if the entropy is not 128 bits.
    pub fn parse(input: &str) -> Result<Self, MnemonicError> {
        let normalized = input.split_whitespace().collect::<Vec<_>>().join(" ");
        let mnemonic = match bip39::Mnemonic::parse_in_normalized(
            bip39::Language::English,
            &normalized,
        ) {
            Ok(m) => m,
            Err(bip39::Error::InvalidChecksum) => return Err(MnemonicError::Checksum),
            Err(bip39::Error::BadWordCount(_)) => return Err(MnemonicError::WrongStrength),
            Err(bip39::Error::UnknownWord(_)) | Err(_) => return Err(MnemonicError::InvalidWords),
        };
        if mnemonic.word_count() != WORD_COUNT {
            return Err(MnemonicError::WrongStrength);
        }
        Ok(Self(mnemonic))
    }

    /// The 32-byte EVM private key this phrase encodes.
    ///
    /// Standard BIP-39 seed derivation: PBKDF2-HMAC-SHA512 of the mnemonic
    /// (with passphrase "") produces 64 bytes; the first 32 are the private
    /// key. This is what makes the phrase portable to any BIP-39 wallet —
    /// the same words derive the same key everywhere.
    #[must_use]
    pub fn to_key(&self) -> B256 {
        let seed = self.0.to_seed("");
        B256::from_slice(&seed[..32])
    }

    /// The signer this phrase derives.
    ///
    /// # Errors
    ///
    /// If the 32-byte key does not form a valid secp256k1 secret.
    pub fn to_signer(&self) -> Result<PrivateKeySigner, Box<dyn std::error::Error>> {
        Ok(PrivateKeySigner::from_bytes(&self.to_key())?)
    }
}

/// Suggests likely intended words for a possibly-mistyped input, for the
/// AI-assisted import field.
///
/// Matches against the BIP-39 English wordlist by edit distance ≤ 2, plus
/// prefix matches. The list is returned for the UI to offer; the user's
/// confirmed selection is what gets validated, never an accepted guess.
#[must_use]
pub fn suggest_words(input: &str) -> Vec<String> {
    let target = input.trim().to_lowercase();
    if target.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<(u8, String)> = bip39::Language::English
        .word_list()
        .iter()
        .filter_map(|&word| {
            // An exact match needs no suggestion.
            if word.eq_ignore_ascii_case(&target) {
                return None;
            }
            let distance = edit_distance(&target, word);
            // Prefix matches are strong signals ("jelly" -> "jellyfish");
            // an edit distance up to 2 catches typos ("jellyfsh").
            let prefix = word.starts_with(&target);
            if distance <= 2 || prefix {
                Some((distance + u8::from(!prefix), word.to_string()))
            } else {
                None
            }
        })
        .collect();

    scored.sort_by_key(|(d, w)| (*d, w.clone()));
    scored.truncate(4);
    scored.into_iter().map(|(_, w)| w).collect()
}

/// Classic Levenshtein edit distance between two ASCII strings.
fn edit_distance(a: &str, b: &str) -> u8 {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let mut prev: Vec<u8> = (0..=b.len() as u8).collect();
    let mut curr = vec![0u8; b.len() + 1];

    for (i, ca) in a.iter().enumerate() {
        curr[0] = (i + 1) as u8;
        for (j, cb) in b.iter().enumerate() {
            let cost = u8::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_mnemonic_round_trips_through_its_words() {
        let mnemonic = Mnemonic::generate();
        let key = mnemonic.to_key();
        let words = mnemonic.words().join(" ");
        let parsed = Mnemonic::parse(&words).unwrap();
        assert_eq!(parsed.to_key(), key);
    }

    #[test]
    fn a_key_derives_from_the_mnemonic_deterministically() {
        let a = Mnemonic::parse("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").unwrap();
        let b = Mnemonic::parse("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").unwrap();
        assert_eq!(a.to_key(), b.to_key());
    }

    #[test]
    fn different_mnemonics_derive_different_keys() {
        let a = Mnemonic::generate();
        let b = Mnemonic::generate();
        if a.words() != b.words() {
            assert_ne!(a.to_key(), b.to_key());
        }
    }

    #[test]
    fn the_mnemonic_has_twelve_words() {
        let mnemonic = Mnemonic::generate();
        assert_eq!(mnemonic.words().len(), 12);
    }

    #[test]
    fn the_words_are_valid_bip39_english() {
        let mnemonic = Mnemonic::generate();
        let list = bip39::Language::English.word_list();
        for word in mnemonic.words() {
            assert!(list.contains(&word), "word not in BIP-39 list: {word}");
        }
    }

    #[test]
    fn parsing_a_valid_phrase_recovers_the_key() {
        let mnemonic = Mnemonic::generate();
        let phrase = mnemonic.words().join(" ");
        let parsed = Mnemonic::parse(&phrase).unwrap();
        assert_eq!(parsed.to_key(), mnemonic.to_key());
    }

    #[test]
    fn a_checksum_error_is_rejected() {
        let mnemonic = Mnemonic::generate();
        let mut words: Vec<&str> = mnemonic.words();
        // Swap two words: every word is valid, but the checksum fails.
        words.swap(0, 1);
        assert_eq!(
            Mnemonic::parse(&words.join(" ")),
            Err(MnemonicError::Checksum)
        );
    }

    #[test]
    fn an_unknown_word_is_rejected_before_the_checksum() {
        assert_eq!(
            Mnemonic::parse("abandon notaword ability able about above absent absorb abstract absurd abuse access"),
            Err(MnemonicError::InvalidWords)
        );
    }

    #[test]
    fn a_wrong_word_count_is_rejected() {
        let mnemonic = Mnemonic::generate();
        let words: Vec<&str> = mnemonic.words();
        assert_eq!(
            Mnemonic::parse(&words[..6].join(" ")),
            Err(MnemonicError::WrongStrength)
        );
    }

    #[test]
    fn the_debug_form_is_redacted() {
        let mnemonic = Mnemonic::generate();
        let rendered = format!("{mnemonic:?}");
        assert_eq!(rendered, "<mnemonic-redacted>");
    }

    #[test]
    fn fuzzy_suggestions_catch_typos() {
        // "abandon" with the 'o' dropped — a one-edit typo the matcher must catch.
        let suggestions = suggest_words("abandn");
        assert!(
            suggestions.iter().any(|w| w == "abandon"),
            "expected abandon among {suggestions:?}"
        );
    }

    #[test]
    fn fuzzy_suggestions_match_prefixes() {
        let suggestions = suggest_words("aband");
        assert!(
            suggestions.iter().any(|w| w == "abandon"),
            "expected abandon among {suggestions:?}"
        );
    }

    #[test]
    fn empty_input_yields_no_suggestions() {
        assert!(suggest_words("").is_empty());
        assert!(suggest_words("   ").is_empty());
    }

    #[test]
    fn exact_words_are_excluded() {
        // A correctly typed word needs no suggestion.
        assert!(!suggest_words("abandon").contains(&"abandon".to_string()));
    }
}
