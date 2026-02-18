//! Data types shared between backend and UI layers.

use super::wifi::IwdDeviceInfo;

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
    BtSetPowered { powered: bool },
    BtSetDiscoverable { discoverable: bool },
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
        devices: Vec<IwdDeviceInfo>,
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
