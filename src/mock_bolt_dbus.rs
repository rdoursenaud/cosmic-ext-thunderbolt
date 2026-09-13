// SPDX-FileCopyrightText: 2026 Raphaël Doursenaud <raphael@doursenaud.fr>
// SPDX-License-Identifier: GPL-3.0-only

#![cfg(feature = "mock")]
#![allow(dead_code)]

//! Mock data generators for UI testing without hardware.
//!
//! Enable by setting the environment variable: COSMIC_TB_MOCK=1

// TODO: Move to environment variables to allow dynamic testing?
const N_CONNECTED_DEVICES: usize = 10;
const N_DISCONNECTED_DEVICES: usize = 20;

use crate::thunderbolt::{
    AuthFlags, BoltDevice, BoltDeviceType, BoltGeneration, BoltSecurityLevel, BoltState,
    BoltStatus, BoltUid, LinkSpeedInfo,
};
use std::time::{SystemTime, UNIX_EPOCH};

/// Helper to generate a valid BoltUid from a simple string
fn mock_uid(seed: &str) -> BoltUid {
    let raw = format!("{}-0000-0000-0000-000000000000", seed);
    BoltUid::new(raw).unwrap_or_else(|| BoltUid::from("00000000-0000-0000-0000-000000000000"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// Scenario 1: Low security level (None) with devices awaiting authorization.
pub fn scenario_insecure_security() -> BoltState {
    BoltState {
        security_level: BoltSecurityLevel::None,
        devices: vec![
            BoltDevice {
                uid: mock_uid("11111111"),
                path: zbus::zvariant::OwnedObjectPath::try_from(
                    "/org/freedesktop/bolt/devices/11111111_0000_0000_0000_000000000000",
                )
                .unwrap(),
                name: "External SSD".into(),
                vendor: Some("Samsung".into()),
                label: None,
                device_type: BoltDeviceType::Peripheral,
                status: BoltStatus::Connected,
                stored: false,
                generation: BoltGeneration::Gen3,
                auth_flags: AuthFlags {
                    is_secure: false,
                    is_boot: false,
                    no_key: true,
                    no_pcie: false,
                },
                parent_uid: None,
                domain_uid: Some(mock_uid("host")),
                has_key: false,
                connect_time: Some(now_ms()),
                authorize_time: None,
                store_time: None,
                sysfs_path: Some("/sys/bus/thunderbolt/devices/1-0".into()),
                link_speed: Some(LinkSpeedInfo {
                    tx_lanes: Some(2),
                    tx_speed: Some(10), // 20 Gbps
                    rx_lanes: Some(2),
                    rx_speed: Some(10),
                }),
            },
            BoltDevice {
                uid: mock_uid("22222222"),
                path: zbus::zvariant::OwnedObjectPath::try_from(
                    "/org/freedesktop/bolt/devices/22222222_0000_0000_0000_000000000000",
                )
                .unwrap(),
                name: "Thunderbolt Dock".into(),
                vendor: Some("CalDigit".into()),
                label: Some("Mon Dock Principal".into()),
                device_type: BoltDeviceType::Peripheral,
                status: BoltStatus::Authorized,
                stored: true,
                generation: BoltGeneration::Gen4,
                auth_flags: AuthFlags {
                    is_secure: false,
                    is_boot: false,
                    no_key: false,
                    no_pcie: false,
                },
                parent_uid: None,
                domain_uid: Some(mock_uid("host")),
                has_key: true,
                connect_time: Some(now_ms() - 3600000),
                authorize_time: Some(now_ms() - 3600000),
                store_time: Some(now_ms() - 3600000),
                sysfs_path: Some("/sys/bus/thunderbolt/devices/1-1".into()),
                link_speed: Some(LinkSpeedInfo {
                    tx_lanes: Some(4),
                    tx_speed: Some(10), // 40 Gbps
                    rx_lanes: Some(4),
                    rx_speed: Some(10),
                }),
            },
        ],
    }
}

/// Scenario 2: Authentication error and unknown device.
pub fn scenario_auth_errors() -> BoltState {
    BoltState {
        security_level: BoltSecurityLevel::Secure,
        devices: vec![
            BoltDevice {
                uid: mock_uid("deadbeef"),
                path: zbus::zvariant::OwnedObjectPath::try_from(
                    "/org/freedesktop/bolt/devices/deadbeef_0000_0000_0000_000000000000",
                )
                .unwrap(),
                name: "Unknown Device".into(),
                vendor: None,
                label: None,
                device_type: BoltDeviceType::Unknown,
                status: BoltStatus::AuthError, // Critical authentication error
                stored: false,
                generation: BoltGeneration::Unknown,
                auth_flags: AuthFlags::default(),
                parent_uid: None,
                domain_uid: Some(mock_uid("host")),
                has_key: false,
                connect_time: Some(now_ms()),
                authorize_time: None,
                store_time: None,
                sysfs_path: None,
                link_speed: None,
            },
            BoltDevice {
                uid: mock_uid("cafebab1"),
                path: zbus::zvariant::OwnedObjectPath::try_from(
                    "/org/freedesktop/bolt/devices/cafebab1_0000_0000_0000_000000000000",
                )
                .unwrap(),
                name: "eGPU Enclosure".into(),
                vendor: Some("Razer".into()),
                label: None,
                device_type: BoltDeviceType::Peripheral,
                status: BoltStatus::Connected,
                stored: false,
                generation: BoltGeneration::Gen3,
                auth_flags: AuthFlags {
                    is_secure: true,
                    is_boot: false,
                    no_key: false,
                    no_pcie: false,
                },
                parent_uid: None,
                domain_uid: Some(mock_uid("host")),
                has_key: false,
                connect_time: Some(now_ms()),
                authorize_time: None,
                store_time: None,
                sysfs_path: Some("/sys/bus/thunderbolt/devices/2-0".into()),
                link_speed: Some(LinkSpeedInfo {
                    tx_lanes: Some(4),
                    tx_speed: Some(10),
                    rx_lanes: Some(4),
                    rx_speed: Some(10),
                }),
            },
        ],
    }
}

/// Scenario 3: Complex topology (Daisy chain) and disconnected devices.
pub fn scenario_complex_topology() -> BoltState {
    // Note: Tree reconstruction logic is not yet active in the UI,
    // but this allows testing long list rendering and scrolling.
    let mut devices = Vec::new();

    // Host
    devices.push(BoltDevice {
        uid: mock_uid("host0001"),
        path: zbus::zvariant::OwnedObjectPath::try_from(
            "/org/freedesktop/bolt/devices/host0001_0000_0000_0000_000000000000",
        )
        .unwrap(),
        name: "Intel Thunderbolt Controller".into(),
        vendor: Some("Intel".into()),
        label: None,
        device_type: BoltDeviceType::Host,
        status: BoltStatus::Authorized,
        stored: true,
        generation: BoltGeneration::Gen4,
        auth_flags: AuthFlags::default(),
        parent_uid: None,
        domain_uid: None,
        has_key: true,
        connect_time: Some(now_ms()),
        authorize_time: Some(now_ms()),
        store_time: Some(now_ms()),
        sysfs_path: Some("/sys/bus/thunderbolt/devices/0-0".into()),
        link_speed: None,
    });

    // N connected devices (to test scrolling)
    for i in 0..N_CONNECTED_DEVICES {
        devices.push(BoltDevice {
            uid: mock_uid(&format!("{:08x}", i + 100)),
            path: zbus::zvariant::OwnedObjectPath::try_from(format!(
                "/org/freedesktop/bolt/devices/{:08x}_0000_0000_0000_000000000000",
                i + 100
            ))
            .unwrap(),
            name: format!("Device {}", i),
            vendor: Some("Mock Vendor".into()),
            label: if i % 3 == 0 {
                Some(format!("Custom Label {}", i))
            } else {
                None
            },
            device_type: BoltDeviceType::Peripheral,
            status: if i % 4 == 0 {
                BoltStatus::Authorized
            } else {
                BoltStatus::Connected
            },
            stored: i % 4 == 0,
            generation: BoltGeneration::Gen3,
            auth_flags: AuthFlags::default(),
            parent_uid: if i > 0 {
                Some(mock_uid(&format!("{:08x}", i - 1 + 100)))
            } else {
                Some(mock_uid("host0001"))
            },
            domain_uid: Some(mock_uid("host0001")),
            has_key: true,
            connect_time: Some(now_ms()),
            authorize_time: Some(now_ms()),
            store_time: Some(now_ms()),
            sysfs_path: Some(format!("/sys/bus/thunderbolt/devices/1-{}", i)),
            link_speed: Some(LinkSpeedInfo {
                tx_lanes: Some(2),
                tx_speed: Some(10),
                rx_lanes: Some(2),
                rx_speed: Some(10),
            }),
        });
    }

    // N disconnected devices
    for i in 0..N_DISCONNECTED_DEVICES {
        devices.push(BoltDevice {
            uid: mock_uid(&format!("{:08x}", i + 200)),
            path: zbus::zvariant::OwnedObjectPath::try_from(format!(
                "/org/freedesktop/bolt/devices/{:08x}_0000_0000_0000_000000000000",
                i + 200
            ))
            .unwrap(),
            name: format!("Old Device {}", i),
            vendor: Some("Legacy Corp".into()),
            label: None,
            device_type: BoltDeviceType::Peripheral,
            status: BoltStatus::Disconnected,
            stored: true,
            generation: BoltGeneration::Gen2,
            auth_flags: AuthFlags::default(),
            parent_uid: None,
            domain_uid: Some(mock_uid("host0001")),
            has_key: true,
            connect_time: Some(now_ms() - 86400000),
            authorize_time: Some(now_ms() - 86400000),
            store_time: Some(now_ms() - 86400000),
            sysfs_path: None,
            link_speed: None,
        });
    }

    BoltState {
        security_level: BoltSecurityLevel::User,
        devices,
    }
}
