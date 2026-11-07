// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! S10c c09: every reason the key vault names reaches visible, distinct
//! copy on each of three paths (the account ceremonies, the bus and the AI
//! participant credential export), in both languages. One file, one test:
//! the locale is process-global.

use std::collections::BTreeSet;

use ramflux_desktop_core::{AgentExportRefusal, CeremonyFailure, VaultReason};
use ramflux_desktop_sdk::bridge::ceremony_refusal;
use ramflux_desktop_sdk::bus::{BusError, vault_reason_of};
use ramflux_desktop_sdk::translate::agents::{export_refusal, export_refusal_key};
use ramflux_desktop_ui::i18n::{backend_text, labels, tr};
use ramflux_sdk::SdkError;
use ramflux_sdk::custody::CustodyVaultReason as Product;

const PRODUCT: [Product; 9] = [
    Product::Missing,
    Product::InitPending,
    Product::ImportPending,
    Product::Corrupt,
    Product::Permission,
    Product::Locked,
    Product::Busy,
    Product::PrimaryUnavailable,
    Product::ModeMismatch,
];

/// The error frame the daemon sends for a reason.
fn frame(reason: Product) -> BusError {
    BusError::Daemon {
        retry_after_ms: reason.retry_after_ms(),
        code: reason.wire_code().to_owned(),
        message: "local bus error".to_owned(),
        reason_code: Some(reason.as_str().to_owned()),
        details: None,
        cause: None,
    }
}

fn has_cjk(text: &str) -> bool {
    text.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c))
}

#[test]
fn custody_vault_reasons_map_and_render() {
    for locale in ["en", "zh-CN"] {
        rust_i18n::set_locale(locale);
        let mut ceremony = BTreeSet::new();
        let mut recovery = BTreeSet::new();
        let mut bus = BTreeSet::new();
        let mut export = BTreeSet::new();
        for product in PRODUCT {
            let reason = vault_reason_of(product);
            assert_eq!(reason.code(), product.as_str(), "the reason codes are the daemon's");
            assert_eq!(reason.wire_code(), product.wire_code());
            let mode = reason == VaultReason::ModeMismatch;

            // Custody path: the ceremony failure and its rendered copy.
            let failure = ceremony_refusal(&SdkError::CustodyVault(product));
            let expected =
                if mode { CeremonyFailure::Invalid } else { CeremonyFailure::Vault(reason) };
            assert_eq!(failure, Some(expected), "{reason:?}: custody mapping");
            let shown = labels::ceremony_ended(expected).to_string();
            let recovered = labels::recovery_ended(expected).to_string();
            for (text, key) in [
                (&shown, labels::ceremony_ended_key(expected)),
                (&recovered, labels::recovery_ended_key(expected)),
            ] {
                assert_ne!(text, key, "{reason:?}: {key} is translated");
                assert!(!text.is_empty());
                assert_eq!(has_cjk(text), locale == "zh-CN", "{reason:?} {locale}: {text}");
            }
            if !mode {
                assert_ne!(
                    shown,
                    tr("ceremony.ended.vault").to_string(),
                    "{reason:?}: not generic"
                );
                assert_ne!(
                    recovered,
                    tr("recovery.ended.vault").to_string(),
                    "{reason:?}: not generic"
                );
            }
            ceremony.insert(shown.clone());
            recovery.insert(recovered);
            // The pending reasons name what to finish.
            match reason {
                VaultReason::InitPending => assert!(shown.contains("rf vault init"), "{shown}"),
                VaultReason::ImportPending => {
                    let english = locale == "en";
                    assert!(!english || shown.contains("provisioning import"), "{shown}");
                }
                _ => {}
            }

            // Bus path: the typed refusal, from the daemon's frame and from
            // the product's error, is retried for a busy vault only.
            for error in [frame(product), BusError::from(SdkError::CustodyVault(product))] {
                assert_eq!(error.vault_reason(), Some(reason), "{reason:?}: reason from the frame");
                assert_eq!(
                    error.is_retryable(),
                    reason == VaultReason::Busy,
                    "{reason:?}: only busy retryable"
                );
                let key = error.message();
                let text = backend_text(&key).to_string();
                assert_ne!(text, key, "{reason:?}: {key} is translated");
                assert_eq!(has_cjk(&text), locale == "zh-CN", "{reason:?} {locale}: {text}");
                bus.insert(text);
            }
            assert_eq!(
                frame(product).vault_reason().map(VaultReason::retryable),
                Some(reason == VaultReason::Busy)
            );

            // Agent export path: a mode mismatch is a refused setup, every
            // other reason has its own wording.
            let refusal = export_refusal(&frame(product));
            let expected_refusal = if mode {
                AgentExportRefusal::NotCustodian
            } else {
                AgentExportRefusal::Vault(reason)
            };
            assert_eq!(refusal, expected_refusal, "{reason:?}: export mapping");
            let said = labels::agent_export_refusal(refusal).to_string();
            let backend = backend_text(export_refusal_key(refusal)).to_string();
            assert_ne!(
                said,
                tr("agent.export.refused.credentials_gone").to_string(),
                "{reason:?}: not the generic credentials-gone text"
            );
            assert_eq!(has_cjk(&said), locale == "zh-CN", "{reason:?} {locale}: {said}");
            assert_eq!(has_cjk(&backend), locale == "zh-CN", "{reason:?} {locale}: {backend}");
            export.insert(said);
        }
        // Eight reasons have their own text; the ninth, a mode mismatch, is
        // the refused setup (ceremonies: the invalid copy; export: not the
        // custodian), also distinct from the rest.
        assert_eq!(ceremony.len(), 9, "{locale}: ceremony copy is distinct");
        assert_eq!(recovery.len(), 9, "{locale}: recovery copy is distinct");
        assert_eq!(bus.len(), 9, "{locale}: bus copy is distinct");
        assert_eq!(export.len(), 9, "{locale}: export copy is distinct");
    }

    // Negatives: nothing is invented. An untyped vault refusal keeps its old
    // meaning; an unknown reason, or one on another code, is no vault reason.
    rust_i18n::set_locale("en");
    assert_eq!(
        ceremony_refusal(&SdkError::CustodyVaultUnavailable),
        Some(CeremonyFailure::VaultUnavailable)
    );
    let untyped = BusError::daemon("CustodyVaultUnavailable", "vault");
    assert_eq!(export_refusal(&untyped), AgentExportRefusal::CredentialsGone);
    assert!(untyped.vault_reason().is_none());
    let wrong_code = BusError::Daemon {
        retry_after_ms: None,
        code: "CustodySetupInvalid".to_owned(),
        message: "x".to_owned(),
        reason_code: Some("vault_busy".to_owned()),
        details: None,
        cause: None,
    };
    assert!(wrong_code.vault_reason().is_none(), "a reason belongs to its own code");
    assert!(!wrong_code.is_retryable());
    let unknown = BusError::Daemon {
        retry_after_ms: None,
        code: "CustodyVaultUnavailable".to_owned(),
        message: "x".to_owned(),
        reason_code: Some("vault_something_new".to_owned()),
        details: None,
        cause: None,
    };
    assert!(unknown.vault_reason().is_none());
    assert_eq!(export_refusal(&unknown), AgentExportRefusal::CredentialsGone);
}
