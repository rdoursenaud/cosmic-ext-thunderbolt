// SPDX-FileCopyrightText: 2026 Raphaël Doursenaud <raphael@doursenaud.fr>
// SPDX-License-Identifier: GPL-3.0-only

//! Background task for interacting with the Thunderbolt D-Bus service (`boltd`).
//!
//! This module spawns an asynchronous task that:
//! 1. Connects to the system D-Bus.
//! 2. Fetches the initial state of all devices.
//! 3. Listens for `DeviceAdded` and `DeviceRemoved` signals.
//! 4. Processes UI commands (Enroll, Forget, Label).
//! 5. Emits `BoltEvent` updates to the UI stream.

use crate::bolt_proxy;
use crate::thunderbolt::{
    AuthFlags, BoltCommand, BoltDevice, BoltDeviceType, BoltError, BoltEvent, BoltGeneration,
    BoltSecurityLevel, BoltState, BoltStatus, BoltUid, LinkSpeedInfo,
};
use crate::thunderbolt::{DBUS_ROOT_PATH, DBUS_SERVICE};
use async_stream::stream;
use futures::{Stream, StreamExt};
use std::pin::Pin;
use tokio::sync::mpsc;
use tokio::time::{Duration, interval};
use zbus::{Connection, zvariant::OwnedObjectPath};

use tracing::{debug, error, info, instrument, warn};
use zbus::proxy::CacheProperties;

/// Parses the D-Bus status string into a typed `BoltStatus`.
///
/// Takes into account the `stored` flag to distinguish between
/// a temporarily connected device and an authorized (stored) one.
fn parse_status(s: &str, stored: bool) -> BoltStatus {
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

/// Fetches and constructs a `BoltDevice` struct from a D-Bus object path.
///
/// This function creates a temporary proxy for the specific device path,
/// reads all relevant properties, normalizes them, and returns a populated struct.
/// Returns `None` if the device cannot be read or if critical properties are missing.
#[instrument(skip(connection, path), fields(path = %path))]
async fn fetch_device_details(
    connection: &Connection,
    path: &OwnedObjectPath,
) -> Option<BoltDevice> {
    // Build the proxy for the specific device path
    let builder = bolt_proxy::DeviceProxy::builder(connection)
        .destination(DBUS_SERVICE)
        .ok()?;

    let builder = builder.path(path.as_ref()).ok()?;

    let proxy = builder
        .cache_properties(CacheProperties::No) // Always fetch fresh data for devices
        .build()
        .await
        .ok()?;

    // --- Critical Properties ---
    let uid_raw = proxy.uid().await.ok()?;

    // --- Standard Properties ---
    let device_type_str = proxy.type_().await.unwrap_or_else(|_| "unknown".into());
    let name = proxy.name().await.unwrap_or_else(|_| "Unknown".into());
    let vendor = proxy.vendor().await.ok().filter(|v| !v.is_empty());
    let generation = proxy.generation().await.unwrap_or(0);
    let status_str = proxy.status().await.unwrap_or_else(|_| "unknown".into());
    let stored = proxy.stored().await.unwrap_or(false);
    let label_raw = proxy.label().await.unwrap_or_default();
    let auth_flags_raw = proxy.auth_flags().await.unwrap_or_default();

    // Normalize label: Only store if it differs from the default "Vendor Name" concatenation
    let label = {
        let trimmed = label_raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            let default_concat = vendor
                .as_ref()
                .map_or(name.clone(), |v| format!("{} {}", v, name));
            if trimmed == default_concat {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
    };

    // --- Relations (Parent / Domain) ---
    // Extract UID from D-Bus path (e.g., "/org/.../devices/0000_0000" -> "0000-0000")
    let parent_uid: Option<BoltUid> = match proxy.parent().await {
        Ok(p_path_str) => {
            if p_path_str == "/" {
                None
            } else {
                p_path_str
                    .rsplit('/')
                    .next()
                    .and_then(|s| BoltUid::new(s.replace('_', "-")))
            }
        }
        Err(_) => None,
    };

    let domain_uid: Option<BoltUid> = match proxy.domain().await {
        Ok(d_path_str) => {
            if d_path_str == "/" {
                None
            } else {
                d_path_str
                    .rsplit('/')
                    .next()
                    .and_then(|s| BoltUid::new(s.replace('_', "-")))
            }
        }
        Err(_) => None,
    };

    // --- Configuration & Security ---
    let policy = proxy.policy().await.ok().filter(|p| !p.is_empty());
    let has_key = proxy.key().await.map(|k| !k.is_empty()).unwrap_or(false);

    // --- Timestamps (convert seconds to milliseconds for UI consistency) ---
    let connect_time = proxy.connect_time().await.ok().map(|t| t * 1000);
    let authorize_time = proxy.authorize_time().await.ok().map(|t| t * 1000);
    let store_time = proxy.store_time().await.ok().map(|t| t * 1000);

    // --- System Info ---
    let sysfs_path = proxy.sysfs_path().await.ok().filter(|p| !p.is_empty());

    // --- Link Speed ---
    // Convert the raw HashMap<String, u32> from D-Bus into our typed LinkSpeedInfo struct.
    let link_speed = proxy.link_speed().await.ok().and_then(|map| {
        if map.is_empty() {
            return None;
        }

        let get_val = |key: &str| map.get(key).copied();

        let info = LinkSpeedInfo {
            tx_lanes: get_val("tx.lanes"),
            tx_speed: get_val("tx.speed"),
            rx_lanes: get_val("rx.lanes"),
            rx_speed: get_val("rx.speed"),
        };

        // Only report link speed if the link is actually active
        if info.is_active() { Some(info) } else { None }
    });

    Some(BoltDevice {
        path: path.clone(),
        uid: BoltUid::from(uid_raw),
        name,
        vendor,
        device_type: BoltDeviceType::from(device_type_str.as_str()),
        status: parse_status(&status_str, stored),
        stored,
        generation: BoltGeneration::from(generation),
        label,
        auth_flags: AuthFlags::from_dbus_string(&auth_flags_raw),
        parent_uid,
        domain_uid,
        policy,
        has_key,
        connect_time,
        authorize_time,
        store_time,
        sysfs_path,
        link_speed,
        icon: "thunderbolt-symbolic",
    })
}

/// The main background task that manages the D-Bus connection and event loop.
///
/// This function returns a stream of `BoltEvent` that the UI subscribes to.
/// It handles:
/// - Initial connection and state fetch.
/// - Listening to D-Bus signals (`DeviceAdded`, `DeviceRemoved`).
/// - Processing commands from the UI (via an MPSC channel).
/// - Periodic reconciliation to ensure state consistency.
pub fn bolt_daemon_task() -> Pin<Box<dyn Stream<Item = BoltEvent> + Send>> {
    Box::pin(stream! {
        info!("Starting bolt_daemon_task...");

        // 1. Connect to the system bus
        let connection = match Connection::system().await {
            Ok(c) => {
                info!("Connected to system bus.");
                c
            },
            Err(e) => {
                error!("D-Bus connection FAILED: {}", e);
                yield BoltEvent::Error(BoltError::DbusConnectionFailed);
                return;
            }
        };

        // 2. Create the Manager Proxy
        let builder = match bolt_proxy::ManagerProxy::builder(&connection).destination(DBUS_SERVICE) {
            Ok(b) => b,
            Err(e) => {
                error!("Failed to set destination: {}", e);
                yield BoltEvent::Error(BoltError::ProxyCreationFailed);
                return;
            }
        };

        let builder = match builder.path(DBUS_ROOT_PATH) {
            Ok(b) => b,
            Err(e) => {
                error!("Failed to set path: {}", e);
                yield BoltEvent::Error(BoltError::ProxyCreationFailed);
                return;
            }
        };

        let manager = match builder.build().await {
            Ok(m) => m,
            Err(e) => {
                error!("Manager creation FAILED: {}", e);
                if e.to_string().contains("ServiceUnknown") {
                    yield BoltEvent::Error(BoltError::ServiceNotFound);
                } else {
                    yield BoltEvent::Error(BoltError::ProxyCreationFailed);
                }
                return;
            }
        };

        info!("Manager Proxy created.");

        // 3. Fetch Initial State
        info!("Reading SecurityLevel...");
        let security_str = manager.security_level().await.unwrap_or_else(|e| {
            warn!("Failed to get security level: {}", e);
            "unknown".to_string()
        });
        let security_level = BoltSecurityLevel::from(security_str.as_str());

        let mut devices = Vec::new();
        debug!("Calling ListDevices...");
        match manager.list_devices().await {
            Ok(paths) => {
                info!("{} devices found.", paths.len());
                for p in paths {
                    if let Some(d) = fetch_device_details(&connection, &p).await {
                        devices.push(d);
                    }
                }
            },
            Err(e) => error!("list_devices error: {}", e),
        }

        // 4. Create Command Channel
        let (tx, mut rx) = mpsc::channel(100);

        let initial_state = BoltState {
            security_level,
            devices: devices.clone(),
        };

        info!("Sending Init with {} devices.", initial_state.devices.len());
        yield BoltEvent::Init {
            sender: tx,
            state: initial_state,
        };

        // 5. Subscribe to D-Bus Signals
        info!("Subscribing to signals...");
        let mut added = match manager.receive_device_added().await {
            Ok(s) => s,
            Err(e) => {
                error!("device_added subscription failed: {}", e);
                return;
            }
        };
        let mut removed = match manager.receive_device_removed().await {
            Ok(s) => s,
            Err(e) => {
                error!("device_removed subscription failed: {}", e);
                return;
            }
        };

        debug!("Main loop active.");

        // 6. Main Event Loop
        let mut reconciliation_timer = interval(Duration::from_secs(10)); // Periodic sync every 10s

        // Local state cache to avoid full reloads on every signal
        let mut current_devices = devices;
        let mut current_security = security_level;

        loop {
            let mut should_reload = false;

            tokio::select! {
                // --- Signal: Device Added ---
                Some(_sig) = added.next() => {
                    debug!("DeviceAdded signal received.");
                    should_reload = true;
                },
                // --- Signal: Device Removed ---
                Some(_sig) = removed.next() => {
                    debug!("DeviceRemoved signal received.");
                    should_reload = true;
                },
                // --- Command: UI Request ---
                Some(req) = rx.recv() => {
                    match req {
                        BoltCommand::EnrollDevice(uid) => {
                            info!("Enroll: {}", uid.as_str());
                            if let Err(e) = manager.enroll_device(uid.as_str(), "auto", "").await {
                                error!("Enroll error: {}", e);
                            }
                            should_reload = true;
                        },
                        BoltCommand::ForgetDevice(uid) => {
                            info!("Forget: {}", uid.as_str());
                            if let Err(e) = manager.forget_device(uid.as_str()).await {
                                error!("Forget error: {}", e);
                            }
                            should_reload = true;
                        },
                        BoltCommand::SetDeviceLabel { uid, label } => {
                            info!("Setting label for {}: {}", uid.as_str(), label);

                            // Fetch the current path dynamically to ensure we target the correct device
                            let result = async {
                                let path = manager.device_by_uid(uid.as_str()).await?;
                                let builder = bolt_proxy::DeviceProxy::builder(&connection);
                                let builder = builder.destination(DBUS_SERVICE)?;
                                let builder = builder.path(path.as_ref())?;
                                let proxy = builder.build().await?;
                                proxy.set_label(&label).await
                            }.await;

                            match result {
                                Ok(_) => {
                                    // Optimistic UI update: Update local cache immediately
                                    if let Some(d) = current_devices.iter_mut().find(|d| d.uid == uid) {
                                        d.label = if label.is_empty() { None } else { Some(label.clone()) };
                                        yield BoltEvent::DevicesChanged {
                                            state: BoltState {
                                                security_level: current_security,
                                                devices: current_devices.clone(),
                                            }
                                        };
                                    } else {
                                        // Edge case: Device exists on bus but not in local cache.
                                        // Trigger a full sync on next iteration to reconcile.
                                        debug!("Device {} updated but not in local cache, triggering sync.", uid.as_str());
                                        should_reload = true;
                                    }
                                }
                                Err(e) => error!("Failed to set label for {}: {}", uid.as_str(), e),
                            }
                            // Continue loop without reloading immediately if optimistic update succeeded
                            if !should_reload {
                                continue;
                            }
                        },
                        other => warn!("Unimplemented request: {:?}", other),
                    }
                }
                // --- Timer: Periodic Reconciliation ---
                _ = reconciliation_timer.tick() => {
                    debug!("Periodic reconciliation check...");
                    should_reload = true;
                }
                // --- Stream Closed ---
                else => { break; }
            }

            // --- Conditional Reload Logic ---
            if should_reload {
                debug!("Syncing devices after event...");
                match manager.list_devices().await {
                    Ok(paths) => {
                        let mut new_devs = Vec::with_capacity(paths.len());
                        debug!("RELOAD: ListDevices returned {} paths.", paths.len());
                        for p in paths {
                            debug!("  - Path: {}", p);
                            if let Some(d) = fetch_device_details(&connection, &p).await {
                                new_devs.push(d);
                            }
                        }

                        // Deep comparison to avoid unnecessary UI updates
                        let has_changed = if new_devs.len() != current_devices.len() {
                            true
                        } else {
                            // Check if every new device has an equivalent in the old list (UID + Status + Label)
                            !new_devs.iter().all(|new_d| {
                                current_devices.iter().any(|old_d| {
                                    old_d.uid == new_d.uid && old_d.status == new_d.status && old_d.label == new_d.label
                                })
                            })
                        };

                        if has_changed {
                            current_devices = new_devs;

                            // Optionally re-read security level in case it changed externally
                            if let Ok(sec_str) = manager.security_level().await {
                                current_security = BoltSecurityLevel::from(sec_str.as_str());
                            }

                            yield BoltEvent::DevicesChanged {
                                state: BoltState {
                                    security_level: current_security,
                                    devices: current_devices.clone(),
                                }
                            };
                        }
                    }
                    Err(e) => {
                        warn!("Sync failed, keeping stale state: {}", e);
                    }
                }
            }
        }
    })
}
