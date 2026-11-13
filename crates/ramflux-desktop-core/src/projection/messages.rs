// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Mutation capability boundary until the bridge supplies message commands.

/// A command the message menu can present without claiming implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mutation {
    Reply,
    Edit,
    React,
    DeleteLocal,
    DeleteEveryone,
}

/// A typed reason a message command cannot be submitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Unavailable {
    /// The current bridge has no operation for this command.
    #[error("message command is unavailable on this bridge")]
    BridgeContract,
}

impl Mutation {
    /// Checks operation availability. Source display facts do not establish
    /// mutation support; no UI path may accept a draft and discard it.
    ///
    /// # Errors
    /// Returns the missing bridge contract until an authoritative adapter exists.
    pub fn check(self) -> Result<(), Unavailable> {
        Err(Unavailable::BridgeContract)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_display_fields_do_not_enable_unimplemented_mutations() {
        for command in [
            Mutation::Reply,
            Mutation::Edit,
            Mutation::React,
            Mutation::DeleteLocal,
            Mutation::DeleteEveryone,
        ] {
            assert_eq!(
                command.check(),
                Err(Unavailable::BridgeContract),
                "a display fact cannot fabricate a bridge mutation for {command:?}"
            );
        }
    }
}
