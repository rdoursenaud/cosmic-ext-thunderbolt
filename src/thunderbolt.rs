// SPDX-FileCopyrightText: 2026 Raphaël Doursenaud <raphael@doursenaud.fr>
// SPDX-License-Identifier: GPL-3.0-only

//! Thunderbolt™ D-Bus interface definitions and state management.
//!
//! This module defines the complete data model for Thunderbolt devices.
//! Some fields and methods are not yet used in the UI but are reserved for
//! future features (topology view, detailed device info, advanced commands).
//!
//!
#![allow(dead_code)]

use crate::bolt_dbus::bolt_daemon_task;
use const_format::concatcp;
use cosmic::iced::Subscription;
use std::fmt;
use tokio::sync::mpsc;
use zvariant::{DeserializeDict, OwnedObjectPath, SerializeDict, Type};
#[cfg(feature = "mock")]
use crate::bolt_dbus::mock_daemon_task;
#[cfg(feature = "mock")]
use std::env;
#[cfg(feature = "mock")]
use tracing::info;


// D-Bus Service and Path constants
pub const DBUS_SERVICE: &str = "org.freedesktop.bolt";
pub const DBUS_ROOT_PATH: &str = "/org/freedesktop/bolt";
pub const DBUS_DEVICES_PATH: &str = concatcp!(DBUS_ROOT_PATH, "/", "devices");
pub const DBUS_DOMAINS_PATH: &str = concatcp!(DBUS_ROOT_PATH, "/", "domains");

/// Represents specific error conditions when interacting with the Thunderbolt daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoltError {
    DbusConnectionFailed,
    ServiceNotFound,
    InvalidDestination,
    InvalidObjectPath,
    ProxyConnectionFailed,
    PropertyReadFailed,
    SignalSubscriptionFailed,
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

    /// Parses parent/domain property from D-Bus
    pub fn from_dbus(s: &str) -> Option<Self> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return None;
        }
        Self::new(trimmed)
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
        // Utilise la représentation interne déjà normalisée (avec des tirets)
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
    pub boot_acl: Option<Vec<BoltUid>>, // Lazy-load
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
///
/// Excerpt from the linux kernel thunderbolt module documentation:
#[allow(clippy::doc_markdown)]
/// [...]
///
/// Starting with Intel Falcon Ridge Thunderbolt controller there are 4
/// security levels available. Intel Titan Ridge added one more security level
/// (usbonly). The reason for these is the fact that the connected devices can
/// be DMA masters and thus read contents of the host memory without CPU and OS
/// knowing about it. There are ways to prevent this by setting up an IOMMU but
/// it is not always available for various reasons.
///
/// Some USB4 systems have a BIOS setting to disable PCIe tunneling. This is
/// treated as another security level (nopcie).
///
/// The security levels are as follows:
///
///   `none`
///     All devices are automatically connected by the firmware. No user
///     approval is needed. In BIOS settings this is typically called
///     *Legacy mode*.
///
///   `user`
///     User is asked whether the device is allowed to be connected.
///     Based on the device identification information available through
///     ``/sys/bus/thunderbolt/devices``, the user then can make the decision.
///     In BIOS settings this is typically called *Unique ID*.
///
///   `secure`
///     User is asked whether the device is allowed to be connected. In
///     addition to UUID the device (if it supports secure connect) is sent
///     a challenge that should match the expected one based on a random key
///     written to the ``key`` sysfs attribute. In BIOS settings this is
///     typically called *One time saved key*.
///
///   `dponly`
///     The firmware automatically creates tunnels for Display Port and
///     USB. No PCIe tunneling is done. In BIOS settings this is
///     typically called *Display Port Only*.
///
///   `usbonly`
///     The firmware automatically creates tunnels for the USB controller and
///     Display Port in a dock. All PCIe links downstream of the dock are
///     removed.
///
///   `nopcie`
///     PCIe tunneling is disabled/forbidden from the BIOS. Available in some
///     USB4 systems.
///
/// [...]
///
/// If the security level reads as ``user`` or ``secure`` the connected
/// device must be authorized by the user before PCIe tunnels are created
/// (e.g the PCIe device appears).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoltSecurityLevel {
    #[default]
    Unknown,
    None,    // Intel Falcon Ridge
    DpOnly,  // Intel Falcon Ridge, Technically means DP+USB only
    User,    // Intel Falcon Ridge
    Secure,  // Intel Falcon Ridge
    UsbOnly, // Starting at Intel Titan Ridge
    NoPcie,  // USB4 optional BIOS setting, Technically disables PCIe only for daisy-chained devices
}

impl From<&str> for BoltSecurityLevel {
    fn from(s: &str) -> Self {
        match s {
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

impl BoltSecurityLevel {
    /// Returns true if `PCIe` tunneling is allowed at this security level.
    pub fn allows_pcie(self) -> bool {
        matches!(
            self,
            BoltSecurityLevel::None | BoltSecurityLevel::User | BoltSecurityLevel::Secure
        )
    }

    /// Returns true if the security level requires user interaction for authorization.
    pub fn is_interactive(self) -> bool {
        matches!(self, BoltSecurityLevel::User | BoltSecurityLevel::Secure)
    }
}

impl From<&str> for BoltDeviceType {
    fn from(s: &str) -> Self {
        match s {
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

impl BoltStatus {
    /// Parses the D-Bus status string into a typed `BoltStatus`.
    ///
    /// Takes into account the `stored` flag to distinguish between
    /// a temporarily connected device and an authorized (stored) one.
    pub fn from_dbus(s: &str, stored: bool) -> Self {
        match s {
            "disconnected" => BoltStatus::Disconnected,
            "connecting" => BoltStatus::Connecting,
            "connected" => {
                if stored {
                    BoltStatus::Authorized
                } else {
                    BoltStatus::Connected
                }
            }
            "authorizing" => BoltStatus::Authorizing,
            "auth_error" => BoltStatus::AuthError,
            "authorized" => BoltStatus::Authorized,
            _ => BoltStatus::Unknown,
        }
    }
}

/// Flags describing the authorization state of a device.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AuthFlags {
    pub is_secure: bool,
    pub is_boot: bool,
    pub no_key: bool,
    pub no_pcie: bool,
}

impl AuthFlags {
    /// Parses the comma-separated flags string from D-Bus.
    pub fn from_dbus(s: &str) -> Self {
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
    pub fn requires_user_action(self) -> bool {
        !self.is_boot && (self.is_secure || !self.no_key)
    }

    /// Returns true if the device is hardware protected.
    pub fn is_hardware_protected(self) -> bool {
        self.is_secure
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
    pub fn from_dbus_map(map: &std::collections::HashMap<String, u32>) -> Option<Self> {
        if map.is_empty() {
            return None;
        }

        let get_val = |key: &str| map.get(key).copied();

        let info = Self {
            tx_lanes: get_val("tx.lanes"),
            tx_speed: get_val("tx.speed"),
            rx_lanes: get_val("rx.lanes"),
            rx_speed: get_val("rx.speed"),
        };

        // Ne retourne une valeur que si le lien est effectivement actif
        info.is_active().then_some(info)
    }

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

    pub label: Option<String>,
    pub name: String,
    pub vendor: Option<String>,

    pub device_type: BoltDeviceType,
    pub status: BoltStatus,
    pub stored: bool,
    pub generation: BoltGeneration,
    pub auth_flags: AuthFlags,

    pub parent_uid: Option<BoltUid>,
    pub domain_uid: Option<BoltUid>,
    //pub policy: BoltPolicy,
    pub has_key: bool,

    pub connect_time: Option<u64>,
    pub authorize_time: Option<u64>,
    pub store_time: Option<u64>,

    pub sysfs_path: Option<String>,
    pub link_speed: Option<LinkSpeedInfo>,
}

impl BoltDevice {
    // Normalize label: Only return if it differs from the default "Vendor Name" concatenation
    pub(crate) fn normalize_label(
        raw: &str,
        vendor: Option<&String>,
        name: &str,
    ) -> Option<String> {
        let trimmed = raw.trim();

        if trimmed.is_empty() {
            return None;
        }

        let default_concat = vendor
            .as_ref()
            .map_or(name.to_string(), |v| format!("{v} {name}"));

        if trimmed == default_concat {
            None
        } else {
            Some(trimmed.to_string())
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoltPolicy {
    Unknown,
    Auto,
    Manual,
    Ignore,
}

impl From<&str> for BoltPolicy {
    fn from(s: &str) -> Self {
        match s {
            "auto" => BoltPolicy::Auto,
            "manual" => BoltPolicy::Manual,
            "ignore" => BoltPolicy::Ignore,
            _ => BoltPolicy::Unknown,
        }
    }
}

/// The global state of the Thunderbolt subsystem.
#[derive(Debug, Clone, Default)]
pub struct BoltState {
    pub security_level: BoltSecurityLevel,
    pub devices: Vec<BoltDevice>,
    // TODO: Replace flat list with hierarchical domain structure
    // pub domains: Vec<DomainState>,
    // pub auth_mode: BoltAuthMode,
    // pub default_policy: BoltPolicy,
    // pub probing: bool,
    // pub power_supported: bool,
    // pub power_state: BoltPowerState,
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
    // /// Set policy for a device
    // SetDevicePolicy { uid: BoltUid, policy: Option<BoltPolicy> },
    // ForceDomainPower { uid: BoltUid, enable: bool },
    // SetDomainConfig { auth_mode: Option<BoltAuthMode>, default_policy: Option<BoltPolicy> },
    // UpdateDomainBootAcl { domain_uid: BoltUid, add: bool, device_uid: BoltUid },
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
///
/// Si la variable d'environnement `COSMIC_TB_MOCK` est définie (ex: "1"),
/// utilise le générateur de données factices (`mock_daemon_task`).
/// Sinon, se connecte au vrai service D-Bus (`bolt_daemon_task`).
pub fn bolt_subscription(_id: u64) -> Subscription<BoltEvent> {
    #[cfg(feature = "mock")]
    {
        let is_mock = env::var("COSMIC_TB_MOCK").unwrap_or_default() == "1";

        if is_mock {
            info!("MOCK MODE ENABLED: Using simulated Thunderbolt data.");
            return Subscription::run(mock_daemon_task)
        }
    }

    Subscription::run(bolt_daemon_task)
}
