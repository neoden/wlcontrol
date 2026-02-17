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
    WifiForget { path: String },           // network path, backend will get known_network from it
    WifiForgetKnown { path: String },       // KnownNetwork D-Bus path, for saved-offline networks
    WifiSetPowered(bool),
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

/// Events sent from backend to UI
#[derive(Debug, Clone)]
pub enum BackendEvent {
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
    BtDeviceRemoved(String), // address
    /// Pairing interaction. kind: "confirm-passkey", "request-pin", "request-passkey",
    /// "display-passkey", "display-pin", "authorize". code: passkey/pin or empty.
    BtPairing { kind: String, address: String, code: String },
    Error(String),
}

mod imp {
    use super::{BackendCommand, BtDevice, KnownNetworkData, Sender, WifiNetwork};
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
                saved_networks: gio::ListStore::new::<WifiNetwork>(),
                bt_devices: gio::ListStore::new::<BtDevice>(),
                cached_known: RefCell::new(Vec::new()),
                cached_visible: RefCell::new(std::collections::HashSet::new()),
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
                // BlueZ confirmed state change — clear local operation flags for this device
                self.set_bt_device_flag(&data.address, |d| {
                    d.set_connecting(false);
                    d.set_disconnecting(false);
                    // Don't clear removing — that's only cleared by BtDeviceRemoved or Error
                });
                self.emit_by_name::<()>("bt-device-updated", &[]);
            }
            BackendEvent::BtDeviceRemoved(address) => self.remove_bt_device(&address),
            BackendEvent::BtPairing { kind, address, code } => {
                self.emit_by_name::<()>("bt-pairing", &[&kind, &address, &code]);
            }
            BackendEvent::Error(msg) => {
                tracing::error!("Backend error: {}", msg);
                self.clear_bt_operations();
                self.clear_wifi_operations();
                self.emit_by_name::<()>("bt-device-updated", &[]);
                self.emit_by_name::<()>("wifi-network-updated", &[]);
                self.emit_by_name::<()>("error", &[&msg]);
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
        let store = &self.imp().bt_devices;
        for i in 0..store.n_items() {
            if let Some(obj) = store.item(i) {
                let device = obj.downcast_ref::<BtDevice>().unwrap();
                device.set_connecting(device.address() == address);
            }
        }
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

use super::bluetooth::backend::{BtAdapterEventStream, BtDeviceEventStream, BtDiscoveryStream, BtPairingRequest};
use super::bluetooth::BluetoothBackend;
use super::wifi::iwd_proxy::{AgentManagerProxy, DeviceProxy, StationProxy};
use super::wifi::{IwdAgent, PassphraseRequest};
use super::wifi_backend::{get_known_networks, get_wifi_networks, has_station_interface, WifiBackend};

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
                // Always send known networks (available even when WiFi is off)
                if let Ok(known) = get_known_networks(&conn).await {
                    let _ = evt_tx.send(BackendEvent::WifiKnownNetworks(known)).await;
                }
            }
        }
    }

    // Initialize Bluetooth backend
    let (bt, bt_pairing_rx): (Option<BluetoothBackend>, Option<async_channel::Receiver<BtPairingRequest>>) =
        match BluetoothBackend::new(evt_tx.clone()).await {
            Ok((bt, rx)) => (Some(bt), Some(rx)),
            Err(e) => {
                tracing::warn!("Failed to initialize Bluetooth backend: {}. BT disabled.", e);
                let _ = evt_tx
                    .send(BackendEvent::Error(format!("Bluetooth: {}", e)))
                    .await;
                (None, None)
            }
        };

    // BT streams stored externally to avoid borrow conflicts in tokio::select!
    let mut bt_discovery_stream: Option<BtDiscoveryStream> = None;
    let mut bt_adapter_events: Option<BtAdapterEventStream> = None;
    let mut bt_device_events: futures::stream::SelectAll<BtDeviceEventStream> =
        futures::stream::SelectAll::new();
    let mut bt_tracked_devices: std::collections::HashSet<bluer::Address> =
        std::collections::HashSet::new();

    // Send initial BT state and start always-on adapter event stream
    if let Some(ref bt_backend) = bt {
        bt_backend
            .send_initial_state(&mut bt_device_events, &mut bt_tracked_devices)
            .await;
        bt_adapter_events = bt_backend.adapter_events().await;
    }

    // Store pending passphrase response sender
    let mut pending_passphrase_response: Option<oneshot::Sender<Option<String>>> = None;

    // Store pending BT pairing response senders
    let mut pending_pairing_response: Option<
        oneshot::Sender<Result<(), bluer::agent::ReqError>>,
    > = None;
    let mut pending_pin_response: Option<
        oneshot::Sender<Result<String, bluer::agent::ReqError>>,
    > = None;
    let mut pending_passkey_response: Option<
        oneshot::Sender<Result<u32, bluer::agent::ReqError>>,
    > = None;

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

    let mut bt_scan_deadline: Option<tokio::time::Instant> = None;

    loop {
        tokio::select! {
            // Auto-stop BT discovery after timeout
            _ = async {
                match bt_scan_deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending().await,
                }
            } => {
                tracing::info!("Bluetooth discovery timeout (30s), stopping scan");
                if bt_discovery_stream.take().is_some() {
                    if let Some(ref bt_backend) = bt {
                        bt_backend.notify_scan_stopped().await;
                    }
                }
                bt_scan_deadline = None;
            }
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
                            // Get initial network list and known networks
                            wifi.send_networks().await;
                            wifi.send_known_networks().await;
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
                    // When scan completes, refresh network list and known networks
                    if !scanning {
                        wifi.send_networks().await;
                        wifi.send_known_networks().await;
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
            // Handle passphrase requests from iwd agent
            Ok(request) = passphrase_rx.recv() => {
                tracing::info!("Passphrase request: {} ({})", request.network_name, request.network_path);
                pending_passphrase_response = Some(request.response_tx);
                let _ = evt_tx.send(BackendEvent::PassphraseRequest {
                    network_path: request.network_path,
                    network_name: request.network_name,
                }).await;
            }
            // Handle BT discovery stream events
            Some(adapter_event) = async {
                match bt_discovery_stream.as_mut() {
                    Some(stream) => stream.next().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(ref bt_backend) = bt {
                    bt_backend.handle_adapter_event(
                        adapter_event,
                        &mut bt_device_events,
                        &mut bt_tracked_devices,
                    ).await;
                }
            }
            // Handle always-on adapter events (DeviceAdded/DeviceRemoved even when not scanning)
            Some(adapter_event) = async {
                match bt_adapter_events.as_mut() {
                    Some(stream) => stream.next().await,
                    None => std::future::pending().await,
                }
            } => {
                if let Some(ref bt_backend) = bt {
                    bt_backend.handle_adapter_event(
                        adapter_event,
                        &mut bt_device_events,
                        &mut bt_tracked_devices,
                    ).await;
                }
            }
            // Handle per-device BT property change events
            Some((addr, event)) = bt_device_events.next() => {
                if let Some(ref bt_backend) = bt {
                    let bluer::DeviceEvent::PropertyChanged(prop) = event;
                    bt_backend.handle_device_property_change(addr, prop).await;
                }
            }
            // Handle BT pairing agent requests
            Ok(request) = async {
                match bt_pairing_rx.as_ref() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                let (kind, address, code) = match request {
                    BtPairingRequest::ConfirmPasskey { address, passkey, response_tx } => {
                        pending_pairing_response = Some(response_tx);
                        ("confirm-passkey", address, format!("{:06}", passkey))
                    }
                    BtPairingRequest::RequestPinCode { address, response_tx } => {
                        pending_pin_response = Some(response_tx);
                        ("request-pin", address, String::new())
                    }
                    BtPairingRequest::RequestPasskey { address, response_tx } => {
                        pending_passkey_response = Some(response_tx);
                        ("request-passkey", address, String::new())
                    }
                    BtPairingRequest::DisplayPasskey { address, passkey } => {
                        ("display-passkey", address, format!("{:06}", passkey))
                    }
                    BtPairingRequest::DisplayPinCode { address, pin_code } => {
                        ("display-pin", address, pin_code)
                    }
                    BtPairingRequest::RequestAuthorization { address, response_tx } => {
                        pending_pairing_response = Some(response_tx);
                        ("authorize", address, String::new())
                    }
                };
                tracing::info!("BT pairing {} for {} ({})", kind, address, code);
                let _ = evt_tx.send(BackendEvent::BtPairing {
                    kind: kind.to_string(),
                    address: address.to_string(),
                    code,
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
                        drop(bt_discovery_stream);
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
                    BackendCommand::WifiForgetKnown { path } => wifi.forget_known(&path).await,
                    BackendCommand::WifiSetPowered(powered) => wifi.set_powered(powered).await,
                    BackendCommand::BtScan => {
                        if bt_discovery_stream.is_none() {
                            if let Some(ref bt_backend) = bt {
                                bt_discovery_stream = bt_backend.start_scan().await;
                                if bt_discovery_stream.is_some() {
                                    bt_scan_deadline = Some(tokio::time::Instant::now() + std::time::Duration::from_secs(30));
                                }
                            }
                        }
                    }
                    BackendCommand::BtStopScan => {
                        if bt_discovery_stream.take().is_some() {
                            bt_scan_deadline = None;
                            if let Some(ref bt_backend) = bt {
                                bt_backend.notify_scan_stopped().await;
                            }
                        }
                    }
                    BackendCommand::BtConnect { path } => {
                        if let Some(ref bt_backend) = bt {
                            bt_backend.connect(&path).await;
                        }
                    }
                    BackendCommand::BtDisconnect { path } => {
                        if let Some(ref bt_backend) = bt {
                            bt_backend.disconnect(&path).await;
                        }
                    }
                    BackendCommand::BtPair { path } => {
                        if let Some(ref bt_backend) = bt {
                            bt_backend.pair(&path);
                        }
                    }
                    BackendCommand::BtRemove { path } => {
                        if let Some(ref bt_backend) = bt {
                            bt_backend.remove(&path).await;
                        }
                    }
                    BackendCommand::BtSetAlias { path, alias } => {
                        if let Some(ref bt_backend) = bt {
                            bt_backend.set_alias(&path, &alias).await;
                        }
                    }
                    BackendCommand::BtSetTrusted { path, trusted } => {
                        if let Some(ref bt_backend) = bt {
                            bt_backend.set_trusted_flag(&path, trusted).await;
                        }
                    }
                    BackendCommand::BtSetPowered(powered) => {
                        if !powered {
                            if bt_discovery_stream.take().is_some() {
                                if let Some(ref bt_backend) = bt {
                                    bt_backend.notify_scan_stopped().await;
                                }
                            }
                            bt_scan_deadline = None;
                            bt_tracked_devices.clear();
                            bt_device_events = futures::stream::SelectAll::new();
                            bt_adapter_events = None;
                        }
                        if let Some(ref bt_backend) = bt {
                            bt_backend.set_powered(powered).await;
                            if powered {
                                bt_backend.send_initial_state(
                                    &mut bt_device_events,
                                    &mut bt_tracked_devices,
                                ).await;
                                bt_adapter_events = bt_backend.adapter_events().await;
                            }
                        }
                    }
                    BackendCommand::BtSetDiscoverable(discoverable) => {
                        if let Some(ref bt_backend) = bt {
                            bt_backend.set_discoverable(discoverable).await;
                        }
                    }
                    BackendCommand::BtPairingResponse { accept } => {
                        if let Some(tx) = pending_pairing_response.take() {
                            let result = if accept {
                                Ok(())
                            } else {
                                Err(bluer::agent::ReqError::Rejected)
                            };
                            let _ = tx.send(result);
                        }
                    }
                    BackendCommand::BtPairingPinResponse { pin } => {
                        if let Some(tx) = pending_pin_response.take() {
                            let result = match pin {
                                Some(p) => Ok(p),
                                None => Err(bluer::agent::ReqError::Rejected),
                            };
                            let _ = tx.send(result);
                        }
                    }
                    BackendCommand::BtPairingPasskeyResponse { passkey } => {
                        if let Some(tx) = pending_passkey_response.take() {
                            let result = match passkey {
                                Some(k) => Ok(k),
                                None => Err(bluer::agent::ReqError::Rejected),
                            };
                            let _ = tx.send(result);
                        }
                    }
                }
            }
        }
    }

    tracing::info!("Backend loop terminated");
    Ok(())
}
