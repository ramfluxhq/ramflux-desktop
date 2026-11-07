// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! S10c c09: the nine refusals of the key vault, as the daemon writes them,
//! are read into the right desktop reason, retry facts and words on each
//! path (the account ceremonies, the bus, the AI participant credential
//! export), in both languages. Every expectation is a literal row in
//! `vault_rows`; the frames are bytes, not product builders. The same rows
//! drive the rendered views in `s10c_vault_live`. One test: the locale is
//! process-global.

mod vault_rows;

use ramflux_desktop_core::{AgentExportRefusal, CeremonyFailure};
use ramflux_desktop_sdk::bridge::ceremony_refusal;
use ramflux_desktop_sdk::bus::BusError;
use ramflux_desktop_sdk::translate::agents::{export_refusal, export_refusal_key};
use ramflux_desktop_ui::i18n::{backend_text, labels};
use ramflux_sdk::SdkError;
use ramflux_sdk::custody::CustodyVaultReason;
use vault_rows::ROWS;

/// The error object of a frame, decoded as the bus client decodes it.
fn decoded(frame: &str) -> BusError {
    let body: ramflux_sdk::LocalBusErrorBody = serde_json::from_str(frame).expect("frame json");
    BusError::from_error_body(Some(body))
}

#[test]
fn custody_vault_reasons_map_and_render() {
    for locale in ["en", "zh-CN"] {
        rust_i18n::set_locale(locale);
        for row in &ROWS {
            let name = format!("{} {locale}", row.reason_code);
            let error = decoded(row.frame);

            // The frame, read by the bus.
            let BusError::Daemon { code, reason_code, retry_after_ms, details, .. } = &error else {
                panic!("{name}: a daemon refusal");
            };
            assert_eq!(code, row.wire_code, "{name}: wire code");
            assert_eq!(reason_code.as_deref(), Some(row.reason_code), "{name}: reason code");
            assert_eq!(*retry_after_ms, row.retry_after_ms, "{name}: retry_after_ms");
            let details = details.as_deref().expect("details");
            assert_eq!(details["retryable"], serde_json::json!(row.retryable), "{name}");
            assert_eq!(error.vault_reason(), Some(row.reason), "{name}: desktop reason");
            assert_eq!(error.is_retryable(), row.retryable, "{name}: retryable");
            assert_eq!(
                backend_text(&error.message()).to_string(),
                row.bus.in_locale(locale),
                "{name}: bus text"
            );

            // The AI participant export.
            let refusal = export_refusal(&error);
            assert_eq!(refusal, row.export_refusal, "{name}: export mapping");
            assert_eq!(
                labels::agent_export_refusal(refusal).to_string(),
                row.export.in_locale(locale),
                "{name}: export text"
            );
            assert_eq!(
                backend_text(export_refusal_key(refusal)).to_string(),
                row.export_backend.in_locale(locale),
                "{name}: export backend text"
            );

            // The ceremonies, from the product's reading of the same wire.
            let product = CustodyVaultReason::from_wire(row.wire_code, Some(row.reason_code))
                .expect("the product reads the frame");
            let failure = ceremony_refusal(&SdkError::CustodyVault(product));
            assert_eq!(failure, Some(row.custody_failure), "{name}: custody mapping");
            assert_eq!(
                labels::ceremony_ended(row.custody_failure).to_string(),
                row.ceremony.in_locale(locale),
                "{name}: ceremony text"
            );
            assert_eq!(
                labels::recovery_ended(row.custody_failure).to_string(),
                row.recovery.in_locale(locale),
                "{name}: recovery text"
            );

            // The product's own error reaches the bus as the same refusal.
            let from_product = BusError::from(SdkError::CustodyVault(product));
            let BusError::Daemon { code, reason_code, retry_after_ms, .. } = &from_product else {
                panic!("{name}: a daemon refusal");
            };
            assert_eq!(code, row.wire_code, "{name}");
            assert_eq!(reason_code.as_deref(), Some(row.reason_code), "{name}");
            assert_eq!(*retry_after_ms, row.retry_after_ms, "{name}");
            assert_eq!(from_product.vault_reason(), Some(row.reason), "{name}");
            assert_eq!(from_product.is_retryable(), row.retryable, "{name}");
        }
    }

    // Negatives: nothing is invented. An untyped vault refusal keeps its old
    // meaning; an unknown reason, or one on another code, is no vault reason.
    rust_i18n::set_locale("en");
    assert_eq!(
        ceremony_refusal(&SdkError::CustodyVaultUnavailable),
        Some(CeremonyFailure::VaultUnavailable)
    );
    let untyped =
        decoded(r#"{"code":"CustodyVaultUnavailable","message":"vault","retry_after_ms":null}"#);
    assert_eq!(export_refusal(&untyped), AgentExportRefusal::CredentialsGone);
    assert!(untyped.vault_reason().is_none());
    let wrong_code = decoded(
        r#"{"code":"CustodySetupInvalid","message":"x","reason_code":"vault_busy","retry_after_ms":1000}"#,
    );
    assert!(wrong_code.vault_reason().is_none(), "a reason belongs to its own code");
    assert!(!wrong_code.is_retryable());
    let unknown = decoded(
        r#"{"code":"CustodyVaultUnavailable","message":"x","reason_code":"vault_something_new","retry_after_ms":null}"#,
    );
    assert!(unknown.vault_reason().is_none());
    assert_eq!(export_refusal(&unknown), AgentExportRefusal::CredentialsGone);
}
