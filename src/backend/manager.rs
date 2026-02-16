use adw::prelude::*;
use adw::subclass::prelude::*;
use async_channel::{Receiver, Sender};
use futures::StreamExt;
use gtk::{gio, glib};
use std::sync::OnceLock;
use tokio::runtime::Runtime;

use super::bluetooth::BtDevice;
use super::wifi::WifiNetwork;

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime")
    })
}

/// Commands sent from UI to backend
#[derive(Debug, Clone)]
pub enum BackendCommand {
    /// Shutdown the backend gracefully
    Shutdown,
    WifiScan,
    WifiConnect { path: String },
    WifiDisconnect,
    WifiForget { path: String }, // network path, backend will get known_network from it
    WifiSetPowered(bool),
    /// Response to a passphrase request (None = cancelled)
    PassphraseResponse { passphrase: Option<String> },
    BtScan,
    BtStopScan,
    BtConnect { path: String },
    BtDisconnect { path: String },
    BtPair { path: String },
    BtRemove { path: String },
    BtSetPowered(bool),
    BtSetDiscoverable(bool),
}

/// Data for a WiFi network, used to transfer between backend and UI threads
#[derive(Debug, Clone)]
pub struct WifiNetworkData {
    pub path: String,
    pub name: String,
    pub network_type: String,
    pub signal_strength: i16,
    pub connected: bool,
    pub known: bool,
}

/// Events sent from backend to UI
#[derive(Debug, Clone)]
pub enum BackendEvent {
    WifiPowered(bool),
    WifiScanning(bool),
    WifiNetworks(Vec<WifiNetworkData>),
    WifiConnected(Option<String>),    // path of connected network, or None
    WifiConnecting(String),           // path of network we're connecting to
    WifiNetworkKnown { path: String }, // network became known (saved)
    /// iwd is requesting a passphrase for a network
    PassphraseRequest {
        network_path: String,
        network_name: String,
    },
    /// Captive portal detected after connection, URL to open in browser
    CaptivePortal { url: String },
    BtPowered(bool),
    BtDiscovering(bool),
    BtDiscoverable(bool),
    Error(String),
}

mod imp {
    use super::{BackendCommand, BtDevice, Sender, WifiNetwork};
    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::{gio, glib};
    use std::cell::RefCell;
    use std::sync::OnceLock;

    pub struct WlcontrolManager {
        pub wifi_networks: gio::ListStore,
        pub bt_devices: gio::ListStore,
        pub wifi_powered: RefCell<bool>,
        pub wifi_scanning: RefCell<bool>,
        pub bt_powered: RefCell<bool>,
        pub bt_discovering: RefCell<bool>,
        pub bt_discoverable: RefCell<bool>,
        pub cmd_tx: OnceLock<Sender<BackendCommand>>,
    }

    impl Default for WlcontrolManager {
        fn default() -> Self {
            Self {
                wifi_networks: gio::ListStore::new::<WifiNetwork>(),
                bt_devices: gio::ListStore::new::<BtDevice>(),
                wifi_powered: RefCell::new(false),
                wifi_scanning: RefCell::new(false),
                bt_powered: RefCell::new(false),
                bt_discovering: RefCell::new(false),
                bt_discoverable: RefCell::new(false),
                cmd_tx: OnceLock::new(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WlcontrolManager {
        const NAME: &'static str = "WlcontrolManager";
        type Type = super::WlcontrolManager;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for WlcontrolManager {
        fn dispose(&self) {
            tracing::debug!("WlcontrolManager disposing, sending shutdown");
            // Send shutdown command to backend
            if let Some(tx) = self.cmd_tx.get() {
                let _ = tx.try_send(BackendCommand::Shutdown);
            }
        }

        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: OnceLock<Vec<glib::ParamSpec>> = OnceLock::new();
            PROPERTIES.get_or_init(|| {
                vec![
                    glib::ParamSpecBoolean::builder("wifi-powered").build(),
                    glib::ParamSpecBoolean::builder("wifi-scanning")
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("bt-powered").build(),
                    glib::ParamSpecBoolean::builder("bt-discovering")
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("bt-discoverable").build(),
                ]
            })
        }

        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    glib::subclass::Signal::builder("passphrase-requested")
                        .param_types([String::static_type(), String::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("captive-portal")
                        .param_types([String::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("error")
                        .param_types([String::static_type()])
                        .build(),
                ]
            })
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "wifi-powered" => self.wifi_powered.borrow().to_value(),
                "wifi-scanning" => self.wifi_scanning.borrow().to_value(),
                "bt-powered" => self.bt_powered.borrow().to_value(),
                "bt-discovering" => self.bt_discovering.borrow().to_value(),
                "bt-discoverable" => self.bt_discoverable.borrow().to_value(),
                _ => unimplemented!(),
            }
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            match pspec.name() {
                "wifi-powered" => {
                    let powered = value.get().unwrap();
                    self.wifi_powered.replace(powered);
                    // Send command to backend
                    if let Some(tx) = self.cmd_tx.get() {
                        let tx = tx.clone();
                        glib::spawn_future_local(async move {
                            let _ = tx.send(BackendCommand::WifiSetPowered(powered)).await;
                        });
                    }
                }
                "bt-powered" => {
                    let powered = value.get().unwrap();
                    self.bt_powered.replace(powered);
                    if let Some(tx) = self.cmd_tx.get() {
                        let tx = tx.clone();
                        glib::spawn_future_local(async move {
                            let _ = tx.send(BackendCommand::BtSetPowered(powered)).await;
                        });
                    }
                }
                "bt-discoverable" => {
                    let discoverable = value.get().unwrap();
                    self.bt_discoverable.replace(discoverable);
                    if let Some(tx) = self.cmd_tx.get() {
                        let tx = tx.clone();
                        glib::spawn_future_local(async move {
                            let _ = tx.send(BackendCommand::BtSetDiscoverable(discoverable)).await;
                        });
                    }
                }
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    pub struct WlcontrolManager(ObjectSubclass<imp::WlcontrolManager>);
}

impl Default for WlcontrolManager {
    fn default() -> Self {
        Self::new()
    }
}

impl WlcontrolManager {
    pub fn new() -> Self {
        glib::Object::new()
    }

    pub fn start(&self) {
        let (cmd_tx, cmd_rx) = async_channel::bounded::<BackendCommand>(32);
        let (evt_tx, evt_rx) = async_channel::bounded::<BackendEvent>(32);

        self.imp().cmd_tx.set(cmd_tx).unwrap();

        // Spawn backend task
        runtime().spawn(async move {
            if let Err(e) = run_backend(cmd_rx, evt_tx).await {
                tracing::error!("Backend error: {}", e);
            }
        });

        // Handle events on GTK main thread
        let manager = self.clone();
        glib::spawn_future_local(async move {
            while let Ok(event) = evt_rx.recv().await {
                manager.handle_event(event);
            }
        });
    }

    fn handle_event(&self, event: BackendEvent) {
        match event {
            BackendEvent::WifiPowered(powered) => self.set_wifi_powered(powered),
            BackendEvent::WifiScanning(scanning) => self.set_wifi_scanning(scanning),
            BackendEvent::WifiNetworks(networks) => self.update_wifi_networks(networks),
            BackendEvent::WifiConnected(path) => {
                self.clear_wifi_connecting();
                self.update_wifi_connected(path);
            }
            BackendEvent::WifiConnecting(path) => self.set_wifi_connecting(&path),
            BackendEvent::WifiNetworkKnown { path } => self.set_wifi_network_known(&path),
            BackendEvent::PassphraseRequest {
                network_path,
                network_name,
            } => {
                tracing::info!(
                    "Passphrase requested for {} ({})",
                    network_name,
                    network_path
                );
                self.emit_by_name::<()>("passphrase-requested", &[&network_path, &network_name]);
            }
            BackendEvent::CaptivePortal { url } => {
                tracing::info!("Captive portal detected: {}", url);
                self.emit_by_name::<()>("captive-portal", &[&url]);
            }
            BackendEvent::BtPowered(powered) => self.set_bt_powered(powered),
            BackendEvent::BtDiscovering(discovering) => self.set_bt_discovering(discovering),
            BackendEvent::BtDiscoverable(discoverable) => self.set_bt_discoverable(discoverable),
            BackendEvent::Error(msg) => {
                tracing::error!("Backend error: {}", msg);
                self.emit_by_name::<()>("error", &[&msg]);
            }
        }
    }

    fn update_wifi_networks(&self, networks: Vec<WifiNetworkData>) {
        let store = &self.imp().wifi_networks;
        store.remove_all();
        for data in networks {
            let network = WifiNetwork::new(
                &data.path,
                &data.name,
                &data.network_type,
                data.signal_strength,
                data.connected,
                data.known,
            );
            store.append(&network);
        }
    }

    fn update_wifi_connected(&self, connected_path: Option<String>) {
        let store = &self.imp().wifi_networks;
        for i in 0..store.n_items() {
            if let Some(obj) = store.item(i) {
                let network = obj.downcast_ref::<WifiNetwork>().unwrap();
                let is_connected = connected_path
                    .as_ref()
                    .map(|p| p == &network.path())
                    .unwrap_or(false);
                network.set_connected(is_connected);
            }
        }
    }

    fn set_wifi_network_known(&self, path: &str) {
        let store = &self.imp().wifi_networks;
        for i in 0..store.n_items() {
            if let Some(obj) = store.item(i) {
                let network = obj.downcast_ref::<WifiNetwork>().unwrap();
                if network.path() == path {
                    network.set_known(true);
                    break;
                }
            }
        }
    }

    fn set_wifi_connecting(&self, path: &str) {
        let store = &self.imp().wifi_networks;
        for i in 0..store.n_items() {
            if let Some(obj) = store.item(i) {
                let network = obj.downcast_ref::<WifiNetwork>().unwrap();
                network.set_connecting(network.path() == path);
            }
        }
    }

    fn clear_wifi_connecting(&self) {
        let store = &self.imp().wifi_networks;
        for i in 0..store.n_items() {
            if let Some(obj) = store.item(i) {
                let network = obj.downcast_ref::<WifiNetwork>().unwrap();
                network.set_connecting(false);
            }
        }
    }

    fn send_command(&self, cmd: BackendCommand) {
        if let Some(tx) = self.imp().cmd_tx.get() {
            let tx = tx.clone();
            glib::spawn_future_local(async move {
                if let Err(e) = tx.send(cmd).await {
                    tracing::error!("Failed to send command: {}", e);
                }
            });
        }
    }

    pub fn wifi_networks(&self) -> gio::ListStore {
        self.imp().wifi_networks.clone()
    }

    pub fn bt_devices(&self) -> gio::ListStore {
        self.imp().bt_devices.clone()
    }

    pub fn wifi_powered(&self) -> bool {
        *self.imp().wifi_powered.borrow()
    }

    pub fn set_wifi_powered(&self, powered: bool) {
        if *self.imp().wifi_powered.borrow() != powered {
            self.imp().wifi_powered.replace(powered);
            self.notify("wifi-powered");
        }
    }

    pub fn wifi_scanning(&self) -> bool {
        *self.imp().wifi_scanning.borrow()
    }

    pub fn set_wifi_scanning(&self, scanning: bool) {
        if *self.imp().wifi_scanning.borrow() != scanning {
            self.imp().wifi_scanning.replace(scanning);
            self.notify("wifi-scanning");
        }
    }

    pub fn bt_powered(&self) -> bool {
        *self.imp().bt_powered.borrow()
    }

    pub fn set_bt_powered(&self, powered: bool) {
        if *self.imp().bt_powered.borrow() != powered {
            self.imp().bt_powered.replace(powered);
            self.notify("bt-powered");
        }
    }

    pub fn bt_discovering(&self) -> bool {
        *self.imp().bt_discovering.borrow()
    }

    pub fn set_bt_discovering(&self, discovering: bool) {
        if *self.imp().bt_discovering.borrow() != discovering {
            self.imp().bt_discovering.replace(discovering);
            self.notify("bt-discovering");
        }
    }

    pub fn bt_discoverable(&self) -> bool {
        *self.imp().bt_discoverable.borrow()
    }

    pub fn set_bt_discoverable(&self, discoverable: bool) {
        if *self.imp().bt_discoverable.borrow() != discoverable {
            self.imp().bt_discoverable.replace(discoverable);
            self.notify("bt-discoverable");
        }
    }

    pub fn request_wifi_scan(&self) {
        self.send_command(BackendCommand::WifiScan);
    }

    pub fn request_wifi_connect(&self, path: &str) {
        self.send_command(BackendCommand::WifiConnect {
            path: path.to_string(),
        });
    }

    pub fn request_wifi_disconnect(&self) {
        self.send_command(BackendCommand::WifiDisconnect);
    }

    pub fn request_wifi_forget(&self, path: &str) {
        self.send_command(BackendCommand::WifiForget {
            path: path.to_string(),
        });
    }

    pub fn send_passphrase_response(&self, passphrase: Option<String>) {
        self.send_command(BackendCommand::PassphraseResponse { passphrase });
    }

    pub fn request_bt_scan(&self) {
        self.send_command(BackendCommand::BtScan);
    }

    pub fn request_bt_connect(&self, path: &str) {
        self.send_command(BackendCommand::BtConnect {
            path: path.to_string(),
        });
    }

    pub fn request_bt_disconnect(&self, path: &str) {
        self.send_command(BackendCommand::BtDisconnect {
            path: path.to_string(),
        });
    }

    pub fn request_bt_pair(&self, path: &str) {
        self.send_command(BackendCommand::BtPair {
            path: path.to_string(),
        });
    }

    pub fn request_bt_remove(&self, path: &str) {
        self.send_command(BackendCommand::BtRemove {
            path: path.to_string(),
        });
    }

    /// Shutdown the backend gracefully
    pub fn shutdown(&self) {
        tracing::info!("Requesting backend shutdown");
        if let Some(tx) = self.imp().cmd_tx.get() {
            // Use try_send to avoid blocking - if channel is full, backend will
            // shutdown anyway when the channel is dropped
            let _ = tx.try_send(BackendCommand::Shutdown);
        }
    }
}

use super::wifi::iwd_proxy::{AgentManagerProxy, DeviceProxy, StationProxy};
use super::wifi::{IwdAgent, PassphraseRequest};
use super::wifi_backend::{get_wifi_networks, has_station_interface, WifiBackend};

async fn run_backend(
    cmd_rx: Receiver<BackendCommand>,
    evt_tx: Sender<BackendEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tokio::sync::oneshot;

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

    // Create WiFi backend
    let wifi = WifiBackend::new(conn.clone(), evt_tx.clone()).await;

    // Register agent with iwd
    if wifi.device_path().is_some() {
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
                            .send(BackendEvent::Error(
                                "Cannot register password agent. Connecting to secured networks may fail.".into(),
                            ))
                            .await;
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to connect to iwd AgentManager: {}", e);
                let _ = evt_tx
                    .send(BackendEvent::Error(
                        "Cannot connect to iwd AgentManager. Connecting to secured networks may fail.".into(),
                    ))
                    .await;
            }
        }
    }

    // Helper to create DeviceProxy safely
    async fn create_device_proxy(
        conn: &zbus::Connection,
        path: &zbus::zvariant::OwnedObjectPath,
    ) -> Option<DeviceProxy<'static>> {
        DeviceProxy::builder(conn)
            .path(path.clone())
            .ok()?
            .build()
            .await
            .ok()
    }

    // Helper to create StationProxy safely
    async fn create_station_proxy(
        conn: &zbus::Connection,
        path: &zbus::zvariant::OwnedObjectPath,
    ) -> Option<StationProxy<'static>> {
        if !has_station_interface(conn, path).await {
            return None;
        }
        StationProxy::builder(conn)
            .path(path.clone())
            .ok()?
            .build()
            .await
            .ok()
    }

    // Send initial state
    if let Some(path) = wifi.device_path() {
        if let Some(device) = create_device_proxy(&conn, path).await {
            if let Ok(powered) = device.powered().await {
                let _ = evt_tx.send(BackendEvent::WifiPowered(powered)).await;

                // If powered, get station info
                if powered {
                    if let Some(station) = create_station_proxy(&conn, path).await {
                        if let Ok(scanning) = station.scanning().await {
                            let _ = evt_tx.send(BackendEvent::WifiScanning(scanning)).await;
                        }
                        if let Ok(networks) = get_wifi_networks(&conn, &station).await {
                            let _ = evt_tx.send(BackendEvent::WifiNetworks(networks)).await;
                        }
                    }
                }
            }
        }
    }

    // TODO: Initialize BlueZ

    // Store pending passphrase response sender
    let mut pending_passphrase_response: Option<oneshot::Sender<Option<String>>> = None;

    // Set up property change streams for Device
    let mut device_powered_stream = if let Some(path) = wifi.device_path() {
        if let Some(device) = create_device_proxy(&conn, path).await {
            Some(device.receive_powered_changed().await)
        } else {
            None
        }
    } else {
        None
    };

    // Set up property change streams for Station (only if powered)
    let mut station_scanning_stream: Option<zbus::PropertyStream<'_, bool>> = None;
    let mut station_state_stream: Option<zbus::PropertyStream<'_, String>> = None;

    // Helper to wait for Station interface with retry
    async fn wait_for_station_proxy(
        conn: &zbus::Connection,
        device_path: &zbus::zvariant::OwnedObjectPath,
        max_attempts: u32,
    ) -> Option<StationProxy<'static>> {
        for attempt in 0..max_attempts {
            if let Some(station) = create_station_proxy(conn, device_path).await {
                tracing::debug!("Station interface available after {} attempts", attempt + 1);
                return Some(station);
            }
            if attempt + 1 < max_attempts {
                // Exponential backoff: 50ms, 100ms, 200ms, 400ms, 800ms
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

    // Helper to refresh station streams
    async fn setup_station_streams(
        conn: &zbus::Connection,
        device_path: &zbus::zvariant::OwnedObjectPath,
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

    // Helper to setup station streams with retry
    async fn setup_station_streams_with_retry(
        conn: &zbus::Connection,
        device_path: &zbus::zvariant::OwnedObjectPath,
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

    // Initialize station streams if already powered
    if let Some(path) = wifi.device_path() {
        if has_station_interface(&conn, path).await {
            let (scanning, state) = setup_station_streams(&conn, path).await;
            station_scanning_stream = scanning;
            station_state_stream = state;
        }
    }

    loop {
        tokio::select! {
            // Handle Device.powered changes
            Some(change) = async {
                match device_powered_stream.as_mut() {
                    Some(s) => s.next().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Ok(powered) = change.get().await {
                    tracing::info!("Device powered changed: {}", powered);
                    let _ = evt_tx.send(BackendEvent::WifiPowered(powered)).await;

                    if let Some(path) = wifi.device_path() {
                        if powered {
                            // Wait for Station interface to appear with retry
                            let (scanning, state) = setup_station_streams_with_retry(&conn, path).await;
                            station_scanning_stream = scanning;
                            station_state_stream = state;
                            // Get initial network list
                            wifi.send_networks().await;
                        } else {
                            station_scanning_stream = None;
                            station_state_stream = None;
                            let _ = evt_tx.send(BackendEvent::WifiNetworks(vec![])).await;
                        }
                    }
                }
            }
            // Handle Station.scanning changes
            Some(change) = async {
                match station_scanning_stream.as_mut() {
                    Some(s) => s.next().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Ok(scanning) = change.get().await {
                    tracing::debug!("Station scanning changed: {}", scanning);
                    let _ = evt_tx.send(BackendEvent::WifiScanning(scanning)).await;
                    // When scan completes, refresh network list
                    if !scanning {
                        wifi.send_networks().await;
                    }
                }
            }
            // Handle Station.state changes (connected/disconnected/etc)
            Some(change) = async {
                match station_state_stream.as_mut() {
                    Some(s) => s.next().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Ok(state) = change.get().await {
                    tracing::info!("Station state changed: {}", state);
                    wifi.send_connected_status().await;
                }
            }
            // Handle passphrase requests from agent
            Ok(request) = passphrase_rx.recv() => {
                tracing::info!("Passphrase request: {} ({})", request.network_name, request.network_path);
                pending_passphrase_response = Some(request.response_tx);
                let _ = evt_tx.send(BackendEvent::PassphraseRequest {
                    network_path: request.network_path,
                    network_name: request.network_name,
                }).await;
            }
            // Handle commands from UI
            result = cmd_rx.recv() => {
                let cmd = match result {
                    Ok(cmd) => cmd,
                    Err(_) => break, // Channel closed
                };
                tracing::debug!("Received command: {:?}", cmd);
                match cmd {
                    BackendCommand::Shutdown => {
                        tracing::info!("Backend shutdown requested");
                        wifi.shutdown();
                        break;
                    }
                    BackendCommand::PassphraseResponse { passphrase } => {
                        if let Some(tx) = pending_passphrase_response.take() {
                            let _ = tx.send(passphrase);
                        }
                    }
                    BackendCommand::WifiScan => wifi.scan().await,
                    BackendCommand::WifiConnect { path } => wifi.connect(&path).await,
                    BackendCommand::WifiDisconnect => wifi.disconnect().await,
                    BackendCommand::WifiForget { path } => wifi.forget(&path).await,
                    BackendCommand::WifiSetPowered(powered) => wifi.set_powered(powered).await,
                    BackendCommand::BtScan => {
                        tracing::info!("Bluetooth scan requested (stub)");
                        let _ = evt_tx.send(BackendEvent::BtDiscovering(true)).await;
                    }
                    BackendCommand::BtStopScan => {
                        tracing::info!("Bluetooth stop scan requested (stub)");
                        let _ = evt_tx.send(BackendEvent::BtDiscovering(false)).await;
                    }
                    BackendCommand::BtConnect { path } => {
                        tracing::info!("Bluetooth connect to {} requested (stub)", path);
                    }
                    BackendCommand::BtDisconnect { path } => {
                        tracing::info!("Bluetooth disconnect from {} requested (stub)", path);
                    }
                    BackendCommand::BtPair { path } => {
                        tracing::info!("Bluetooth pair with {} requested (stub)", path);
                    }
                    BackendCommand::BtRemove { path } => {
                        tracing::info!("Bluetooth remove {} requested (stub)", path);
                    }
                    BackendCommand::BtSetPowered(powered) => {
                        tracing::info!("Bluetooth set powered {} (stub)", powered);
                    }
                    BackendCommand::BtSetDiscoverable(discoverable) => {
                        tracing::info!("Bluetooth set discoverable {} (stub)", discoverable);
                    }
                }
            }
        }
    }

    tracing::info!("Backend loop terminated");
    Ok(())
}
