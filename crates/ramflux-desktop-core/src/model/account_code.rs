// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The six-digit account code a person types to unlock an account on this
//! device, and the rule a new code must meet (the daemon's own rule: exactly
//! six ASCII digits, not all equal and not a straight run up or down). The
//! daemon checks it again; this only keeps the desktop from spending a
//! ceremony's single confirmation on a code it would refuse.

/// Number of digits in an account code.
pub const ACCOUNT_CODE_DIGITS: usize = 6;

/// Why a new account code is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountCodeProblem {
    /// Not exactly six ASCII digits.
    NotSixDigits,
    /// All digits equal, or a straight run (`123456`, `654321`).
    Trivial,
}

/// Checks a new account code.
///
/// # Errors
///
/// The [`AccountCodeProblem`] that applies.
pub fn check_account_code(code: &str) -> Result<(), AccountCodeProblem> {
    if code.len() != ACCOUNT_CODE_DIGITS || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(AccountCodeProblem::NotSixDigits);
    }
    let digits = code.as_bytes();
    let steps: Vec<i16> =
        digits.windows(2).map(|pair| i16::from(pair[1]) - i16::from(pair[0])).collect();
    if [0_i16, 1, -1].iter().any(|step| steps.iter().all(|value| value == step)) {
        return Err(AccountCodeProblem::Trivial);
    }
    Ok(())
}
