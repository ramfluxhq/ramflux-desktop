// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Mutation capability boundary: which message commands the bridge carries.
//!
//! Reply, edit and delete for everyone go to the daemon's conversation
//! mutations (`conversation.reply`, `conversation.edit` with the revision it
//! expects, `conversation.delete` scope `everyone`). Reactions and a delete
//! on this device only have no control yet.

/// A command the message menu can present.
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
    /// Checks operation availability. No UI path may accept a draft for a
    /// command the bridge cannot carry and discard it.
    ///
    /// # Errors
    /// Returns the missing bridge contract for reactions and local deletes.
    pub fn check(self) -> Result<(), Unavailable> {
        match self {
            Self::Reply | Self::Edit | Self::DeleteEveryone => Ok(()),
            Self::React | Self::DeleteLocal => Err(Unavailable::BridgeContract),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_carried_mutations_are_available() {
        for command in [Mutation::Reply, Mutation::Edit, Mutation::DeleteEveryone] {
            assert_eq!(command.check(), Ok(()), "{command:?}");
        }
        for command in [Mutation::React, Mutation::DeleteLocal] {
            assert_eq!(command.check(), Err(Unavailable::BridgeContract), "{command:?}");
        }
    }
}
