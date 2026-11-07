// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! The nine key-vault refusals as the daemon writes them, with every
//! expectation spelled out as a literal (S10c c09 r2): the wire code and
//! reason, the error object as bytes, the desktop's reason, the retry
//! facts, and the words each path shows in each language. Nothing here is
//! computed with a production helper. The zh-CN texts are written as `\u{..}`
//! escapes because no Rust source may contain CJK text (`locales.rs`).

#![allow(dead_code)]

use ramflux_desktop_core::{AgentExportRefusal, CeremonyFailure, VaultReason};

/// One text in both languages.
pub struct Text {
    pub en: &'static str,
    pub zh: &'static str,
}

impl Text {
    pub fn in_locale(&self, locale: &str) -> &'static str {
        match locale {
            "en" => self.en,
            "zh-CN" => self.zh,
            other => panic!("no literal for {other}"),
        }
    }
}

pub struct Row {
    /// The daemon's `reason_code`.
    pub reason_code: &'static str,
    /// The error `code` that carries it.
    pub wire_code: &'static str,
    /// The `error` object of the frame, as bytes on the wire.
    pub frame: &'static str,
    /// The desktop's reason.
    pub reason: VaultReason,
    /// Whether trying again is worth it.
    pub retryable: bool,
    /// The frame's `retry_after_ms`.
    pub retry_after_ms: Option<u64>,
    /// The account creation that ended.
    pub custody_failure: CeremonyFailure,
    pub ceremony: Text,
    pub recovery: Text,
    /// The status strip's reason of a failed bus request.
    pub bus: Text,
    /// The export refusal in the agents view.
    pub export_refusal: AgentExportRefusal,
    pub export: Text,
    /// The export's reason in the status strip.
    pub export_backend: Text,
}

pub const ROWS: [Row; 9] = [
    Row {
        reason_code: "vault_missing",
        wire_code: "CustodyVaultUnavailable",
        frame: r#"{"details":{"retryable":false},"reason_code":"vault_missing","code":"CustodyVaultUnavailable","message":"vault_missing","retry_after_ms":null}"#,
        reason: VaultReason::Missing,
        retryable: false,
        retry_after_ms: None,
        custody_failure: CeremonyFailure::Vault(VaultReason::Missing),
        ceremony: Text {
            en: r#"This device's key vault is not set up, so no account was created. Set it up, then try again."#,
            zh: "\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5c1a}\u{672a}\u{8bbe}\u{7f6e}\u{ff0c}\u{56e0}\u{6b64}\u{6ca1}\u{6709}\u{521b}\u{5efa}\u{8d26}\u{6237}\u{3002}\u{8bf7}\u{5148}\u{8bbe}\u{7f6e}\u{ff0c}\u{7136}\u{540e}\u{91cd}\u{8bd5}\u{3002}",
        },
        recovery: Text {
            en: r#"This device's key vault is not set up, so the account was not recovered. Set it up, then start over. Nothing was changed."#,
            zh: "\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5c1a}\u{672a}\u{8bbe}\u{7f6e}\u{ff0c}\u{56e0}\u{6b64}\u{672a}\u{6062}\u{590d}\u{8d26}\u{6237}\u{3002}\u{8bf7}\u{5148}\u{8bbe}\u{7f6e}\u{ff0c}\u{7136}\u{540e}\u{91cd}\u{65b0}\u{5f00}\u{59cb}\u{3002}\u{672a}\u{505a}\u{4efb}\u{4f55}\u{66f4}\u{6539}\u{3002}",
        },
        bus: Text {
            en: r#"The key vault of this device is not set up."#,
            zh: "\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5c1a}\u{672a}\u{8bbe}\u{7f6e}\u{3002}",
        },
        export_refusal: AgentExportRefusal::Vault(VaultReason::Missing),
        export: Text {
            en: r#"The credentials were not exported: this device's key vault is not set up. Set it up first. They can be exported once."#,
            zh: "\u{51ed}\u{636e}\u{672a}\u{5bfc}\u{51fa}\u{ff1a}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5c1a}\u{672a}\u{8bbe}\u{7f6e}\u{3002}\u{8bf7}\u{5148}\u{8bbe}\u{7f6e}\u{3002}\u{51ed}\u{636e}\u{53ea}\u{80fd}\u{5bfc}\u{51fa}\u{4e00}\u{6b21}\u{3002}",
        },
        export_backend: Text {
            en: r#"The credentials were not exported: this device's key vault is not set up. Set it up first. They can be exported once."#,
            zh: "\u{51ed}\u{636e}\u{672a}\u{5bfc}\u{51fa}\u{ff1a}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5c1a}\u{672a}\u{8bbe}\u{7f6e}\u{3002}\u{8bf7}\u{5148}\u{8bbe}\u{7f6e}\u{3002}\u{51ed}\u{636e}\u{53ea}\u{80fd}\u{5bfc}\u{51fa}\u{4e00}\u{6b21}\u{3002}",
        },
    },
    Row {
        reason_code: "vault_init_pending",
        wire_code: "CustodyVaultUnavailable",
        frame: r#"{"details":{"remedy":"rf_vault_init","retryable":false},"reason_code":"vault_init_pending","code":"CustodyVaultUnavailable","message":"vault_init_pending","retry_after_ms":null}"#,
        reason: VaultReason::InitPending,
        retryable: false,
        retry_after_ms: None,
        custody_failure: CeremonyFailure::Vault(VaultReason::InitPending),
        ceremony: Text {
            en: r#"Setting up this device's key vault was not finished, so no account was created. Finish it on this device (rf vault init), then try again."#,
            zh: "\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5c1a}\u{672a}\u{8bbe}\u{7f6e}\u{5b8c}\u{6210}\u{ff0c}\u{56e0}\u{6b64}\u{6ca1}\u{6709}\u{521b}\u{5efa}\u{8d26}\u{6237}\u{3002}\u{8bf7}\u{5728}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{4e0a}\u{5b8c}\u{6210}\u{8bbe}\u{7f6e}\u{ff08}rf vault init\u{ff09}\u{ff0c}\u{7136}\u{540e}\u{91cd}\u{8bd5}\u{3002}",
        },
        recovery: Text {
            en: r#"Setting up this device's key vault was not finished, so the account was not recovered. Finish it on this device (rf vault init), then start over. Nothing was changed."#,
            zh: "\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5c1a}\u{672a}\u{8bbe}\u{7f6e}\u{5b8c}\u{6210}\u{ff0c}\u{56e0}\u{6b64}\u{672a}\u{6062}\u{590d}\u{8d26}\u{6237}\u{3002}\u{8bf7}\u{5728}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{4e0a}\u{5b8c}\u{6210}\u{8bbe}\u{7f6e}\u{ff08}rf vault init\u{ff09}\u{ff0c}\u{7136}\u{540e}\u{91cd}\u{65b0}\u{5f00}\u{59cb}\u{3002}\u{672a}\u{505a}\u{4efb}\u{4f55}\u{66f4}\u{6539}\u{3002}",
        },
        bus: Text {
            en: r#"Setting up the key vault of this device was not finished. Finish it on this device (rf vault init)."#,
            zh: "\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5c1a}\u{672a}\u{8bbe}\u{7f6e}\u{5b8c}\u{6210}\u{3002}\u{8bf7}\u{5728}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{4e0a}\u{5b8c}\u{6210}\u{8bbe}\u{7f6e}\u{ff08}rf vault init\u{ff09}\u{3002}",
        },
        export_refusal: AgentExportRefusal::Vault(VaultReason::InitPending),
        export: Text {
            en: r#"The credentials were not exported: setting up this device's key vault was not finished. Finish it on this device (rf vault init), then export again."#,
            zh: "\u{51ed}\u{636e}\u{672a}\u{5bfc}\u{51fa}\u{ff1a}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5c1a}\u{672a}\u{8bbe}\u{7f6e}\u{5b8c}\u{6210}\u{3002}\u{8bf7}\u{5728}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{4e0a}\u{5b8c}\u{6210}\u{8bbe}\u{7f6e}\u{ff08}rf vault init\u{ff09}\u{ff0c}\u{7136}\u{540e}\u{518d}\u{6b21}\u{5bfc}\u{51fa}\u{3002}",
        },
        export_backend: Text {
            en: r#"The credentials were not exported: setting up this device's key vault was not finished. Finish it on this device (rf vault init), then export again."#,
            zh: "\u{51ed}\u{636e}\u{672a}\u{5bfc}\u{51fa}\u{ff1a}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5c1a}\u{672a}\u{8bbe}\u{7f6e}\u{5b8c}\u{6210}\u{3002}\u{8bf7}\u{5728}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{4e0a}\u{5b8c}\u{6210}\u{8bbe}\u{7f6e}\u{ff08}rf vault init\u{ff09}\u{ff0c}\u{7136}\u{540e}\u{518d}\u{6b21}\u{5bfc}\u{51fa}\u{3002}",
        },
    },
    Row {
        reason_code: "vault_import_pending",
        wire_code: "CustodyVaultUnavailable",
        frame: r#"{"details":{"remedy":"provisioning_import_retry","retryable":false},"reason_code":"vault_import_pending","code":"CustodyVaultUnavailable","message":"vault_import_pending","retry_after_ms":null}"#,
        reason: VaultReason::ImportPending,
        retryable: false,
        retry_after_ms: None,
        custody_failure: CeremonyFailure::Vault(VaultReason::ImportPending),
        ceremony: Text {
            en: r#"Importing the provisioned keys into this device's key vault was not finished, so no account was created. Retry the provisioning import, then try again."#,
            zh: "\u{5411}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5bfc}\u{5165}\u{9884}\u{7f6e}\u{5bc6}\u{94a5}\u{5c1a}\u{672a}\u{5b8c}\u{6210}\u{ff0c}\u{56e0}\u{6b64}\u{6ca1}\u{6709}\u{521b}\u{5efa}\u{8d26}\u{6237}\u{3002}\u{8bf7}\u{91cd}\u{8bd5}\u{9884}\u{7f6e}\u{5bfc}\u{5165}\u{ff0c}\u{7136}\u{540e}\u{91cd}\u{8bd5}\u{3002}",
        },
        recovery: Text {
            en: r#"Importing the provisioned keys into this device's key vault was not finished. Retry the provisioning import, then start over. Nothing was changed."#,
            zh: "\u{5411}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5bfc}\u{5165}\u{9884}\u{7f6e}\u{5bc6}\u{94a5}\u{5c1a}\u{672a}\u{5b8c}\u{6210}\u{3002}\u{8bf7}\u{91cd}\u{8bd5}\u{9884}\u{7f6e}\u{5bfc}\u{5165}\u{ff0c}\u{7136}\u{540e}\u{91cd}\u{65b0}\u{5f00}\u{59cb}\u{3002}\u{672a}\u{505a}\u{4efb}\u{4f55}\u{66f4}\u{6539}\u{3002}",
        },
        bus: Text {
            en: r#"Importing the provisioned keys into the key vault was not finished. Retry the provisioning import."#,
            zh: "\u{5411}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5bfc}\u{5165}\u{9884}\u{7f6e}\u{5bc6}\u{94a5}\u{5c1a}\u{672a}\u{5b8c}\u{6210}\u{3002}\u{8bf7}\u{91cd}\u{8bd5}\u{9884}\u{7f6e}\u{5bfc}\u{5165}\u{3002}",
        },
        export_refusal: AgentExportRefusal::Vault(VaultReason::ImportPending),
        export: Text {
            en: r#"The credentials were not exported: importing the provisioned keys into this device's key vault was not finished. Retry the provisioning import, then export again."#,
            zh: "\u{51ed}\u{636e}\u{672a}\u{5bfc}\u{51fa}\u{ff1a}\u{5411}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5bfc}\u{5165}\u{9884}\u{7f6e}\u{5bc6}\u{94a5}\u{5c1a}\u{672a}\u{5b8c}\u{6210}\u{3002}\u{8bf7}\u{91cd}\u{8bd5}\u{9884}\u{7f6e}\u{5bfc}\u{5165}\u{ff0c}\u{7136}\u{540e}\u{518d}\u{6b21}\u{5bfc}\u{51fa}\u{3002}",
        },
        export_backend: Text {
            en: r#"The credentials were not exported: importing the provisioned keys into this device's key vault was not finished. Retry the provisioning import, then export again."#,
            zh: "\u{51ed}\u{636e}\u{672a}\u{5bfc}\u{51fa}\u{ff1a}\u{5411}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5bfc}\u{5165}\u{9884}\u{7f6e}\u{5bc6}\u{94a5}\u{5c1a}\u{672a}\u{5b8c}\u{6210}\u{3002}\u{8bf7}\u{91cd}\u{8bd5}\u{9884}\u{7f6e}\u{5bfc}\u{5165}\u{ff0c}\u{7136}\u{540e}\u{518d}\u{6b21}\u{5bfc}\u{51fa}\u{3002}",
        },
    },
    Row {
        reason_code: "vault_corrupt",
        wire_code: "CustodyVaultUnavailable",
        frame: r#"{"details":{"retryable":false},"reason_code":"vault_corrupt","code":"CustodyVaultUnavailable","message":"vault_corrupt","retry_after_ms":null}"#,
        reason: VaultReason::Corrupt,
        retryable: false,
        retry_after_ms: None,
        custody_failure: CeremonyFailure::Vault(VaultReason::Corrupt),
        ceremony: Text {
            en: r#"This device's key vault is damaged or failed its checks, so no account was created. Nothing was changed."#,
            zh: "\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5df2}\u{635f}\u{574f}\u{6216}\u{672a}\u{901a}\u{8fc7}\u{68c0}\u{67e5}\u{ff0c}\u{56e0}\u{6b64}\u{6ca1}\u{6709}\u{521b}\u{5efa}\u{8d26}\u{6237}\u{3002}\u{672a}\u{505a}\u{4efb}\u{4f55}\u{66f4}\u{6539}\u{3002}",
        },
        recovery: Text {
            en: r#"This device's key vault is damaged or failed its checks, so the account was not recovered. Nothing was changed."#,
            zh: "\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5df2}\u{635f}\u{574f}\u{6216}\u{672a}\u{901a}\u{8fc7}\u{68c0}\u{67e5}\u{ff0c}\u{56e0}\u{6b64}\u{672a}\u{6062}\u{590d}\u{8d26}\u{6237}\u{3002}\u{672a}\u{505a}\u{4efb}\u{4f55}\u{66f4}\u{6539}\u{3002}",
        },
        bus: Text {
            en: r#"The key vault of this device is damaged or failed its checks."#,
            zh: "\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5df2}\u{635f}\u{574f}\u{6216}\u{672a}\u{901a}\u{8fc7}\u{68c0}\u{67e5}\u{3002}",
        },
        export_refusal: AgentExportRefusal::Vault(VaultReason::Corrupt),
        export: Text {
            en: r#"The credentials were not exported: this device's key vault is damaged or failed its checks. Nothing was written."#,
            zh: "\u{51ed}\u{636e}\u{672a}\u{5bfc}\u{51fa}\u{ff1a}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5df2}\u{635f}\u{574f}\u{6216}\u{672a}\u{901a}\u{8fc7}\u{68c0}\u{67e5}\u{3002}\u{6ca1}\u{6709}\u{5199}\u{5165}\u{4efb}\u{4f55}\u{5185}\u{5bb9}\u{3002}",
        },
        export_backend: Text {
            en: r#"The credentials were not exported: this device's key vault is damaged or failed its checks. Nothing was written."#,
            zh: "\u{51ed}\u{636e}\u{672a}\u{5bfc}\u{51fa}\u{ff1a}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5df2}\u{635f}\u{574f}\u{6216}\u{672a}\u{901a}\u{8fc7}\u{68c0}\u{67e5}\u{3002}\u{6ca1}\u{6709}\u{5199}\u{5165}\u{4efb}\u{4f55}\u{5185}\u{5bb9}\u{3002}",
        },
    },
    Row {
        reason_code: "vault_permission",
        wire_code: "CustodyVaultUnavailable",
        frame: r#"{"details":{"retryable":false},"reason_code":"vault_permission","code":"CustodyVaultUnavailable","message":"vault_permission","retry_after_ms":null}"#,
        reason: VaultReason::Permission,
        retryable: false,
        retry_after_ms: None,
        custody_failure: CeremonyFailure::Vault(VaultReason::Permission),
        ceremony: Text {
            en: r#"This device's key vault cannot be used: its files are not safe to use or access was denied. No account was created."#,
            zh: "\u{65e0}\u{6cd5}\u{4f7f}\u{7528}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{ff1a}\u{5176}\u{6587}\u{4ef6}\u{4e0d}\u{5b89}\u{5168}\u{6216}\u{8bbf}\u{95ee}\u{88ab}\u{62d2}\u{7edd}\u{3002}\u{6ca1}\u{6709}\u{521b}\u{5efa}\u{8d26}\u{6237}\u{3002}",
        },
        recovery: Text {
            en: r#"This device's key vault cannot be used: its files are not safe to use or access was denied. Nothing was changed."#,
            zh: "\u{65e0}\u{6cd5}\u{4f7f}\u{7528}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{ff1a}\u{5176}\u{6587}\u{4ef6}\u{4e0d}\u{5b89}\u{5168}\u{6216}\u{8bbf}\u{95ee}\u{88ab}\u{62d2}\u{7edd}\u{3002}\u{672a}\u{505a}\u{4efb}\u{4f55}\u{66f4}\u{6539}\u{3002}",
        },
        bus: Text {
            en: r#"The key vault of this device cannot be used: its files are not safe to use or access was denied."#,
            zh: "\u{65e0}\u{6cd5}\u{4f7f}\u{7528}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{ff1a}\u{5176}\u{6587}\u{4ef6}\u{4e0d}\u{5b89}\u{5168}\u{6216}\u{8bbf}\u{95ee}\u{88ab}\u{62d2}\u{7edd}\u{3002}",
        },
        export_refusal: AgentExportRefusal::Vault(VaultReason::Permission),
        export: Text {
            en: r#"The credentials were not exported: this device's key vault cannot be used because its files are not safe to use or access was denied. Nothing was written."#,
            zh: "\u{51ed}\u{636e}\u{672a}\u{5bfc}\u{51fa}\u{ff1a}\u{65e0}\u{6cd5}\u{4f7f}\u{7528}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{ff0c}\u{5176}\u{6587}\u{4ef6}\u{4e0d}\u{5b89}\u{5168}\u{6216}\u{8bbf}\u{95ee}\u{88ab}\u{62d2}\u{7edd}\u{3002}\u{6ca1}\u{6709}\u{5199}\u{5165}\u{4efb}\u{4f55}\u{5185}\u{5bb9}\u{3002}",
        },
        export_backend: Text {
            en: r#"The credentials were not exported: this device's key vault cannot be used because its files are not safe to use or access was denied. Nothing was written."#,
            zh: "\u{51ed}\u{636e}\u{672a}\u{5bfc}\u{51fa}\u{ff1a}\u{65e0}\u{6cd5}\u{4f7f}\u{7528}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{ff0c}\u{5176}\u{6587}\u{4ef6}\u{4e0d}\u{5b89}\u{5168}\u{6216}\u{8bbf}\u{95ee}\u{88ab}\u{62d2}\u{7edd}\u{3002}\u{6ca1}\u{6709}\u{5199}\u{5165}\u{4efb}\u{4f55}\u{5185}\u{5bb9}\u{3002}",
        },
    },
    Row {
        reason_code: "vault_locked",
        wire_code: "CustodyVaultUnavailable",
        frame: r#"{"details":{"retryable":false},"reason_code":"vault_locked","code":"CustodyVaultUnavailable","message":"vault_locked","retry_after_ms":null}"#,
        reason: VaultReason::Locked,
        retryable: false,
        retry_after_ms: None,
        custody_failure: CeremonyFailure::Vault(VaultReason::Locked),
        ceremony: Text {
            en: r#"This device's key vault is locked, or the unlock was cancelled or took too long. Unlock it and try again. No account was created."#,
            zh: "\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5df2}\u{9501}\u{5b9a}\u{ff0c}\u{6216}\u{89e3}\u{9501}\u{88ab}\u{53d6}\u{6d88}\u{3001}\u{8d85}\u{65f6}\u{3002}\u{8bf7}\u{89e3}\u{9501}\u{540e}\u{91cd}\u{8bd5}\u{3002}\u{6ca1}\u{6709}\u{521b}\u{5efa}\u{8d26}\u{6237}\u{3002}",
        },
        recovery: Text {
            en: r#"This device's key vault is locked, or the unlock was cancelled or took too long. Unlock it and start over. Nothing was changed."#,
            zh: "\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5df2}\u{9501}\u{5b9a}\u{ff0c}\u{6216}\u{89e3}\u{9501}\u{88ab}\u{53d6}\u{6d88}\u{3001}\u{8d85}\u{65f6}\u{3002}\u{8bf7}\u{89e3}\u{9501}\u{540e}\u{91cd}\u{65b0}\u{5f00}\u{59cb}\u{3002}\u{672a}\u{505a}\u{4efb}\u{4f55}\u{66f4}\u{6539}\u{3002}",
        },
        bus: Text {
            en: r#"The key vault of this device is locked, or the unlock was cancelled or took too long."#,
            zh: "\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5df2}\u{9501}\u{5b9a}\u{ff0c}\u{6216}\u{89e3}\u{9501}\u{88ab}\u{53d6}\u{6d88}\u{3001}\u{8d85}\u{65f6}\u{3002}",
        },
        export_refusal: AgentExportRefusal::Vault(VaultReason::Locked),
        export: Text {
            en: r#"The credentials were not exported: this device's key vault is locked, or the unlock was cancelled or took too long. Unlock it and export again."#,
            zh: "\u{51ed}\u{636e}\u{672a}\u{5bfc}\u{51fa}\u{ff1a}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5df2}\u{9501}\u{5b9a}\u{ff0c}\u{6216}\u{89e3}\u{9501}\u{88ab}\u{53d6}\u{6d88}\u{3001}\u{8d85}\u{65f6}\u{3002}\u{8bf7}\u{89e3}\u{9501}\u{540e}\u{518d}\u{6b21}\u{5bfc}\u{51fa}\u{3002}",
        },
        export_backend: Text {
            en: r#"The credentials were not exported: this device's key vault is locked, or the unlock was cancelled or took too long. Unlock it and export again."#,
            zh: "\u{51ed}\u{636e}\u{672a}\u{5bfc}\u{51fa}\u{ff1a}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5df2}\u{9501}\u{5b9a}\u{ff0c}\u{6216}\u{89e3}\u{9501}\u{88ab}\u{53d6}\u{6d88}\u{3001}\u{8d85}\u{65f6}\u{3002}\u{8bf7}\u{89e3}\u{9501}\u{540e}\u{518d}\u{6b21}\u{5bfc}\u{51fa}\u{3002}",
        },
    },
    Row {
        reason_code: "vault_busy",
        wire_code: "CustodyVaultUnavailable",
        frame: r#"{"details":{"retryable":true},"reason_code":"vault_busy","code":"CustodyVaultUnavailable","message":"vault_busy","retry_after_ms":1000}"#,
        reason: VaultReason::Busy,
        retryable: true,
        retry_after_ms: Some(1000),
        custody_failure: CeremonyFailure::Vault(VaultReason::Busy),
        ceremony: Text {
            en: r#"This device's key vault is busy right now. Wait a moment and try again. No account was created."#,
            zh: "\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5f53}\u{524d}\u{6b63}\u{5fd9}\u{3002}\u{8bf7}\u{7a0d}\u{5019}\u{518d}\u{8bd5}\u{3002}\u{6ca1}\u{6709}\u{521b}\u{5efa}\u{8d26}\u{6237}\u{3002}",
        },
        recovery: Text {
            en: r#"This device's key vault is busy right now. Wait a moment and start over. Nothing was changed."#,
            zh: "\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5f53}\u{524d}\u{6b63}\u{5fd9}\u{3002}\u{8bf7}\u{7a0d}\u{5019}\u{540e}\u{91cd}\u{65b0}\u{5f00}\u{59cb}\u{3002}\u{672a}\u{505a}\u{4efb}\u{4f55}\u{66f4}\u{6539}\u{3002}",
        },
        bus: Text {
            en: r#"The key vault of this device is busy right now. Try again in a moment."#,
            zh: "\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5f53}\u{524d}\u{6b63}\u{5fd9}\u{3002}\u{8bf7}\u{7a0d}\u{540e}\u{91cd}\u{8bd5}\u{3002}",
        },
        export_refusal: AgentExportRefusal::Vault(VaultReason::Busy),
        export: Text {
            en: r#"The credentials were not exported: this device's key vault is busy right now. Wait a moment and export again."#,
            zh: "\u{51ed}\u{636e}\u{672a}\u{5bfc}\u{51fa}\u{ff1a}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5f53}\u{524d}\u{6b63}\u{5fd9}\u{3002}\u{8bf7}\u{7a0d}\u{5019}\u{540e}\u{518d}\u{6b21}\u{5bfc}\u{51fa}\u{3002}",
        },
        export_backend: Text {
            en: r#"The credentials were not exported: this device's key vault is busy right now. Wait a moment and export again."#,
            zh: "\u{51ed}\u{636e}\u{672a}\u{5bfc}\u{51fa}\u{ff1a}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{5f53}\u{524d}\u{6b63}\u{5fd9}\u{3002}\u{8bf7}\u{7a0d}\u{5019}\u{540e}\u{518d}\u{6b21}\u{5bfc}\u{51fa}\u{3002}",
        },
    },
    Row {
        reason_code: "vault_primary_unavailable",
        wire_code: "CustodyVaultUnavailable",
        frame: r#"{"details":{"retryable":false},"reason_code":"vault_primary_unavailable","code":"CustodyVaultUnavailable","message":"vault_primary_unavailable","retry_after_ms":null}"#,
        reason: VaultReason::PrimaryUnavailable,
        retryable: false,
        retry_after_ms: None,
        custody_failure: CeremonyFailure::Vault(VaultReason::PrimaryUnavailable),
        ceremony: Text {
            en: r#"This device's key vault holds no primary key for this account, so no account was created."#,
            zh: "\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{4e2d}\u{6ca1}\u{6709}\u{6b64}\u{8d26}\u{6237}\u{7684}\u{4e3b}\u{5bc6}\u{94a5}\u{ff0c}\u{56e0}\u{6b64}\u{6ca1}\u{6709}\u{521b}\u{5efa}\u{8d26}\u{6237}\u{3002}",
        },
        recovery: Text {
            en: r#"This device's key vault holds no primary key for this account, so it was not recovered. Nothing was changed."#,
            zh: "\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{4e2d}\u{6ca1}\u{6709}\u{6b64}\u{8d26}\u{6237}\u{7684}\u{4e3b}\u{5bc6}\u{94a5}\u{ff0c}\u{56e0}\u{6b64}\u{672a}\u{6062}\u{590d}\u{8d26}\u{6237}\u{3002}\u{672a}\u{505a}\u{4efb}\u{4f55}\u{66f4}\u{6539}\u{3002}",
        },
        bus: Text {
            en: r#"The key vault of this device holds no primary key for this account."#,
            zh: "\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{4e2d}\u{6ca1}\u{6709}\u{6b64}\u{8d26}\u{6237}\u{7684}\u{4e3b}\u{5bc6}\u{94a5}\u{3002}",
        },
        export_refusal: AgentExportRefusal::Vault(VaultReason::PrimaryUnavailable),
        export: Text {
            en: r#"The credentials were not exported: this device's key vault holds no primary key for this account."#,
            zh: "\u{51ed}\u{636e}\u{672a}\u{5bfc}\u{51fa}\u{ff1a}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{4e2d}\u{6ca1}\u{6709}\u{6b64}\u{8d26}\u{6237}\u{7684}\u{4e3b}\u{5bc6}\u{94a5}\u{3002}",
        },
        export_backend: Text {
            en: r#"The credentials were not exported: this device's key vault holds no primary key for this account."#,
            zh: "\u{51ed}\u{636e}\u{672a}\u{5bfc}\u{51fa}\u{ff1a}\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{4e2d}\u{6ca1}\u{6709}\u{6b64}\u{8d26}\u{6237}\u{7684}\u{4e3b}\u{5bc6}\u{94a5}\u{3002}",
        },
    },
    Row {
        reason_code: "vault_mode_mismatch",
        wire_code: "CustodySetupInvalid",
        frame: r#"{"details":{"retryable":false},"reason_code":"vault_mode_mismatch","code":"CustodySetupInvalid","message":"vault_mode_mismatch","retry_after_ms":null}"#,
        reason: VaultReason::ModeMismatch,
        retryable: false,
        retry_after_ms: None,
        custody_failure: CeremonyFailure::Invalid,
        ceremony: Text {
            en: r#"The setup could not continue because a step was refused. No account was created."#,
            zh: "\u{8bbe}\u{7f6e}\u{56e0}\u{67d0}\u{4e00}\u{6b65}\u{88ab}\u{62d2}\u{7edd}\u{800c}\u{65e0}\u{6cd5}\u{7ee7}\u{7eed}\u{ff0c}\u{6ca1}\u{6709}\u{521b}\u{5efa}\u{8d26}\u{6237}\u{3002}",
        },
        recovery: Text {
            en: r#"The recovery could not be completed. Check the account passphrase and start over. Nothing was changed."#,
            zh: "\u{65e0}\u{6cd5}\u{5b8c}\u{6210}\u{6062}\u{590d}\u{3002}\u{8bf7}\u{68c0}\u{67e5}\u{8d26}\u{6237}\u{53e3}\u{4ee4}\u{540e}\u{91cd}\u{65b0}\u{5f00}\u{59cb}\u{3002}\u{672a}\u{505a}\u{4efb}\u{4f55}\u{66f4}\u{6539}\u{3002}",
        },
        bus: Text {
            en: r#"This device's key vault does not support what was asked."#,
            zh: "\u{8fd9}\u{53f0}\u{8bbe}\u{5907}\u{7684}\u{5bc6}\u{94a5}\u{4fdd}\u{7ba1}\u{5e93}\u{4e0d}\u{652f}\u{6301}\u{6240}\u{8bf7}\u{6c42}\u{7684}\u{64cd}\u{4f5c}\u{3002}",
        },
        export_refusal: AgentExportRefusal::NotCustodian,
        export: Text {
            en: r#"Only the custodian of this AI participant can export its credentials, and your account is not its custodian."#,
            zh: "\u{53ea}\u{6709}\u{6b64} AI \u{53c2}\u{4e0e}\u{8005}\u{7684}\u{6258}\u{7ba1}\u{4eba}\u{624d}\u{80fd}\u{5bfc}\u{51fa}\u{5b83}\u{7684}\u{51ed}\u{636e}\u{ff0c}\u{800c}\u{4f60}\u{7684}\u{8d26}\u{6237}\u{4e0d}\u{662f}\u{5b83}\u{7684}\u{6258}\u{7ba1}\u{4eba}\u{3002}",
        },
        export_backend: Text {
            en: r#"The credentials were not exported: your account is not the custodian of this AI participant."#,
            zh: "\u{51ed}\u{636e}\u{672a}\u{5bfc}\u{51fa}\u{ff1a}\u{4f60}\u{7684}\u{8d26}\u{6237}\u{4e0d}\u{662f}\u{6b64} AI \u{53c2}\u{4e0e}\u{8005}\u{7684}\u{6258}\u{7ba1}\u{4eba}\u{3002}",
        },
    },
];
