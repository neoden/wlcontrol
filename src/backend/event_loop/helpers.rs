use async_channel::Sender;
use futures::StreamExt;
use zbus::zvariant::OwnedObjectPath;

use super::super::types::BackendEvent;
use super::super::wifi::iwd_proxy::{DeviceProxy, StationProxy};
use super::super::wifi::{get_known_networks, get_wifi_networks};

/// Create DeviceProxy safely, returning None on failure.
pub async fn create_device_proxy(
    conn: &zbus::Connection,
    path: &OwnedObjectPath,
) -> Option<DeviceProxy<'static>> {
    DeviceProxy::builder(conn)
        .path(path.clone())
        .ok()?
        .build()
        .await
        .ok()
}

/// Create StationProxy safely, returning None if Station interface is unavailable.
pub async fn create_station_proxy(
    conn: &zbus::Connection,
    path: &OwnedObjectPath,
) -> Option<StationProxy<'static>> {
    StationProxy::builder(conn)
        .path(path.clone())
        .ok()?
        .build()
        .await
        .ok()
}

/// Send initial WiFi state for a device (powered, scanning, networks, known networks).
pub async fn send_wifi_initial_state(
    conn: &zbus::Connection,
    device_path: &OwnedObjectPath,
    evt_tx: &Sender<BackendEvent>,
) {
    if let Some(device) = create_device_proxy(conn, device_path).await {
        if let Ok(powered) = device.powered().await {
            let _ = evt_tx.send(BackendEvent::WifiPowered(powered)).await;

            if powered {
                if let Some(station) = create_station_proxy(conn, device_path).await {
                    if let Ok(scanning) = station.scanning().await {
                        let _ = evt_tx.send(BackendEvent::WifiScanning(scanning)).await;
                    }
                    if let Ok(networks) = get_wifi_networks(conn, &station).await {
                        let _ = evt_tx.send(BackendEvent::WifiNetworks(networks)).await;
                    }
                }
            }
            if let Ok(known) = get_known_networks(conn).await {
                let _ = evt_tx.send(BackendEvent::WifiKnownNetworks(known)).await;
            }
        }
    }
}

/// Wait for Station interface with exponential backoff retry.
pub async fn wait_for_station_proxy(
    conn: &zbus::Connection,
    device_path: &OwnedObjectPath,
    max_attempts: u32,
) -> Option<StationProxy<'static>> {
    for attempt in 0..max_attempts {
        if let Some(station) = create_station_proxy(conn, device_path).await {
            tracing::debug!("Station interface available after {} attempts", attempt + 1);
            return Some(station);
        }
        if attempt + 1 < max_attempts {
            let delay = 50 * (1 << attempt.min(4));
            tokio::time::sleep(tokio::time::Duration::from_millis(delay)).await;
        }
    }
    tracing::warn!(
        "Station interface not available after {} attempts",
        max_attempts
    );
    None
}

/// Set up Station property change streams (scanning + state).
pub async fn setup_station_streams(
    conn: &zbus::Connection,
    device_path: &OwnedObjectPath,
) -> (
    Option<zbus::PropertyStream<'static, bool>>,
    Option<zbus::PropertyStream<'static, String>>,
) {
    let Some(station) = create_station_proxy(conn, device_path).await else {
        return (None, None);
    };
    let scanning = station.receive_scanning_changed().await;
    let state = station.receive_state_changed().await;
    (Some(scanning), Some(state))
}

/// Set up Station streams with retry (waits for Station interface to appear).
pub async fn setup_station_streams_with_retry(
    conn: &zbus::Connection,
    device_path: &OwnedObjectPath,
) -> (
    Option<zbus::PropertyStream<'static, bool>>,
    Option<zbus::PropertyStream<'static, String>>,
) {
    let Some(station) = wait_for_station_proxy(conn, device_path, 10).await else {
        return (None, None);
    };
    let scanning = station.receive_scanning_changed().await;
    let state = station.receive_state_changed().await;
    (Some(scanning), Some(state))
}

/// Poll next event from iwd InterfacesAdded stream (or pend forever if None).
pub async fn next_iwd_added(
    stream: &mut Option<zbus::fdo::InterfacesAddedStream<'static>>,
) -> Option<zbus::fdo::InterfacesAdded> {
    match stream.as_mut() {
        Some(s) => StreamExt::next(s).await,
        None => std::future::pending().await,
    }
}

/// Poll next event from iwd InterfacesRemoved stream (or pend forever if None).
pub async fn next_iwd_removed(
    stream: &mut Option<zbus::fdo::InterfacesRemovedStream<'static>>,
) -> Option<zbus::fdo::InterfacesRemoved> {
    match stream.as_mut() {
        Some(s) => StreamExt::next(s).await,
        None => std::future::pending().await,
    }
}
