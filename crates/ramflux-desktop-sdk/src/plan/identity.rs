// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Fresh identity material for `account.create`.
//!
//! The daemon expects the client to choose the local account, principal,
//! device and delivery-target ids and to supply the root and device seeds
//! (as the reference CLI does). Ids are random; seeds come from the
//! product's crypto crate and travel only as secret fields.

use ramflux_desktop_core::{AccountKind, NodeId, SecretInput};
use serde_json::json;
use zeroize::Zeroizing;

use crate::bus::BusRequest;

/// Builds the `account.create` request, or why it cannot be built.
///
/// # Errors
///
/// Returns a message when the platform cannot supply randomness.
pub fn create_request(
    kind: AccountKind,
    node_id: &NodeId,
    secret: SecretInput,
) -> Result<(String, BusRequest), String> {
    let account = format!("acct-{}", random_hex(8)?);
    let principal = format!("principal-{}", random_hex(16)?);
    let device = format!("device-{}", random_hex(16)?);
    let target = format!("target-{}", random_hex(16)?);
    let root_seed = Zeroizing::new(seed()?);
    let device_seed = Zeroizing::new(seed()?);
    let commitment =
        ramflux_sdk::identity_root_public_key_commitment_for_seed(&principal, *root_seed);
    let client_mode = match kind {
        AccountKind::HumanInteractive => "attended_cli",
        AccountKind::HeadlessAi => "headless_ai",
    };
    let request = BusRequest::new(
        None,
        "account",
        "account.create",
        json!({
            "node_id": node_id,
            "local_account_id": account,
            "principal_id": principal,
            "principal_commitment": commitment,
            "device_id": device,
            "target_delivery_id": target,
            "client_mode": client_mode,
        }),
    )
    .with_secret("account_secret", secret)
    .with_seed("root_seed", root_seed)
    .with_seed("device_seed", device_seed);
    Ok((account, request))
}

fn seed() -> Result<[u8; 32], String> {
    ramflux_crypto::random_32().map_err(|error| format!("no randomness: {error}"))
}

fn random_hex(bytes: usize) -> Result<String, String> {
    let random = Zeroizing::new(seed()?);
    Ok(random.iter().take(bytes.min(32)).map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_requests_carry_fresh_ids_and_keep_seeds_secret() {
        let (account, request) = create_request(
            AccountKind::HumanInteractive,
            &NodeId::new("n-1"),
            SecretInput::new("river stone"),
        )
        .expect("randomness");
        assert!(account.starts_with("acct-") && account.len() == 21);
        assert_eq!(request.body["local_account_id"], account.as_str());
        assert_eq!(request.body["client_mode"], "attended_cli");
        assert!(request.body.get("root_seed").is_none(), "seeds are not in the plain body");
        assert!(!format!("{request:?}").contains("river"));
        let wire = request.wire_body();
        assert_eq!(wire["root_seed"].as_array().map(Vec::len), Some(32));
        assert_eq!(wire["account_secret"], "river stone");
        crate::bus::wipe(wire);
        let (other, _) =
            create_request(AccountKind::HeadlessAi, &NodeId::new("n-1"), SecretInput::new("x"))
                .expect("randomness");
        assert_ne!(account, other);
    }
}
