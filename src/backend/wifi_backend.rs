//! WiFi backend implementation using iwd (Intel Wireless Daemon)

use async_channel::Sender;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::AbortHandle;
use zbus::zvariant::OwnedObjectPath;

use super::manager::{BackendEvent, WifiNetworkData};
use super::wifi::iwd_proxy::{DeviceProxy, KnownNetworkProxy, NetworkProxy, StationProxy};

/// Convert iwd D-Bus errors to user-friendly messages
pub fn format_iwd_error(e: &zbus::Error) -> String {
    let s = e.to_string();
    if s.contains("Aborted") || s.contains("Canceled") {
        "Connection cancelled".into()
    } else if s.contains("InvalidFormat") || s.contains("InvalidArguments") {
        "Invalid password".into()
    } else if s.contains("AuthenticationFailed") {
        "Wrong password".into()
    } else if s.contains("NotConnected") {
        "Not connected".into()
    } else if s.contains("Busy") {
        "Device is busy, try again".into()
    } else if s.contains("NotFound") {
        "Network not found".into()
    } else if s.contains("NoAgent") {
        "No agent registered".into()
    } else if s.contains("Failed") {
        "Connection failed".into()
    } else {
        format!("Connection failed: {}", s)
    }
}

/// Find iwd Device path (exists even when WiFi is off)
pub async fn find_iwd_device_path(
    conn: &zbus::Connection,
) -> Result<OwnedObjectPath, Box<dyn std::error::Error + Send + Sync>> {
    use zbus::fdo::ObjectManagerProxy;

    let obj_manager = ObjectManagerProxy::builder(conn)
        .destination("net.connman.iwd")?
        .path("/")?
        .build()
        .await?;

    let objects = obj_manager.get_managed_objects().await?;

    let device_path = objects
        .iter()
        .find(|(_, ifaces)| ifaces.contains_key("net.connman.iwd.Device"))
        .map(|(path, _)| path.clone())
        .ok_or("No WiFi device found. Is iwd running with a WiFi adapter?")?;

    tracing::info!("Found iwd device at {}", device_path);
    Ok(device_path)
}

/// Check if Station interface exists (only when powered)
pub async fn has_station_interface(conn: &zbus::Connection, device_path: &OwnedObjectPath) -> bool {
    use zbus::fdo::ObjectManagerProxy;

    let obj_manager = match ObjectManagerProxy::builder(conn)
        .destination("net.connman.iwd")
        .and_then(|b| b.path("/"))
    {
        Ok(builder) => match builder.build().await {
            Ok(proxy) => proxy,
            Err(_) => return false,
        },
        Err(_) => return false,
    };

    let Ok(objects) = obj_manager.get_managed_objects().await else {
        return false;
    };

    objects
        .get(device_path)
        .map(|ifaces| ifaces.contains_key("net.connman.iwd.Station"))
        .unwrap_or(false)
}

/// Get list of WiFi networks from iwd Station
pub async fn get_wifi_networks(
    conn: &zbus::Connection,
    station: &StationProxy<'_>,
) -> Result<Vec<WifiNetworkData>, Box<dyn std::error::Error + Send + Sync>> {
    let ordered = station.get_ordered_networks().await?;
    tracing::debug!("Found {} networks from iwd", ordered.len());
    let mut networks = Vec::with_capacity(ordered.len());

    let connected_path = station.connected_network().await.ok();

    for (path, signal_strength) in ordered {
        let network = NetworkProxy::builder(conn)
            .path(path.clone())?
            .build()
            .await?;

        let name = network.name().await.unwrap_or_default();
        let network_type = network.network_type().await.unwrap_or_else(|_| "open".into());
        let connected = connected_path
            .as_ref()
            .map(|cp| cp.as_str() == path.as_str())
            .unwrap_or(false);
        let known = network.known_network().await.is_ok();

        tracing::debug!(
            "  Network: {} ({} dBm, type={}, connected={}, known={})",
            name,
            signal_strength / 100,
            network_type,
            connected,
            known
        );
        networks.push(WifiNetworkData {
            path: path.to_string(),
            name,
            network_type,
            signal_strength,
            connected,
            known,
        });
    }

    tracing::info!("Loaded {} WiFi networks", networks.len());
    Ok(networks)
}

/// Helper to create NetworkProxy from path
async fn create_network_proxy(
    conn: &zbus::Connection,
    path: &str,
) -> Result<NetworkProxy<'static>, String> {
    let owned_path: OwnedObjectPath = path
        .try_into()
        .map_err(|e| format!("Invalid network path: {}", e))?;
    NetworkProxy::builder(conn)
        .path(owned_path)
        .map_err(|e| format!("Invalid network path: {}", e))?
        .build()
        .await
        .map_err(|e| format!("Failed to create network proxy: {}", e))
}

/// Helper to create KnownNetworkProxy from path
async fn create_known_network_proxy(
    conn: &zbus::Connection,
    path: OwnedObjectPath,
) -> Result<KnownNetworkProxy<'static>, String> {
    KnownNetworkProxy::builder(conn)
        .path(path)
        .map_err(|e| format!("Invalid known network path: {}", e))?
        .build()
        .await
        .map_err(|e| format!("Failed to create known network proxy: {}", e))
}

/// Helper to create DeviceProxy from path
async fn create_device_proxy(
    conn: &zbus::Connection,
    path: &OwnedObjectPath,
) -> Result<DeviceProxy<'static>, String> {
    DeviceProxy::builder(conn)
        .path(path.clone())
        .map_err(|e| format!("Invalid device path: {}", e))?
        .build()
        .await
        .map_err(|e| format!("Failed to create device proxy: {}", e))
}

/// WiFi backend abstraction over iwd
pub struct WifiBackend {
    conn: zbus::Connection,
    device_path: Option<OwnedObjectPath>,
    evt_tx: Sender<BackendEvent>,
    /// Handle to abort pending connection task
    pending_connect: Arc<Mutex<Option<AbortHandle>>>,
}

impl WifiBackend {
    /// Create a new WifiBackend
    pub async fn new(conn: zbus::Connection, evt_tx: Sender<BackendEvent>) -> Self {
        let device_path = match find_iwd_device_path(&conn).await {
            Ok(path) => {
                tracing::info!("Found iwd device");
                Some(path)
            }
            Err(e) => {
                tracing::warn!("Failed to find iwd device: {}. WiFi features disabled.", e);
                let _ = evt_tx.send(BackendEvent::Error(format!("iwd: {}", e))).await;
                None
            }
        };

        Self {
            conn,
            device_path,
            evt_tx,
            pending_connect: Arc::new(Mutex::new(None)),
        }
    }

    /// Get the device path if available
    pub fn device_path(&self) -> Option<&OwnedObjectPath> {
        self.device_path.as_ref()
    }

    /// Get StationProxy if device is powered and Station interface exists
    async fn station(&self) -> Option<StationProxy<'static>> {
        let path = self.device_path.as_ref()?;
        if !has_station_interface(&self.conn, path).await {
            return None;
        }
        StationProxy::builder(&self.conn)
            .path(path.clone())
            .ok()?
            .build()
            .await
            .ok()
    }

    /// Trigger a WiFi scan
    pub async fn scan(&self) {
        let Some(station) = self.station().await else { return };
        tracing::info!("Starting WiFi scan");
        if let Err(e) = station.scan().await {
            tracing::error!("Scan failed: {}", e);
            let _ = self.evt_tx.send(BackendEvent::Error(format!("Scan: {}", e))).await;
        }
    }

    /// Connect to a WiFi network (spawns a task for passphrase handling)
    pub async fn connect(&self, path: &str) {
        tracing::info!("Connecting to WiFi network: {}", path);

        // Abort any previous pending connection using async lock (no race condition)
        {
            let mut guard = self.pending_connect.lock().await;
            if let Some(prev_handle) = guard.take() {
                tracing::debug!("Aborting previous connection attempt");
                prev_handle.abort();
            }
        }

        let path = path.to_string();
        let conn = self.conn.clone();
        let evt_tx = self.evt_tx.clone();
        let device_path = self.device_path.clone();
        let pending_connect = self.pending_connect.clone();

        // Spawn connect in separate task to not block passphrase handling
        let handle = tokio::spawn(async move {
            let _ = evt_tx.send(BackendEvent::WifiConnecting(path.clone())).await;

            let network = match create_network_proxy(&conn, &path).await {
                Ok(n) => n,
                Err(e) => {
                    tracing::error!("{}", e);
                    let _ = evt_tx.send(BackendEvent::WifiConnected(None)).await;
                    let _ = evt_tx.send(BackendEvent::Error("Invalid network path".into())).await;
                    return;
                }
            };

            // 60 second timeout - enough for password entry, protects against iwd hangs
            const CONNECT_TIMEOUT_SECS: u64 = 60;
            let connect_timeout = std::time::Duration::from_secs(CONNECT_TIMEOUT_SECS);
            match tokio::time::timeout(connect_timeout, network.connect()).await {
                Ok(Ok(())) => {
                    tracing::info!("Connected to {}", path);
                    let _ = evt_tx.send(BackendEvent::WifiConnected(Some(path.clone()))).await;
                    let _ = evt_tx.send(BackendEvent::WifiNetworkKnown { path }).await;
                }
                Ok(Err(e)) => {
                    tracing::error!("Connect failed: {}", e);
                    // Query actual state from iwd to preserve existing connection
                    let actual_connected = Self::get_connected_network_static(&conn, device_path.as_ref()).await;
                    let _ = evt_tx.send(BackendEvent::WifiConnected(actual_connected)).await;
                    let _ = evt_tx.send(BackendEvent::Error(format_iwd_error(&e))).await;
                }
                Err(_) => {
                    tracing::error!("Connect timed out for {}", path);
                    let actual_connected = Self::get_connected_network_static(&conn, device_path.as_ref()).await;
                    let _ = evt_tx.send(BackendEvent::WifiConnected(actual_connected)).await;
                    let _ = evt_tx.send(BackendEvent::Error("Connection timed out".into())).await;
                }
            }

            // Clear pending handle when done
            let mut guard = pending_connect.lock().await;
            *guard = None;
        });

        // Store abort handle for this connection task
        let mut guard = self.pending_connect.lock().await;
        *guard = Some(handle.abort_handle());
    }

    /// Helper for connect task - get connected network without &self
    async fn get_connected_network_static(
        conn: &zbus::Connection,
        device_path: Option<&OwnedObjectPath>,
    ) -> Option<String> {
        let path = device_path?;
        if !has_station_interface(conn, path).await {
            return None;
        }
        let station = StationProxy::builder(conn)
            .path(path.clone())
            .ok()?
            .build()
            .await
            .ok()?;
        station.connected_network().await.ok().map(|p| p.to_string())
    }

    /// Disconnect from current WiFi network
    pub async fn disconnect(&self) {
        let Some(station) = self.station().await else { return };
        tracing::info!("Disconnecting from WiFi");
        match station.disconnect().await {
            Ok(()) => {
                let _ = self.evt_tx.send(BackendEvent::WifiConnected(None)).await;
            }
            Err(e) => {
                tracing::error!("Disconnect failed: {}", e);
                let _ = self.evt_tx.send(BackendEvent::Error(format!("Disconnect: {}", e))).await;
            }
        }
    }

    /// Forget a known network
    pub async fn forget(&self, network_path: &str) {
        tracing::info!("Forgetting network: {}", network_path);

        let network = match create_network_proxy(&self.conn, network_path).await {
            Ok(n) => n,
            Err(e) => {
                tracing::error!("{}", e);
                let _ = self.evt_tx.send(BackendEvent::Error("Invalid network path".into())).await;
                return;
            }
        };

        let known_path = match network.known_network().await {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("Network is not known: {}", e);
                let _ = self.evt_tx.send(BackendEvent::Error("Network is not saved".into())).await;
                return;
            }
        };

        let known = match create_known_network_proxy(&self.conn, known_path).await {
            Ok(k) => k,
            Err(e) => {
                tracing::error!("{}", e);
                let _ = self.evt_tx.send(BackendEvent::Error("Failed to forget network".into())).await;
                return;
            }
        };

        match known.forget().await {
            Ok(()) => {
                tracing::info!("Forgot network: {}", network_path);
                self.send_networks().await;
            }
            Err(e) => {
                tracing::error!("Forget failed: {}", e);
                let _ = self.evt_tx.send(BackendEvent::Error(format!("Forget: {}", e))).await;
            }
        }
    }

    /// Set WiFi adapter power state
    pub async fn set_powered(&self, powered: bool) {
        let Some(ref path) = self.device_path else { return };

        let device = match create_device_proxy(&self.conn, path).await {
            Ok(d) => d,
            Err(e) => {
                tracing::error!("{}", e);
                return;
            }
        };

        tracing::info!("Setting WiFi powered: {}", powered);
        if let Err(e) = device.set_powered(powered).await {
            tracing::error!("Set powered failed: {}", e);
            let _ = self.evt_tx.send(BackendEvent::Error(format!("Power: {}", e))).await;
        }
    }

    /// Send current network list to UI
    pub async fn send_networks(&self) {
        let Some(station) = self.station().await else { return };
        if let Ok(networks) = get_wifi_networks(&self.conn, &station).await {
            let _ = self.evt_tx.send(BackendEvent::WifiNetworks(networks)).await;
        }
    }

    /// Send current connected status to UI
    pub async fn send_connected_status(&self) {
        let Some(station) = self.station().await else { return };
        let connected = station.connected_network().await.ok().map(|p| p.to_string());
        let _ = self.evt_tx.send(BackendEvent::WifiConnected(connected)).await;
    }

    /// Cancel any pending connection and cleanup
    ///
    /// Uses try_lock() because this is called from Drop (can't be async).
    /// If lock is held, the connection task will be aborted when tokio runtime shuts down anyway.
    pub fn shutdown(&self) {
        if let Ok(mut guard) = self.pending_connect.try_lock() {
            if let Some(handle) = guard.take() {
                tracing::debug!("Aborting pending connection on shutdown");
                handle.abort();
            }
        }
    }
}

impl Drop for WifiBackend {
    fn drop(&mut self) {
        self.shutdown();
    }
}
