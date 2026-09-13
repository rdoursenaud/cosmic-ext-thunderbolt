// SPDX-FileCopyrightText: 2026 Raphaël Doursenaud <raphael@doursenaud.fr>
// SPDX-License-Identifier: GPL-3.0-only

//! UI-specific extensions and helpers for Thunderbolt data types.
//!
//! This module separates presentation logic (icons, labels, translation keys)
//! from the core data model defined in `crate::thunderbolt`.

use crate::fl;
use crate::thunderbolt::{
    AuthFlags, BoltDevice, BoltDeviceType, BoltGeneration, BoltSecurityLevel, BoltStatus,
};

/// Extension trait for mapping core data models to UI resources.
pub trait BoltDeviceUiExt {
    /// Returns the icon name corresponding to the device's current status and type.
    ///
    /// The icon name is compatible with `cosmic::icon::from_name()`.
    fn ui_icon_name(&self) -> &'static str;

    /// Returns the primary text label for the device.
    ///
    /// Prioritizes the user-defined label, falling back to the device name.
    fn ui_display_label(&self) -> &str;

    /// Returns the formatted generation string (e.g., "3", "4", "?").
    fn ui_generation_text(&self) -> String;

    ///// Returns the secondary text line (e.g., status or type) for the device row.
    //fn ui_status_key(&self) -> &'static str;
}

#[allow(clippy::match_same_arms)]
impl BoltDeviceUiExt for BoltDevice {
    fn ui_icon_name(&self) -> &'static str {
        match self.status {
            BoltStatus::Disconnected => "thunderbolt-symbolic",
            BoltStatus::Connecting | BoltStatus::Authorizing => "thunderbolt-symbolic", // TODO: desaturated/pulsating icon
            BoltStatus::AuthError => "dialog-error-symbolic", // TODO: Error (!) overlay over thunderbolt-symbolic
            BoltStatus::Connected | BoltStatus::Authorized => match self.device_type {
                BoltDeviceType::Host | BoltDeviceType::Peripheral => "thunderbolt-symbolic",
                BoltDeviceType::Unknown => "dialog-question-symbolic", // TODO: Warning (?) overlay over thunderbolt-symbolic
            },
            BoltStatus::Unknown => "dialog-question-symbolic", // TODO: Error (?!) overlay over thunderbolt-symbolic
        }
    }

    fn ui_display_label(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.name)
    }

    fn ui_generation_text(&self) -> String {
        format_generation(self.generation)
    }

    // fn ui_status_key(&self) -> &'static str {
    //     match self.status {
    //         BoltStatus::Authorized => "status-authorized",
    //         BoltStatus::Connected => "status-connected",
    //         BoltStatus::Authorizing => "status-authorizing",
    //         BoltStatus::AuthError => "status-auth-error",
    //         BoltStatus::Disconnected => "status-disconnected",
    //         BoltStatus::Connecting => "status-connecting",
    //         BoltStatus::Unknown => "status-unknown",
    //     }
    // }
}

/// Formats the Thunderbolt generation for display.
pub fn format_generation(generation: BoltGeneration) -> String {
    match generation {
        BoltGeneration::Unknown => "?".to_string(),
        BoltGeneration::Gen1 => "1".to_string(),
        BoltGeneration::Gen2 => "2".to_string(),
        BoltGeneration::Gen3 => "3".to_string(),
        BoltGeneration::Gen4 => "4".to_string(),
        BoltGeneration::Gen5 => "5".to_string(),
        BoltGeneration::Future(v) => v.to_string(),
    }
}

/// Determines if a security level requires user attention or warning.
///
/// Returns `true` for levels considered insecure or unknown:
/// - `Unknown`, `None`, `DpOnly`, `UsbOnly`, `NoPcie`
///
/// Returns `false` for secure levels:
/// - `User`, `Secure`
pub fn security_level_is_problematic(level: BoltSecurityLevel) -> bool {
    matches!(
        level,
        BoltSecurityLevel::Unknown
            | BoltSecurityLevel::None
            | BoltSecurityLevel::DpOnly
            | BoltSecurityLevel::UsbOnly
            | BoltSecurityLevel::NoPcie
    )
}

/// Maps security levels to translation keys.
pub fn format_security_level(level: BoltSecurityLevel) -> String {
    match level {
        BoltSecurityLevel::Unknown => fl!("security-level-unknown"),
        BoltSecurityLevel::None => fl!("security-level-none"),
        BoltSecurityLevel::User => fl!("security-level-user"),
        BoltSecurityLevel::Secure => fl!("security-level-secure"),
        BoltSecurityLevel::DpOnly => fl!("security-level-dponly"),
        BoltSecurityLevel::UsbOnly => fl!("security-level-usbonly"),
        BoltSecurityLevel::NoPcie => fl!("security-level-nopcie"),
    }
}

/// Formats auth flags for a tooltip or detailed view.
#[allow(dead_code)]
pub fn format_auth_flags(flags: AuthFlags) -> String {
    let mut parts = Vec::new();

    if flags.is_secure {
        parts.push("auth-flag-secure");
    }
    if flags.is_boot {
        parts.push("auth-flag-boot");
    }
    if flags.no_key {
        parts.push("auth-flag-nokey");
    }
    if flags.no_pcie {
        parts.push("auth-flag-nopcie");
    }

    if parts.is_empty() {
        "auth-flag-none".to_string()
    } else {
        parts.join(", ")
    }
}
