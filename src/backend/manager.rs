use adw::prelude::*;
use adw::subclass::prelude::*;
use async_channel::{Receiver, Sender};
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
    WifiForget { path: String },           // network path, backend will get known_network from it
    WifiForgetKnown { path: String },       // KnownNetwork D-Bus path, for saved-offline networks
    WifiSetPowered { powered: bool },
    /// Switch to a different WiFi adapter (recreate backend + streams)
    WifiSwitchAdapter { device_path: String },
    /// Response to a passphrase request (None = cancelled)
    PassphraseResponse { passphrase: Option<String> },
    BtScan,
    BtStopScan,
    BtConnect { path: String },
    BtDisconnect { path: String },
    BtPair { path: String },
    BtRemove { path: String },
    BtSetAlias { path: String, alias: String },
    BtSetTrusted { path: String, trusted: bool },
    BtSetPowered(bool),
    BtSetDiscoverable(bool),
    /// Response to a pairing confirmation/authorization (accept or reject)
    BtPairingResponse { accept: bool },
    /// Response with PIN code
    BtPairingPinResponse { pin: Option<String> },
    /// Response with numeric passkey
    BtPairingPasskeyResponse { passkey: Option<u32> },
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

/// Data for a saved (known) WiFi network from iwd KnownNetwork interface
#[derive(Debug, Clone)]
pub struct KnownNetworkData {
    pub path: String, // KnownNetwork D-Bus path
    pub name: String,
    pub network_type: String,
}

/// Data for a Bluetooth device, used to transfer between backend and UI threads
#[derive(Debug, Clone)]
pub struct BtDeviceData {
    pub address: String,
    pub name: String,
    pub alias: String,
    pub icon: String,
    pub paired: bool,
    pub trusted: bool,
    pub connected: bool,
    pub battery_percentage: i32, // -1 if not available
    pub rssi: i16,               // i16::MIN = no data
}

/// Kind of Bluetooth pairing interaction
#[derive(Debug, Clone)]
pub enum BtPairingKind {
    ConfirmPasskey(String),
    RequestPin,
    RequestPasskey,
    DisplayPasskey(String),
    DisplayPin(String),
    Authorize,
}

/// Events sent from backend to UI
#[derive(Debug, Clone)]
pub enum BackendEvent {
    /// Whether WiFi backend (iwd) is available
    WifiAvailable(bool),
    /// Whether Bluetooth backend (bluez) is available
    BtAvailable(bool),
    /// List of available WiFi adapters + which one is active
    WifiDevices {
        devices: Vec<super::wifi_backend::IwdDeviceInfo>,
        active_path: Option<String>,
    },
    WifiPowered(bool),
    WifiScanning(bool),
    WifiNetworks(Vec<WifiNetworkData>),
    WifiConnected(Option<String>),    // path of connected network, or None
    WifiConnecting(String),           // path of network we're connecting to
    WifiNetworkKnown { path: String },            // network became known (saved)
    WifiKnownNetworks(Vec<KnownNetworkData>),      // all saved networks from iwd
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
    BtConnecting(String),    // address of device we're connecting/pairing to
    BtDeviceAdded(BtDeviceData),
    BtDeviceChanged(BtDeviceData),
    /// Device operation (connect/disconnect/pair) completed — carries
    /// re-read device state from BlueZ + optional error message.
    BtOperationDone {
        data: BtDeviceData,
        error: Option<String>,
    },
    BtDeviceRemoved(String), // address
    BtPairing { kind: BtPairingKind, address: String },
    BtError(String),
    WifiError(String),
}

mod imp {
    use super::{BackendCommand, BtDevice, KnownNetworkData, Sender, WifiNetwork};
    use super::super::wifi_backend::IwdDeviceInfo;
    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use gtk::{gio, glib};
    use std::cell::RefCell;
    use std::sync::OnceLock;

    pub struct WlcontrolManager {
        pub wifi_networks: gio::ListStore,
        pub saved_networks: gio::ListStore,
        pub bt_devices: gio::ListStore,
        /// Cached known networks from iwd, for cross-filtering with scan results
        pub cached_known: RefCell<Vec<KnownNetworkData>>,
        /// Cached (name, type) pairs from scan results, for filtering known networks
        pub cached_visible: RefCell<std::collections::HashSet<(String, String)>>,
        pub wifi_available: RefCell<bool>,
        pub wifi_powered: RefCell<bool>,
        pub wifi_scanning: RefCell<bool>,
        pub bt_available: RefCell<bool>,
        pub bt_powered: RefCell<bool>,
        pub bt_discovering: RefCell<bool>,
        pub bt_discoverable: RefCell<bool>,
        pub cmd_tx: OnceLock<Sender<BackendCommand>>,
        /// All discovered WiFi adapters
        pub wifi_adapters: RefCell<Vec<IwdDeviceInfo>>,
        /// Device path of the currently active WiFi adapter
        pub active_wifi_device: RefCell<Option<String>>,
    }

    impl Default for WlcontrolManager {
        fn default() -> Self {
            Self {
                wifi_networks: gio::ListStore::new::<WifiNetwork>(),
                saved_networks: gio::ListStore::new::<WifiNetwork>(),
                bt_devices: gio::ListStore::new::<BtDevice>(),
                cached_known: RefCell::new(Vec::new()),
                cached_visible: RefCell::new(std::collections::HashSet::new()),
                wifi_available: RefCell::new(false),
                wifi_powered: RefCell::new(false),
                wifi_scanning: RefCell::new(false),
                bt_available: RefCell::new(false),
                bt_powered: RefCell::new(false),
                bt_discovering: RefCell::new(false),
                bt_discoverable: RefCell::new(false),
                cmd_tx: OnceLock::new(),
                wifi_adapters: RefCell::new(Vec::new()),
                active_wifi_device: RefCell::new(None),
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
                    glib::ParamSpecBoolean::builder("wifi-available")
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("wifi-powered").build(),
                    glib::ParamSpecBoolean::builder("wifi-scanning")
                        .read_only()
                        .build(),
                    glib::ParamSpecUInt::builder("wifi-adapter-count")
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("bt-available")
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
                    glib::subclass::Signal::builder("wifi-error")
                        .param_types([String::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("bt-error")
                        .param_types([String::static_type()])
                        .build(),
                    glib::subclass::Signal::builder("wifi-adapters-changed").build(),
                    glib::subclass::Signal::builder("wifi-network-updated").build(),
                    glib::subclass::Signal::builder("bt-device-updated").build(),
                    glib::subclass::Signal::builder("bt-pairing")
                        .param_types([
                            String::static_type(), // kind
                            String::static_type(), // address
                            String::static_type(), // code
                        ])
                        .build(),
                ]
            })
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "wifi-available" => self.wifi_available.borrow().to_value(),
                "wifi-powered" => self.wifi_powered.borrow().to_value(),
                "wifi-scanning" => self.wifi_scanning.borrow().to_value(),
                "wifi-adapter-count" => (self.wifi_adapters.borrow().len() as u32).to_value(),
                "bt-available" => self.bt_available.borrow().to_value(),
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
                    if let Some(tx) = self.cmd_tx.get() {
                        let tx = tx.clone();
                        glib::spawn_future_local(async move {
                            let _ = tx.send(BackendCommand::WifiSetPowered { powered }).await;
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
            BackendEvent::WifiAvailable(available) => {
                self.set_wifi_available(available);
            }
            BackendEvent::BtAvailable(available) => {
                self.set_bt_available(available);
            }
            BackendEvent::WifiDevices { devices, active_path } => {
                if let Some(ref path) = active_path {
                    self.imp().active_wifi_device.replace(Some(path.clone()));
                } else {
                    let current = self.imp().active_wifi_device.borrow().clone();
                    let still_present = current.as_ref()
                        .map(|p| devices.iter().any(|d| d.device_path == *p))
                        .unwrap_or(false);
                    if !still_present {
                        self.imp().active_wifi_device.replace(
                            devices.first().map(|d| d.device_path.clone())
                        );
                    }
                }
                self.imp().wifi_adapters.replace(devices);
                self.notify("wifi-adapter-count");
                self.emit_by_name::<()>("wifi-adapters-changed", &[]);
            }
            BackendEvent::WifiPowered(powered) => self.set_wifi_powered(powered),
            BackendEvent::WifiScanning(scanning) => self.set_wifi_scanning(scanning),
            BackendEvent::WifiNetworks(networks) => {
                self.update_wifi_networks(networks);
                self.rebuild_saved_networks();
            }
            BackendEvent::WifiKnownNetworks(known) => {
                self.imp().cached_known.replace(known);
                self.rebuild_saved_networks();
            }
            BackendEvent::WifiConnected(path) => {
                self.clear_wifi_operations();
                self.update_wifi_connected(path);
                self.emit_by_name::<()>("wifi-network-updated", &[]);
            }
            BackendEvent::WifiConnecting(path) => {
                self.set_wifi_connecting(&path);
                self.emit_by_name::<()>("wifi-network-updated", &[]);
            }
            BackendEvent::WifiNetworkKnown { path } => {
                self.set_wifi_network_known(&path);
                self.emit_by_name::<()>("wifi-network-updated", &[]);
            }
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
            BackendEvent::BtPowered(powered) => {
                if !powered {
                    self.set_bt_discovering(false);
                    self.clear_bt_operations();
                    self.reset_bt_connected_state();
                    self.remove_unpaired_bt_devices();
                    self.emit_by_name::<()>("bt-device-updated", &[]);
                }
                self.set_bt_powered(powered);
            }
            BackendEvent::BtDiscovering(discovering) => self.set_bt_discovering(discovering),
            BackendEvent::BtDiscoverable(discoverable) => self.set_bt_discoverable(discoverable),
            BackendEvent::BtConnecting(address) => self.set_bt_connecting(&address),
            BackendEvent::BtDeviceAdded(data) => self.add_bt_device(&data),
            BackendEvent::BtDeviceChanged(data) => {
                self.update_bt_device(&data);
                self.emit_by_name::<()>("bt-device-updated", &[]);
            }
            BackendEvent::BtOperationDone { data, error } => {
                self.update_bt_device(&data);
                self.set_bt_device_flag(&data.address, |d| {
                    d.set_connecting(false);
                    d.set_disconnecting(false);
                });
                self.emit_by_name::<()>("bt-device-updated", &[]);
                if let Some(msg) = error {
                    self.emit_by_name::<()>("bt-error", &[&msg]);
                }
            }
            BackendEvent::BtDeviceRemoved(address) => self.remove_bt_device(&address),
            BackendEvent::BtPairing { kind, address } => {
                let (kind_str, code) = match &kind {
                    BtPairingKind::ConfirmPasskey(code) => ("confirm-passkey", code.as_str()),
                    BtPairingKind::RequestPin => ("request-pin", ""),
                    BtPairingKind::RequestPasskey => ("request-passkey", ""),
                    BtPairingKind::DisplayPasskey(code) => ("display-passkey", code.as_str()),
                    BtPairingKind::DisplayPin(code) => ("display-pin", code.as_str()),
                    BtPairingKind::Authorize => ("authorize", ""),
                };
                self.emit_by_name::<()>("bt-pairing", &[&kind_str, &address, &code]);
            }
            BackendEvent::BtError(msg) => {
                tracing::error!("BT error: {}", msg);
                self.clear_bt_operations();
                self.emit_by_name::<()>("bt-device-updated", &[]);
                self.emit_by_name::<()>("bt-error", &[&msg]);
            }
            BackendEvent::WifiError(msg) => {
                tracing::error!("WiFi error: {}", msg);
                self.clear_wifi_operations();
                self.emit_by_name::<()>("wifi-network-updated", &[]);
                self.emit_by_name::<()>("wifi-error", &[&msg]);
            }
        }
    }

    fn update_wifi_networks(&self, networks: Vec<WifiNetworkData>) {
        // Cache visible (name, type) pairs for filtering known networks
        let visible: std::collections::HashSet<(String, String)> = networks
            .iter()
            .map(|n| (n.name.clone(), n.network_type.clone()))
            .collect();
        self.imp().cached_visible.replace(visible);

        let store = &self.imp().wifi_networks;

        // Index existing GObjects by path for reuse
        let mut existing: std::collections::HashMap<String, WifiNetwork> =
            std::collections::HashMap::new();
        for i in 0..store.n_items() {
            let network = store.item(i).unwrap().downcast::<WifiNetwork>().unwrap();
            existing.insert(network.path(), network);
        }

        // Build new list, reusing existing GObjects (preserves operation flags)
        let new_items: Vec<WifiNetwork> = networks
            .iter()
            .map(|data| {
                if let Some(network) = existing.remove(&data.path) {
                    network.set_signal_strength(data.signal_strength);
                    network.set_connected(data.connected);
                    network.set_known(data.known);
                    network
                } else {
                    WifiNetwork::new(
                        &data.path,
                        &data.name,
                        &data.network_type,
                        data.signal_strength,
                        data.connected,
                        data.known,
                    )
                }
            })
            .collect();

        store.splice(0, store.n_items(), &new_items);
    }

    /// Rebuild saved_networks store from cached known networks,
    /// excluding those already visible in scan results.
    fn rebuild_saved_networks(&self) {
        let imp = self.imp();
        let known = imp.cached_known.borrow();
        let visible = imp.cached_visible.borrow();
        let store = &imp.saved_networks;
        store.remove_all();
        for data in known.iter() {
            if !visible.contains(&(data.name.clone(), data.network_type.clone())) {
                let network = WifiNetwork::new_saved_offline(
                    &data.path,
                    &data.name,
                    &data.network_type,
                );
                store.append(&network);
            }
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

    /// Clear all local operation flags on all WiFi networks.
    /// Called on connection events and errors as a conservative reset.
    fn clear_wifi_operations(&self) {
        let store = &self.imp().wifi_networks;
        for i in 0..store.n_items() {
            if let Some(obj) = store.item(i) {
                let network = obj.downcast_ref::<WifiNetwork>().unwrap();
                network.set_connecting(false);
                network.set_disconnecting(false);
                network.set_forgetting(false);
            }
        }
    }

    fn set_wifi_network_flag(&self, path: &str, f: impl Fn(&WifiNetwork)) {
        let store = &self.imp().wifi_networks;
        for i in 0..store.n_items() {
            if let Some(obj) = store.item(i) {
                let network = obj.downcast_ref::<WifiNetwork>().unwrap();
                if network.path() == path {
                    f(&network);
                    return;
                }
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

    pub fn saved_networks(&self) -> gio::ListStore {
        self.imp().saved_networks.clone()
    }

    pub fn bt_devices(&self) -> gio::ListStore {
        self.imp().bt_devices.clone()
    }

    pub fn wifi_available(&self) -> bool {
        *self.imp().wifi_available.borrow()
    }

    fn set_wifi_available(&self, available: bool) {
        if *self.imp().wifi_available.borrow() != available {
            self.imp().wifi_available.replace(available);
            self.notify("wifi-available");
        }
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

    pub fn bt_available(&self) -> bool {
        *self.imp().bt_available.borrow()
    }

    fn set_bt_available(&self, available: bool) {
        if *self.imp().bt_available.borrow() != available {
            self.imp().bt_available.replace(available);
            self.notify("bt-available");
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

    fn remove_unpaired_bt_devices(&self) {
        let store = &self.imp().bt_devices;
        let mut i = 0;
        while i < store.n_items() {
            if let Some(obj) = store.item(i) {
                let device = obj.downcast_ref::<BtDevice>().unwrap();
                if !device.paired() {
                    store.remove(i);
                    continue;
                }
            }
            i += 1;
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

    pub fn wifi_adapters(&self) -> Vec<super::wifi_backend::IwdDeviceInfo> {
        self.imp().wifi_adapters.borrow().clone()
    }

    pub fn active_wifi_device_path(&self) -> Option<String> {
        self.imp().active_wifi_device.borrow().clone()
    }

    pub fn set_active_wifi_adapter(&self, device_path: &str) {
        self.imp().active_wifi_device.replace(Some(device_path.to_string()));
        // Clear current models while backend loads new state
        self.imp().wifi_networks.remove_all();
        self.imp().saved_networks.remove_all();
        self.send_command(BackendCommand::WifiSwitchAdapter {
            device_path: device_path.to_string(),
        });
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
        // Set disconnecting flag on the currently connected network for instant UI feedback
        let store = &self.imp().wifi_networks;
        for i in 0..store.n_items() {
            if let Some(obj) = store.item(i) {
                let network = obj.downcast_ref::<WifiNetwork>().unwrap();
                if network.connected() {
                    network.set_disconnecting(true);
                    break;
                }
            }
        }
        self.emit_by_name::<()>("wifi-network-updated", &[]);
        self.send_command(BackendCommand::WifiDisconnect);
    }

    pub fn request_wifi_forget(&self, path: &str) {
        self.set_wifi_network_flag(path, |n| n.set_forgetting(true));
        self.emit_by_name::<()>("wifi-network-updated", &[]);
        self.send_command(BackendCommand::WifiForget {
            path: path.to_string(),
        });
    }

    /// Forget a saved-offline network using its KnownNetwork D-Bus path directly
    pub fn request_wifi_forget_known(&self, path: &str) {
        // Set forgetting flag on the saved network for UI feedback
        let store = &self.imp().saved_networks;
        for i in 0..store.n_items() {
            if let Some(obj) = store.item(i) {
                let network = obj.downcast_ref::<WifiNetwork>().unwrap();
                if network.path() == path {
                    network.set_forgetting(true);
                    break;
                }
            }
        }
        self.send_command(BackendCommand::WifiForgetKnown {
            path: path.to_string(),
        });
    }

    pub fn send_passphrase_response(&self, passphrase: Option<String>) {
        self.send_command(BackendCommand::PassphraseResponse { passphrase });
    }

    pub fn request_bt_scan(&self) {
        self.send_command(BackendCommand::BtScan);
    }

    pub fn request_bt_stop_scan(&self) {
        self.send_command(BackendCommand::BtStopScan);
    }

    pub fn request_bt_connect(&self, path: &str) {
        self.set_bt_device_flag(path, |d| d.set_connecting(true));
        self.send_command(BackendCommand::BtConnect {
            path: path.to_string(),
        });
    }

    pub fn request_bt_disconnect(&self, path: &str) {
        self.set_bt_device_flag(path, |d| d.set_disconnecting(true));
        self.send_command(BackendCommand::BtDisconnect {
            path: path.to_string(),
        });
    }

    pub fn request_bt_pair(&self, path: &str) {
        self.set_bt_device_flag(path, |d| d.set_connecting(true));
        self.send_command(BackendCommand::BtPair {
            path: path.to_string(),
        });
    }

    pub fn request_bt_set_alias(&self, path: &str, alias: &str) {
        self.send_command(BackendCommand::BtSetAlias {
            path: path.to_string(),
            alias: alias.to_string(),
        });
    }

    pub fn request_bt_set_trusted(&self, path: &str, trusted: bool) {
        self.send_command(BackendCommand::BtSetTrusted {
            path: path.to_string(),
            trusted,
        });
    }

    pub fn request_bt_remove(&self, path: &str) {
        self.set_bt_device_flag(path, |d| d.set_removing(true));
        self.send_command(BackendCommand::BtRemove {
            path: path.to_string(),
        });
    }

    pub fn send_bt_pairing_response(&self, accept: bool) {
        self.send_command(BackendCommand::BtPairingResponse { accept });
    }

    pub fn send_bt_pairing_pin(&self, pin: Option<String>) {
        self.send_command(BackendCommand::BtPairingPinResponse { pin });
    }

    pub fn send_bt_pairing_passkey(&self, passkey: Option<u32>) {
        self.send_command(BackendCommand::BtPairingPasskeyResponse { passkey });
    }

    fn set_bt_device_flag(&self, address: &str, f: impl Fn(&BtDevice)) {
        if let Some(idx) = self.find_bt_device_index(address) {
            if let Some(obj) = self.imp().bt_devices.item(idx) {
                f(obj.downcast_ref::<BtDevice>().unwrap());
            }
        }
    }

    fn set_bt_connecting(&self, address: &str) {
        self.set_bt_device_flag(address, |d| d.set_connecting(true));
    }

    /// Reset connected state on all devices (adapter powered off).
    fn reset_bt_connected_state(&self) {
        let store = &self.imp().bt_devices;
        for i in 0..store.n_items() {
            if let Some(obj) = store.item(i) {
                let device = obj.downcast_ref::<BtDevice>().unwrap();
                device.set_connected(false);
            }
        }
    }

    /// Clear all local operation flags on all devices.
    /// Called on errors and state changes as a conservative reset.
    fn clear_bt_operations(&self) {
        let store = &self.imp().bt_devices;
        for i in 0..store.n_items() {
            if let Some(obj) = store.item(i) {
                let device = obj.downcast_ref::<BtDevice>().unwrap();
                device.set_connecting(false);
                device.set_disconnecting(false);
                device.set_removing(false);
            }
        }
    }

    fn find_bt_device_index(&self, address: &str) -> Option<u32> {
        let store = &self.imp().bt_devices;
        for i in 0..store.n_items() {
            if let Some(obj) = store.item(i) {
                let device = obj.downcast_ref::<BtDevice>().unwrap();
                if device.address() == address {
                    return Some(i);
                }
            }
        }
        None
    }

    fn add_bt_device(&self, data: &BtDeviceData) {
        // If already exists, update instead
        if self.find_bt_device_index(&data.address).is_some() {
            self.update_bt_device(data);
            self.emit_by_name::<()>("bt-device-updated", &[]);
            return;
        }
        let device = BtDevice::new(
            &data.address, // path = address (bluer uses addresses, not D-Bus paths)
            &data.address,
            if data.name.is_empty() {
                &data.alias
            } else {
                &data.name
            },
            &data.icon,
            data.paired,
            data.connected,
        );
        device.set_alias(&data.alias);
        device.set_trusted(data.trusted);
        device.set_battery_percentage(data.battery_percentage);
        device.set_rssi(data.rssi);
        self.imp().bt_devices.append(&device);
    }

    fn update_bt_device(&self, data: &BtDeviceData) {
        let store = &self.imp().bt_devices;
        if let Some(idx) = self.find_bt_device_index(&data.address) {
            if let Some(obj) = store.item(idx) {
                let device = obj.downcast_ref::<BtDevice>().unwrap();
                device.set_name(&data.name);
                device.set_alias(&data.alias);
                device.set_icon(&data.icon);
                device.set_paired(data.paired);
                device.set_trusted(data.trusted);
                device.set_connected(data.connected);
                device.set_battery_percentage(data.battery_percentage);
                device.set_rssi(data.rssi);
            }
        }
    }

    fn remove_bt_device(&self, address: &str) {
        if let Some(idx) = self.find_bt_device_index(address) {
            self.imp().bt_devices.remove(idx);
        }
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

async fn run_backend(
    cmd_rx: Receiver<BackendCommand>,
    evt_tx: Sender<BackendEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (mut state, mut streams) = super::event_loop::init(cmd_rx, evt_tx).await?;

    loop {
        let event = streams.next_event().await;
        match state.handle_event(event, &mut streams).await {
            super::event_loop::LoopAction::Continue => {}
            super::event_loop::LoopAction::Break => break,
        }
    }

    tracing::info!("Backend loop terminated");
    Ok(())
}
