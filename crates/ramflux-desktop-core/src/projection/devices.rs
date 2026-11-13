// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (c) 2026 Span Brain

//! Device security facts kept separate from labels and local-device identity.

use crate::{Device, DeviceId};
use serde::{Deserialize, Serialize};

/// Backend-reported device platform. Missing and future kinds stay unknown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevicePlatform {
    /// Native desktop client.
    Desktop,
    /// Native iOS client.
    Ios,
    /// Native Android client.
    Android,
    /// Linked browser client. Never primary custody.
    Web,
    /// No platform was authenticated or recognized.
    #[default]
    #[serde(other)]
    Unknown,
}

/// Additional source facts for one device, never inferred from its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DeviceFacts {
    /// Device these facts belong to.
    pub device_id: DeviceId,
    /// Explicitly reported platform.
    pub platform: DevicePlatform,
    /// Exact origin reported for a browser device, including its port.
    pub origin: Option<String>,
}

impl DeviceFacts {
    /// Constructs source facts without trusting a device label.
    #[must_use]
    pub fn new(device_id: DeviceId, platform: DevicePlatform, origin: Option<String>) -> Self {
        Self { device_id, platform, origin }
    }
}

/// Security presentation for a device row or pairing review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceTrust<'a> {
    /// No Web claim is made; this does not imply primary custody.
    NativeOrUnknown,
    /// Browser with an exact origin; operator can read its keys and plaintext.
    WebOperator { origin: &'a str },
    /// Browser without origin metadata. Pairing must remain unavailable.
    WebOriginMissing,
}

/// Uses only facts for this exact device. Revoked rows retain their warning.
#[must_use]
pub fn trust<'a>(device: &Device, facts: Option<&'a DeviceFacts>) -> DeviceTrust<'a> {
    match facts.filter(|facts| facts.device_id == device.id) {
        Some(facts) if facts.platform == DevicePlatform::Web => match facts.origin.as_deref() {
            Some(origin) if !origin.trim().is_empty() && !origin.chars().any(char::is_control) => {
                DeviceTrust::WebOperator { origin }
            }
            _ => DeviceTrust::WebOriginMissing,
        },
        _ => DeviceTrust::NativeOrUnknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AccountId;

    #[test]
    fn web_warning_uses_exact_source_origin_and_never_a_device_label() {
        let device = Device {
            id: DeviceId::new("web-1"),
            account_id: AccountId::new("a"),
            label: "Trusted desktop".into(),
            this_device: false,
            revoked: true,
            last_seen: None,
        };
        let facts = DeviceFacts::new(
            device.id.clone(),
            DevicePlatform::Web,
            Some("https://node.example:8443".into()),
        );
        assert_eq!(
            trust(&device, Some(&facts)),
            DeviceTrust::WebOperator { origin: "https://node.example:8443" },
            "revocation and a deceptive label must not erase operator risk"
        );
        assert_eq!(
            trust(&device, None),
            DeviceTrust::NativeOrUnknown,
            "legacy rows must not gain invented platform facts"
        );
        let missing = DeviceFacts::new(device.id.clone(), DevicePlatform::Web, None);
        assert_eq!(
            trust(&device, Some(&missing)),
            DeviceTrust::WebOriginMissing,
            "Web pairing needs an explicit origin"
        );
        let foreign = DeviceFacts::new(
            DeviceId::new("other"),
            DevicePlatform::Web,
            Some("https://other.example".into()),
        );
        assert_eq!(
            trust(&device, Some(&foreign)),
            DeviceTrust::NativeOrUnknown,
            "device facts cannot cross device identities"
        );
    }
}
