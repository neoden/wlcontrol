mod helpers;
mod state;
mod streams;

use std::collections::HashSet;

use async_channel::{Receiver, Sender};
use bluer::{AdapterEvent, Address, DeviceProperty};
use futures::stream::SelectAll;

use super::bluetooth::backend::{
    BtAdapterEventStream, BtDeviceEventStream, BtDiscoveryStream, BtPairingRequest,
};
use super::bluetooth::BluetoothBackend;
use super::manager::{BackendCommand, BackendEvent};
use super::wifi::iwd_proxy::{AgentManagerProxy, StationProxy};
use super::wifi::{IwdAgent, PassphraseRequest};
use super::wifi_backend::{find_all_iwd_devices, WifiBackend};

pub use state::{BackendState, LoopAction};
pub use streams::EventStreams;

use helpers::{create_device_proxy, send_wifi_initial_state, setup_station_streams};

pub enum LoopEvent {
    WifiPoweredChanged(bool),
    WifiScanningChanged(bool),
    WifiStationStateChanged(String),
    PassphraseRequest(PassphraseRequest),
    IwdDeviceAdded { object_path: String },
    IwdDeviceRemoved { object_path: String },
    BtDiscoveryEvent(AdapterEvent),
    BtAdapterEvent(AdapterEvent),
    BtDevicePropertyChanged {
        address: Address,
        property: DeviceProperty,
    },
    BtPairingRequest(BtPairingRequest),
    BtScanTimeout,
    Command(BackendCommand),
    CommandChannelClosed,
}

/// Initialize the backend: D-Bus, WiFi, Bluetooth, all streams.
/// Returns (BackendState, EventStreams) ready for the event loop.
pub async fn init(
    cmd_rx: Receiver<BackendCommand>,
    evt_tx: Sender<BackendEvent>,
) -> Result<(BackendState, EventStreams), Box<dyn std::error::Error + Send + Sync>> {
    tracing::info!("Starting backend...");

    // Connect to system D-Bus
    let conn = zbus::Connection::system().await?;
    tracing::info!("Connected to system D-Bus");

    // Create channel for passphrase requests from agent
    let (passphrase_tx, passphrase_rx) = async_channel::unbounded::<PassphraseRequest>();

    // Create and register iwd agent
    let agent = IwdAgent::new(passphrase_tx);
    let agent_path = "/dev/neoden/wlcontrol/Agent";
    conn.object_server().at(agent_path, agent).await?;
    tracing::info!("Registered iwd agent at {}", agent_path);

    // Discover all WiFi devices
    let wifi_device_infos = match find_all_iwd_devices(&conn).await {
        Ok(infos) => infos,
        Err(e) => {
            tracing::warn!("Failed to enumerate iwd devices: {}", e);
            Vec::new()
        }
    };

    // Pick initial adapter: prefer one that's already connected, fall back to first
    let initial_device = {
        let mut connected_device = None;
        for info in &wifi_device_infos {
            let path: zbus::zvariant::OwnedObjectPath =
                info.device_path.as_str().try_into().unwrap();
            if let Ok(station) = StationProxy::builder(&conn).path(path).unwrap().build().await {
                if station.connected_network().await.is_ok() {
                    connected_device = Some(info);
                    break;
                }
            }
        }
        connected_device.or(wifi_device_infos.first())
    };

    let _ = evt_tx
        .send(BackendEvent::WifiDevices {
            devices: wifi_device_infos.clone(),
            active_path: initial_device.map(|d| d.device_path.clone()),
        })
        .await;

    let wifi: Option<WifiBackend> = initial_device.map(|info| {
        tracing::info!(
            "Selected initial WiFi adapter: {} ({})",
            info.device_name,
            info.device_path
        );
        let path: zbus::zvariant::OwnedObjectPath =
            info.device_path.as_str().try_into().unwrap();
        WifiBackend::new(conn.clone(), evt_tx.clone(), path)
    });

    // Register agent with iwd (agent is global, handles all devices)
    if !wifi_device_infos.is_empty() {
        match AgentManagerProxy::new(&conn).await {
            Ok(agent_manager) => {
                match agent_manager
                    .register_agent(agent_path.try_into().unwrap())
                    .await
                {
                    Ok(()) => tracing::info!("Registered agent with iwd"),
                    Err(e) => {
                        tracing::warn!("Failed to register agent with iwd: {}", e);
                        let _ = evt_tx
                            .send(BackendEvent::WifiError(
                                "Cannot register password agent. Connecting to secured networks may fail.".into(),
                            ))
                            .await;
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to connect to iwd AgentManager: {}", e);
                let _ = evt_tx
                    .send(BackendEvent::WifiError(
                        "Cannot connect to iwd AgentManager. Connecting to secured networks may fail.".into(),
                    ))
                    .await;
            }
        }
    }

    // Send initial state for active WiFi device
    if let Some(ref w) = wifi {
        if let Some(path) = w.device_path() {
            send_wifi_initial_state(&conn, path, &evt_tx).await;
        }
    }

    // Initialize Bluetooth backend
    let (bt, bt_pairing_rx): (
        Option<BluetoothBackend>,
        Option<async_channel::Receiver<BtPairingRequest>>,
    ) = match BluetoothBackend::new(evt_tx.clone()).await {
        Ok((bt, rx)) => (Some(bt), Some(rx)),
        Err(e) => {
            tracing::warn!(
                "Failed to initialize Bluetooth backend: {}. BT disabled.",
                e
            );
            let _ = evt_tx
                .send(BackendEvent::BtError(format!("Bluetooth: {}", e)))
                .await;
            (None, None)
        }
    };

    // BT streams
    let bt_discovery_stream: Option<BtDiscoveryStream> = None;
    let mut bt_adapter_events: Option<BtAdapterEventStream> = None;
    let mut bt_device_events: SelectAll<BtDeviceEventStream> = SelectAll::new();
    let mut bt_tracked_devices: HashSet<bluer::Address> = HashSet::new();

    // Send initial BT state and start always-on adapter event stream
    if let Some(ref bt_backend) = bt {
        bt_backend
            .send_initial_state(&mut bt_device_events, &mut bt_tracked_devices)
            .await;
        bt_adapter_events = bt_backend.adapter_events().await;
    }

    // Set up property change streams for Device
    let device_powered_stream = match wifi.as_ref().and_then(|w| w.device_path()) {
        Some(path) => {
            if let Some(device) = create_device_proxy(&conn, path).await {
                Some(device.receive_powered_changed().await)
            } else {
                None
            }
        }
        None => None,
    };

    // Set up property change streams for Station (only if powered)
    let mut station_scanning_stream: Option<zbus::PropertyStream<'static, bool>> = None;
    let mut station_state_stream: Option<zbus::PropertyStream<'static, String>> = None;

    // Initialize station streams if already powered
    if let Some(ref w) = wifi {
        if let Some(path) = w.device_path() {
            let (scanning, state) = setup_station_streams(&conn, path).await;
            station_scanning_stream = scanning;
            station_state_stream = state;
        }
    }

    // Subscribe to iwd ObjectManager for hot-plug (adapter add/remove)
    let iwd_obj_manager = zbus::fdo::ObjectManagerProxy::builder(&conn)
        .destination("net.connman.iwd")
        .ok()
        .and_then(|b| b.path("/").ok());
    let mut iwd_interfaces_added = None;
    let mut iwd_interfaces_removed = None;
    if let Some(builder) = iwd_obj_manager {
        if let Ok(proxy) = builder.build().await {
            iwd_interfaces_added = proxy.receive_interfaces_added().await.ok();
            iwd_interfaces_removed = proxy.receive_interfaces_removed().await.ok();
        }
    }

    let state = BackendState {
        conn,
        evt_tx,
        wifi,
        bt,
        bt_tracked_devices,
        wifi_device_infos,
        pending_passphrase_response: None,
        pending_pairing_response: None,
        pending_pin_response: None,
        pending_passkey_response: None,
    };

    let streams = EventStreams {
        cmd_rx,
        passphrase_rx,
        bt_pairing_rx,
        device_powered_stream,
        station_scanning_stream,
        station_state_stream,
        bt_discovery_stream,
        bt_adapter_events,
        bt_device_events,
        bt_scan_deadline: None,
        iwd_interfaces_added,
        iwd_interfaces_removed,
    };

    Ok((state, streams))
}
