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
    BoltSecurityLevel, BoltState, BoltStatus, BoltUid, DBUS_ROOT_PATH, DBUS_SERVICE, LinkSpeedInfo,
};

use async_stream::stream;
use futures::{Stream, StreamExt};
use std::pin::Pin;
use tokio::sync::mpsc;
use tokio::time::{Duration, interval};
use zbus::{Connection, zvariant::OwnedObjectPath};

use tracing::{debug, error, info, instrument, warn};

#[cfg(feature = "mock")]
use crate::mock_bolt_dbus;


const RECONCILIATION_INTERVAL_SECS: u64 = 30;

/// Creates Bolt Manager proxy
async fn create_manager(
    connection: &Connection,
) -> Result<bolt_proxy::ManagerProxy<'_>, BoltEvent> {
    info!("Creating Manager Proxy...");

    let builder = bolt_proxy::ManagerProxy::builder(connection)
        .destination(DBUS_SERVICE)
        .map_err(|e| {
            error!("Failed to set destination: {e}");
            BoltEvent::Error(BoltError::InvalidDestination)
        })?;

    let builder = builder.path(DBUS_ROOT_PATH).map_err(|e| {
        error!("Failed to set path: {e}");
        BoltEvent::Error(BoltError::InvalidObjectPath)
    })?;

    builder
        .build()
        .await
        .inspect_err(|e| error!("Manager creation FAILED: {e}"))
        .map_err(|e| {
            if e.to_string().contains("ServiceUnknown") {
                BoltEvent::Error(BoltError::ServiceNotFound)
            } else {
                BoltEvent::Error(BoltError::ProxyConnectionFailed)
            }
        })
}

/// Fetches and constructs a `BoltDevice` struct from a D-Bus object path.
///
/// This function creates a temporary proxy for the specific device path,
/// reads all relevant properties, normalizes them, and returns a populated struct.
/// Returns `None` if the device cannot be read or if critical properties are missing.
#[instrument(skip(connection, path), fields(path = %path))]
#[allow(clippy::too_many_lines)] // Acceptable due to sequential D-Bus property reads with uniform error handling
async fn fetch_device_details(
    connection: &Connection,
    path: &OwnedObjectPath,
) -> Result<Option<BoltDevice>, BoltEvent> {
    // Build the proxy for the specific device path
    let builder = bolt_proxy::DeviceProxy::builder(connection)
        .destination(DBUS_SERVICE)
        .map_err(|e| {
            warn!("Failed to set destination: {}", e);
            BoltEvent::Error(BoltError::InvalidDestination)
        })?;

    let builder = match builder.path(path.as_ref()) {
        Ok(b) => b,
        Err(e) => {
            warn!("Invalid object path for {}: {}", path, e);
            return Err(BoltEvent::Error(BoltError::InvalidObjectPath));
        }
    };

    let proxy = match builder.build().await {
        Ok(p) => p,
        Err(e) => {
            warn!("Failed to build proxy for {}: {}", path, e);
            return Err(BoltEvent::Error(BoltError::ProxyConnectionFailed));
        }
    };

    // --- Critical Properties ---
    let uid_raw = proxy
        .uid()
        .await
        .inspect_err(|e| warn!("Failed to read uid for {}: {}", path, e));

    let uid_raw = match uid_raw {
        Ok(val) => val,
        Err(_) => return Err(BoltEvent::Error(BoltError::PropertyReadFailed)),
    };

    // --- Standard Properties ---
    let device_type_str = proxy
        .type_()
        .await
        .inspect_err(|e| warn!("Failed to read type for {}: {}", path, e))
        .unwrap_or_else(|_| "unknown".into());
    let name = proxy
        .name()
        .await
        .inspect_err(|e| warn!("Failed to read name for {}: {}", path, e))
        .unwrap_or_else(|_| "Unknown".into());
    let vendor = proxy
        .vendor()
        .await
        .inspect_err(|e| warn!("Failed to read vendor for {}: {}", path, e))
        .ok()
        .filter(|v| !v.is_empty());
    let generation = proxy
        .generation()
        .await
        .inspect_err(|e| warn!("Failed to read generation for {}: {}", path, e))
        .unwrap_or(0);
    let status_str = proxy
        .status()
        .await
        .inspect_err(|e| warn!("Failed to read status for {}: {}", path, e))
        .unwrap_or_else(|_| "unknown".into());
    let stored = proxy
        .stored()
        .await
        .inspect_err(|e| warn!("Failed to read stored for {}: {}", path, e))
        .unwrap_or(false);
    let label_raw = proxy
        .label()
        .await
        .inspect_err(|e| warn!("Failed to read label_raw for {}: {}", path, e))
        .unwrap_or_default();
    let auth_flags_raw = proxy
        .auth_flags()
        .await
        .inspect_err(|e| warn!("Failed to read auth_flags for {}: {}", path, e))
        .unwrap_or_default();

    // --- Relations (Parent / Domain) ---
    let parent_uid = proxy
        .parent()
        .await
        .inspect_err(|e| warn!("Failed to read parent for {}: {}", path, e))
        .unwrap_or_default();
    let domain_uid = proxy
        .domain()
        .await
        .inspect_err(|e| warn!("Failed to read domain for {}: {}", path, e))
        .unwrap_or_default();

    // --- Configuration & Security ---
    // let policy = proxy.policy().await.inspect_err(|e| warn!("Failed to read policy for {}: {}", path, e)).ok().filter(|p| !p.is_empty());
    let has_key = proxy
        .key()
        .await
        .inspect_err(|e| warn!("Failed to read key for {}: {}", path, e))
        .is_ok_and(|k| !k.is_empty());

    // --- Timestamps (convert seconds to milliseconds for UI consistency) ---
    let connect_time = proxy
        .connect_time()
        .await
        .inspect_err(|e| warn!("Failed to read connect_time for {}: {}", path, e))
        .ok()
        .map(|t| t * 1000);
    let authorize_time = proxy
        .authorize_time()
        .await
        .inspect_err(|e| warn!("Failed to read authorize_time for {}: {}", path, e))
        .ok()
        .map(|t| t * 1000);
    let store_time = proxy
        .store_time()
        .await
        .inspect_err(|e| warn!("Failed to read store_time for {}: {}", path, e))
        .ok()
        .map(|t| t * 1000);

    // --- System Info ---
    let sysfs_path = proxy
        .sysfs_path()
        .await
        .inspect_err(|e| warn!("Failed to read sysfs_path for {}: {}", path, e))
        .ok()
        .filter(|p| !p.is_empty());

    // --- Link Speed ---
    let link_speed_raw = proxy
        .link_speed()
        .await
        .inspect_err(|e| warn!("Failed to read link_speed for {}: {}", path, e))
        .ok();

    Ok(Some(BoltDevice {
        path: path.clone(),
        uid: BoltUid::from(uid_raw),
        label: BoltDevice::normalize_label(&label_raw, vendor.as_ref(), &name),
        name,
        vendor,
        device_type: BoltDeviceType::from(device_type_str.as_str()),
        status: BoltStatus::from_dbus(&status_str, stored),
        stored,
        generation: BoltGeneration::from(generation),
        auth_flags: AuthFlags::from_dbus(&auth_flags_raw),
        parent_uid: BoltUid::from_dbus(&parent_uid),
        domain_uid: BoltUid::from_dbus(&domain_uid),
        //policy,
        has_key,
        connect_time,
        authorize_time,
        store_time,
        sysfs_path,
        link_speed: link_speed_raw
            .as_ref()
            .and_then(LinkSpeedInfo::from_dbus_map),
    }))
}

/// Helper function to fetch device details for a list of paths, ignoring critical errors per device.
///
/// This ensures that a single faulty device does not prevent the rest of the list from being processed.
async fn fetch_devices_safe(
    connection: &Connection,
    paths: Vec<OwnedObjectPath>,
) -> Vec<BoltDevice> {
    let mut devices = Vec::with_capacity(paths.len());

    for p in paths {
        match fetch_device_details(connection, &p).await {
            Ok(Some(d)) => devices.push(d),
            Ok(None) => {
                // Proxy creation failed for this specific path, silently ignore
                debug!("Failed to create proxy for {}, skipping.", p);
                continue;
            }
            Err(e) => {
                // Critical property read failed (e.g., UID)
                warn!("Skipping device {} due to critical error: {:?}", p, e);
                continue;
            }
        }
    }

    devices
}

async fn fetch_initial_state(
    manager: &bolt_proxy::ManagerProxy<'_>,
    connection: &Connection,
) -> (BoltSecurityLevel, Vec<BoltDevice>) {
    info!("Reading initial state...");

    // 1. Security Level
    let security_str = manager
        .security_level()
        .await
        .inspect_err(|e| warn!("Failed to get security level: {e}"))
        .unwrap_or_else(|_| "unknown".to_string());
    let security_level = BoltSecurityLevel::from(security_str.as_str());

    // 2. Device List
    let mut devices = Vec::new();
    match manager.list_devices().await {
        Ok(paths) => {
            info!("{} devices found.", paths.len());
            devices = fetch_devices_safe(connection, paths).await;
        }
        Err(e) => error!("list_devices error: {e}"),
    }

    (security_level, devices)
}

/// Subscribe to D-Bus signals and return streams.
async fn subscribe_signals(
    manager: &bolt_proxy::ManagerProxy<'_>,
) -> Result<(impl Stream<Item = ()> + Send, impl Stream<Item = ()> + Send), BoltEvent> {
    debug!("Subscribing to signals...");

    let added = manager
        .receive_device_added()
        .await
        .inspect_err(|e| error!("device_added subscription failed: {e}"))
        .map_err(|_| BoltEvent::Error(BoltError::SignalSubscriptionFailed))?;

    let removed = manager
        .receive_device_removed()
        .await
        .inspect_err(|e| error!("device_removed subscription failed: {e}"))
        .map_err(|_| BoltEvent::Error(BoltError::SignalSubscriptionFailed))?;

    // Map signals to keep only the "something happened" event
    // since a sync is performed anyway.
    let added_stream = added.map(|_| ());
    let removed_stream = removed.map(|_| ());

    Ok((added_stream, removed_stream))
}

/// Executes a D-Bus command and returns true if a sync is necessary.
async fn execute_command(
    manager: &bolt_proxy::ManagerProxy<'_>,
    connection: &Connection,
    req: BoltCommand,
) -> Result<bool, String> {
    // Returns Ok(should_sync) or Err(error message)
    match req {
        BoltCommand::EnrollDevice(uid) => {
            info!("Enroll: {}", uid.as_str());
            manager
                .enroll_device(uid.as_str(), "auto", "")
                .await
                .map_err(|e| format!("Enroll error: {e}"))?;
            Ok(true)
        }
        BoltCommand::ForgetDevice(uid) => {
            info!("Forget: {}", uid.as_str());
            manager
                .forget_device(uid.as_str())
                .await
                .map_err(|e| format!("Forget error: {e}"))?;
            Ok(true)
        }
        BoltCommand::SetDeviceLabel { uid, label } => {
            info!("Setting label for {}: {}", uid.as_str(), label);

            let path = manager
                .device_by_uid(uid.as_str())
                .await
                .inspect_err(|e| warn!("Failed to get device: {e}"))
                .map_err(|e| format!("Device lookup failed: {e}"))?;

            let builder = bolt_proxy::DeviceProxy::builder(connection)
                .destination(DBUS_SERVICE)
                .map_err(|e| format!("Builder failed: {e}"))?;

            let builder = builder
                .path(path.as_ref())
                .map_err(|e| format!("Path failed: {e}"))?;

            let proxy = builder
                .build()
                .await
                .inspect_err(|e| warn!("Failed to build proxy: {e}"))
                .map_err(|e| format!("Proxy build failed: {e}"))?;

            proxy
                .set_label(&label)
                .await
                .inspect_err(|e| warn!("Failed to set label: {e}"))
                .map_err(|e| format!("Set label failed: {e}"))?;

            Ok(false) // No sync needed on success (optimistic update handled in the loop)
        }
        BoltCommand::AuthorizeDevice(_) => {
            warn!("Unimplemented request: AuthorizeDevice");
            Ok(false)
        }
    }
}

async fn sync_devices(
    manager: &bolt_proxy::ManagerProxy<'_>,
    connection: &Connection,
    current_devices: &[BoltDevice],
    current_security: BoltSecurityLevel,
) -> Option<BoltState> {
    debug!("Syncing devices after event...");

    let paths = match manager.list_devices().await {
        Ok(p) => p,
        Err(e) => {
            warn!("Sync failed, keeping stale state: {e}");
            return None;
        }
    };

    debug!("SYNC: ListDevices returned {} paths.", paths.len());

    let new_devs = fetch_devices_safe(connection, paths).await;

    // Deep comparison
    let has_changed = if new_devs.len() == current_devices.len() {
        !new_devs.iter().all(|new_d| {
            current_devices.iter().any(|old_d| {
                old_d.uid == new_d.uid && old_d.status == new_d.status && old_d.label == new_d.label
            })
        })
    } else {
        true
    };

    if has_changed {
        Some(BoltState {
            security_level: current_security,
            devices: new_devs,
        })
    } else {
        Some(BoltState {
            security_level: current_security,
            devices: current_devices.to_vec(), // Keep the old list
        })
    }
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
                error!("D-Bus connection FAILED: {e}");
                yield BoltEvent::Error(BoltError::DbusConnectionFailed);
                return;
            }
        };

        // 2. Create the Manager Proxy
        let manager = match create_manager(&connection).await {
            Ok(m) => m,
            Err(e) => { yield e; return; }
        };
        info!("Manager Proxy created.");

        // 3. Fetch Initial State
        let (security_level, devices) = fetch_initial_state(&manager, &connection).await;

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
        let (mut added, mut removed) = match subscribe_signals(&manager).await {
                 Ok(streams) => streams,
                 Err(e) => { yield e; return; }
        };

        // 6. Main Event Loop
        let mut reconciliation_timer = interval(Duration::from_secs(RECONCILIATION_INTERVAL_SECS)); // Periodic sync
        let mut current_devices = devices;
        debug!("Main loop active.");
        loop {
            let mut should_sync = false;

            tokio::select! {
                // --- Signal: Device Added ---
                Some(_sig) = added.next() => {
                    debug!("DeviceAdded signal received.");
                    should_sync = true;
                },
                // --- Signal: Device Removed ---
                Some(_sig) = removed.next() => {
                    debug!("DeviceRemoved signal received.");
                    should_sync = true;
                },
                // --- Command: UI Request ---
                Some(req) = rx.recv() => {
                    match execute_command(&manager, &connection, req).await {
                        Ok(true) => should_sync = true,
                        Ok(false) => should_sync = false,
                        Err(e) => error!("Command execution failed: {e}"),
                    }
                }
                // --- Timer: Periodic Reconciliation ---
                _ = reconciliation_timer.tick() => {
                    debug!("Periodic reconciliation check...");
                    should_sync = true;
                }
                // --- Stream Closed ---
                else => { break; }
            }

            // --- Conditional Sync Logic ---
            if should_sync && let Some(new_state) = sync_devices(&manager, &connection, &current_devices, security_level).await {
                current_devices = new_state.devices.clone();
                yield BoltEvent::DevicesChanged { state: new_state };
            }
        }
    })
}

//--- UI TESTS ---

/// Simulated task for UI tests
/// Emits predefined states
#[cfg(feature = "mock")]
pub fn mock_daemon_task() -> Pin<Box<dyn Stream<Item = BoltEvent> + Send>> {
    Box::pin(stream! {
        info!("Starting MOCK bolt_daemon_task...");

        // Dummy command channel (commands are ignored or logged)
        let (tx, mut rx) = mpsc::channel(100);

        // Initial scenario: Load the one you want to test.
        // To change scenarios, modify the line below or implement a cycling logic.
        let initial_state = mock_bolt_dbus::scenario_complex_topology();
        // let initial_state = mock_bolt_dbus::scenario_insecure_security();
        // let initial_state = mock_bolt_dbus::scenario_auth_errors();

        yield BoltEvent::Init {
            sender: tx,
            state: initial_state.clone(),
        };

        let mut tick = 0;
        let mut timer = interval(Duration::from_secs(5)); // Cycle every 5s for demo purposes

        loop {
            tokio::select! {
                // Ignore UI commands in mock mode, or log them
                Some(req) = rx.recv() => {
                    info!("MOCK: Command received (ignored): {:?}", req);
                    // Optional: Simulate a response here to test optimistic UI updates
                }
                _ = timer.tick() => {
                    tick += 1;

                    // Dynamic example: Periodically change a device's status
                    let mut state = initial_state.clone();

                    if tick % 2 == 0 {
                        // Simulate an appearance/disappearance or status change
                        if let Some(dev) = state.devices.iter_mut().find(|d| d.vendor.as_deref() == Some("Razer")) {
                            if dev.status == BoltStatus::Connected {
                                dev.status = BoltStatus::Authorized;
                                dev.stored = true;
                                info!("MOCK: Device authorized automatically");
                            }
                        }
                    }

                    yield BoltEvent::DevicesChanged { state };
                }
                else => { break; }
            }
        }
    })
}
