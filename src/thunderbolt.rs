// SPDX-FileCopyrightText: 2026 Raphaël Doursenaud <raphael@doursenaud.fr>
// SPDX-License-Identifier: GPL-3.0-only

//! Thunderbolt™ D-Bus interface definitions and state management.
//!
//! This module defines the complete data model for Thunderbolt devices.
//! Some fields and methods are not yet used in the UI but are reserved for
//! future features (topology view, detailed device info, advanced commands).
#![allow(dead_code)]

use crate::bolt_dbus;
use cosmic::iced::Subscription;
use std::fmt;
use tokio::sync::mpsc;
use zvariant::{DeserializeDict, OwnedObjectPath, SerializeDict, Type};

// D-Bus Service and Path constants
pub const DBUS_SERVICE: &str = "org.freedesktop.bolt";
pub const DBUS_ROOT_PATH: &str = "/org/freedesktop/bolt";
pub const DBUS_DEVICES_PATH: &str = const_format::concatcp!(DBUS_ROOT_PATH, "/", "devices");
pub const DBUS_DOMAINS_PATH: &str = const_format::concatcp!(DBUS_ROOT_PATH, "/", "domains");

/// Represents specific error conditions when interacting with the Thunderbolt daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoltError {
    DbusConnectionFailed,
    ServiceNotFound,
    ProxyCreationFailed,
    PropertyReadFailed,
    DaemonTerminated,
    GenericError { detail: String },
}

/// A normalized Thunderbolt device UID.
///
/// Thunderbolt UIDs can be represented in two formats:
/// - D-Bus format: Uses underscores (e.g., `20219200_0230_3c00_ffff_ffffffffffff`)
/// - Standard format: Uses hyphens (e.g., `20219200-0230-3c00-ffff-ffffffffffff`)
///
/// This struct normalizes all inputs to the standard hyphenated format for internal storage
/// and provides methods to convert back to D-Bus format when constructing paths.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BoltUid(String);

impl BoltUid {
    /// Creates a new `BoltUid` from a string.
    ///
    /// Accepts both hyphenated and underscored formats.
    /// Returns `None` if the format is invalid (mixed separators or non-hex characters).
    pub fn new(s: impl Into<String>) -> Option<Self> {
        let input = s.into();
        if input.is_empty() {
            return None;
        }

        let has_dash = input.contains('-');
        let has_underscore = input.contains('_');

        // Normalize: Convert underscores to hyphens.
        // Reject mixed separators to avoid ambiguity, unless it's a raw hex string.
        let normalized = if has_underscore && !has_dash {
            input.replace('_', "-")
        } else if has_dash {
            input
        } else {
            // If no separators, check if it's a raw 32-char hex string.
            // We accept it as-is, assuming it's a valid UUID without formatting.
            if input.len() == 32 && input.chars().all(|c| c.is_ascii_hexdigit()) {
                input
            } else {
                return None;
            }
        };

        // Final validation: must only contain hex digits and hyphens.
        if !normalized
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == '-')
        {
            return None;
        }

        Some(Self(normalized))
    }

    /// Returns the UID in standard format (with hyphens).
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the UID in D-Bus format (with underscores).
    ///
    /// Use this method when constructing D-Bus object paths or calling methods.
    pub fn as_dbus_path_segment(&self) -> String {
        self.0.replace('-', "_")
    }

    /// Constructs the full D-Bus object path for this device.
    pub fn to_device_path(&self) -> String {
        format!("{}/{}", DBUS_DEVICES_PATH, self.as_dbus_path_segment())
    }

    /// Constructs the full D-Bus object path for this domain.
    pub fn to_domain_path(&self) -> String {
        format!("{}/{}", DBUS_DOMAINS_PATH, self.as_dbus_path_segment())
    }
}

/// Legacy conversion implementations.
///
/// NOTE: These bypass validation logic present in `new()`.
/// They assume the input is already well-formed. Prefer `BoltUid::new()` for untrusted input.
impl From<String> for BoltUid {
    fn from(s: String) -> Self {
        Self(s.replace('_', "-"))
    }
}

impl From<&str> for BoltUid {
    fn from(s: &str) -> Self {
        Self(s.replace('_', "-"))
    }
}

impl fmt::Display for BoltUid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Represents a Thunderbolt domain (typically a host controller).
#[derive(Debug, Clone)]
pub struct BoltDomain {
    pub uid: BoltUid,
    pub id: String,
    pub security_level: BoltSecurityLevel,
    pub iommu_active: bool,
    pub boot_acl: Vec<String>,
    pub sysfs_path: Option<String>,
}

/// The type of the Thunderbolt device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoltDeviceType {
    Unknown,
    Host,
    Peripheral,
}

/// The security level of the Thunderbolt domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoltSecurityLevel {
    #[default]
    Unknown,
    None,
    DpOnly,
    User,
    Secure,
    UsbOnly,
    NoPcie,
}

impl From<&str> for BoltSecurityLevel {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "none" => BoltSecurityLevel::None,
            "dponly" => BoltSecurityLevel::DpOnly,
            "user" => BoltSecurityLevel::User,
            "secure" => BoltSecurityLevel::Secure,
            "usbonly" => BoltSecurityLevel::UsbOnly,
            "nopcie" => BoltSecurityLevel::NoPcie,
            _ => BoltSecurityLevel::Unknown,
        }
    }
}

impl std::fmt::Display for BoltSecurityLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            BoltSecurityLevel::Unknown => "unknown",
            BoltSecurityLevel::None => "none",
            BoltSecurityLevel::DpOnly => "dponly",
            BoltSecurityLevel::User => "user",
            BoltSecurityLevel::Secure => "secure",
            BoltSecurityLevel::UsbOnly => "usbonly",
            BoltSecurityLevel::NoPcie => "nopcie",
        };
        write!(f, "{}", s)
    }
}

impl BoltSecurityLevel {
    /// Returns true if PCIe tunneling is allowed at this security level.
    pub fn allows_pcie(&self) -> bool {
        matches!(
            self,
            BoltSecurityLevel::None | BoltSecurityLevel::User | BoltSecurityLevel::Secure
        )
    }

    /// Returns true if the security level requires user interaction for authorization.
    pub fn is_interactive(&self) -> bool {
        matches!(self, BoltSecurityLevel::User | BoltSecurityLevel::Secure)
    }

    /// Returns a human-readable identifier for the security level.
    pub fn display_name(&self) -> &'static str {
        match self {
            BoltSecurityLevel::None => "none",
            BoltSecurityLevel::User => "user",
            BoltSecurityLevel::Secure => "secure",
            BoltSecurityLevel::DpOnly => "dponly",
            BoltSecurityLevel::UsbOnly => "usbonly",
            BoltSecurityLevel::NoPcie => "nopcie",
            BoltSecurityLevel::Unknown => "unknown",
        }
    }
}

impl From<&str> for BoltDeviceType {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "host" => BoltDeviceType::Host,
            "peripheral" => BoltDeviceType::Peripheral,
            _ => BoltDeviceType::Unknown,
        }
    }
}

/// The Thunderbolt generation of the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoltGeneration {
    Unknown,
    Gen1,
    Gen2,
    Gen3,
    Gen4,
    Gen5,
    Future(u32),
}

impl From<u32> for BoltGeneration {
    fn from(val: u32) -> Self {
        match val {
            0 => Self::Unknown,
            1 => Self::Gen1,
            2 => Self::Gen2,
            3 => Self::Gen3,
            4 => Self::Gen4,
            5 => Self::Gen5,
            x => Self::Future(x),
        }
    }
}

impl std::fmt::Display for BoltGeneration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BoltGeneration::Unknown => write!(f, "?"),
            BoltGeneration::Gen1 => write!(f, "1"),
            BoltGeneration::Gen2 => write!(f, "2"),
            BoltGeneration::Gen3 => write!(f, "3"),
            BoltGeneration::Gen4 => write!(f, "4"),
            BoltGeneration::Gen5 => write!(f, "5"),
            BoltGeneration::Future(v) => write!(f, "{}", v),
        }
    }
}

/// The current connection and authorization status of a device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoltStatus {
    Unknown,
    Disconnected,
    Connecting,
    Connected,
    Authorizing,
    AuthError,
    Authorized,
}

/// Flags describing the authorization state of a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AuthFlags {
    pub is_secure: bool,
    pub is_boot: bool,
    pub no_key: bool,
    pub no_pcie: bool,
}

impl AuthFlags {
    /// Parses the comma-separated flags string from D-Bus.
    pub fn from_dbus_string(s: &str) -> Self {
        let mut flags = Self::default();
        for part in s.split(',') {
            match part.trim() {
                "secure" => flags.is_secure = true,
                "boot" => flags.is_boot = true,
                "nokey" => flags.no_key = true,
                "nopcie" => flags.no_pcie = true,
                _ => {} // Ignore unknown flags for forward compatibility
            }
        }
        flags
    }

    /// Returns true if the device requires user action to authorize.
    pub fn requires_user_action(&self) -> bool {
        !self.is_boot && (self.is_secure || !self.no_key)
    }

    /// Returns true if the device is hardware protected.
    pub fn is_hardware_protected(&self) -> bool {
        self.is_secure
    }
}

impl fmt::Display for AuthFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.is_secure {
            parts.push("secure");
        }
        if self.is_boot {
            parts.push("boot");
        }
        if self.no_key {
            parts.push("nokey");
        }
        if self.no_pcie {
            parts.push("nopcie");
        }

        if parts.is_empty() {
            write!(f, "none")
        } else {
            write!(f, "{}", parts.join(", "))
        }
    }
}

/// Typed representation of the `LinkSpeed` property (a{su}) from boltd.
///
/// Note: D-Bus keys use dots (e.g., `tx.speed`), not hyphens.
#[derive(Debug, Clone, Default, DeserializeDict, SerializeDict, Type)]
#[zbus(signature = "a{su}")]
pub struct LinkSpeedInfo {
    #[zbus(rename = "tx.lanes")]
    pub tx_lanes: Option<u32>,

    #[zbus(rename = "tx.speed")]
    pub tx_speed: Option<u32>,

    #[zbus(rename = "rx.lanes")]
    pub rx_lanes: Option<u32>,

    #[zbus(rename = "rx.speed")]
    pub rx_speed: Option<u32>,
}

impl LinkSpeedInfo {
    /// Calculates the total theoretical bandwidth (Tx + Rx) in Gb/s.
    pub fn total_bandwidth_gbps(&self) -> u32 {
        let tx = self.tx_lanes.unwrap_or(0) * self.tx_speed.unwrap_or(0);
        let rx = self.rx_lanes.unwrap_or(0) * self.rx_speed.unwrap_or(0);
        tx + rx
    }

    /// Returns true if the link is active (speed > 0).
    pub fn is_active(&self) -> bool {
        self.tx_speed.unwrap_or(0) > 0 || self.rx_speed.unwrap_or(0) > 0
    }
}

/// Represents a single Thunderbolt device.
#[derive(Debug, Clone)]
pub struct BoltDevice {
    pub path: OwnedObjectPath,
    pub uid: BoltUid,

    pub name: String,
    pub vendor: Option<String>,
    pub label: Option<String>,

    pub device_type: BoltDeviceType,
    pub status: BoltStatus,
    pub stored: bool,
    pub generation: BoltGeneration,
    pub auth_flags: AuthFlags,

    pub parent_uid: Option<BoltUid>,
    pub domain_uid: Option<BoltUid>,
    pub policy: Option<String>,
    pub has_key: bool,

    pub connect_time: Option<u64>,
    pub authorize_time: Option<u64>,
    pub store_time: Option<u64>,

    pub sysfs_path: Option<String>,
    pub link_speed: Option<LinkSpeedInfo>,

    /// Icon name suitable for direct use with `icon::from_name()`.
    pub icon: &'static str,
}

impl BoltDevice {
    /// Returns the label if set, otherwise falls back to the device name.
    pub fn display_name(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.name)
    }
}

/// A node in the Thunderbolt device topology tree.
///
/// TODO: Implement full tree construction logic in `refresh_devices_lists`.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TopologyNode {
    pub device: BoltDevice,
    pub children: Vec<TopologyNode>, // Supports daisy-chaining
}

/// Represents the state of a Thunderbolt domain (host controller).
///
/// TODO: Integrate into `BoltState` to support multi-domain setups.
#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct DomainState {
    pub uid: String,              // Domain UUID
    pub name: Option<String>,     // Optional domain name
    pub host: Option<BoltDevice>, // The root "Host" device
    pub trees: Vec<TopologyNode>, // Topology trees connected to this domain
}

/// The global state of the Thunderbolt subsystem.
#[derive(Debug, Clone, Default)]
pub struct BoltState {
    pub security_level: BoltSecurityLevel,
    pub devices: Vec<BoltDevice>,
    // TODO: Replace flat list with hierarchical domain structure
    // pub domains: Vec<DomainState>,
}

/// Commands sent to the background D-Bus worker task.
///
/// These messages trigger actions such as authorizing, enrolling, or forgetting devices.
#[derive(Debug, Clone)]
pub enum BoltCommand {
    /// Authorize a device for the current session (temporary).
    AuthorizeDevice(BoltUid),
    /// Enroll a device (store key for automatic future authorization).
    EnrollDevice(BoltUid),
    /// Forget a device (remove stored key and revoke authorization).
    ForgetDevice(BoltUid),
    /// Set a custom label for a device.
    SetDeviceLabel { uid: BoltUid, label: String },
}

/// Events emitted by the background D-Bus worker to the UI.
#[derive(Debug, Clone)]
pub enum BoltEvent {
    /// An error occurred during D-Bus communication or state processing.
    Error(BoltError),
    /// Initial state loaded. Includes the command sender channel for UI actions.
    ///
    /// FIXME: In a future agnostic library design, the command channel should be passed
    /// as an argument to the `run()` function instead of being embedded in this event.
    Init {
        sender: mpsc::Sender<BoltCommand>,
        state: BoltState,
    },
    /// The device list or state has changed. Contains the updated full state.
    DevicesChanged { state: BoltState },
    /// The D-Bus subscription stream has ended (e.g., daemon terminated).
    Finished,
}

/// Creates a subscription to the Thunderbolt D-Bus service.
///
/// This spawns a background task (`bolt_daemon_task`) that monitors `boltd`
/// and emits `BoltEvent` messages to the UI.
pub fn bolt_subscription(_id: u64) -> Subscription<BoltEvent> {
    Subscription::run(bolt_dbus::bolt_daemon_task)
}
