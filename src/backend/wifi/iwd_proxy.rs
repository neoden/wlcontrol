//! zbus proxy traits for iwd D-Bus interfaces
//!
//! iwd uses the service name `net.connman.iwd` and provides several interfaces:
//! - Station: Main interface for scanning and connecting
//! - Network: Represents an available network
//! - KnownNetwork: Represents a saved network
//! - Device: Low-level adapter control

use zbus::proxy;

/// net.connman.iwd.Station interface
/// Object path: /net/connman/iwd/{phy}/{dev}/station
#[proxy(
    interface = "net.connman.iwd.Station",
    default_service = "net.connman.iwd",
    gen_blocking = false
)]
pub trait Station {
    /// Initiate a network scan
    fn scan(&self) -> zbus::Result<()>;

    /// Disconnect from current network
    fn disconnect(&self) -> zbus::Result<()>;

    /// Get list of networks ordered by signal strength
    /// Returns Vec<(object_path, signal_strength)>
    fn get_ordered_networks(&self) -> zbus::Result<Vec<(zbus::zvariant::OwnedObjectPath, i16)>>;

    /// Current station state: "connected", "connecting", "disconnecting", "disconnected", etc.
    #[zbus(property)]
    fn state(&self) -> zbus::Result<String>;

    /// Whether a scan is in progress
    #[zbus(property)]
    fn scanning(&self) -> zbus::Result<bool>;

    /// Path to currently connected network (if any)
    #[zbus(property)]
    fn connected_network(&self) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

/// net.connman.iwd.Network interface
/// Object path: /net/connman/iwd/{phy}/{dev}/{network_id}
#[proxy(
    interface = "net.connman.iwd.Network",
    default_service = "net.connman.iwd",
    gen_blocking = false
)]
pub trait Network {
    /// Connect to this network
    fn connect(&self) -> zbus::Result<()>;

    /// Network SSID
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;

    /// Security type: "open", "psk", "8021x"
    #[zbus(property, name = "Type")]
    fn network_type(&self) -> zbus::Result<String>;

    /// Whether currently connected
    #[zbus(property)]
    fn connected(&self) -> zbus::Result<bool>;

    /// Path to KnownNetwork if this is a saved network
    #[zbus(property)]
    fn known_network(&self) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

/// net.connman.iwd.KnownNetwork interface
/// Object path: /net/connman/iwd/{known_network_id}
#[proxy(
    interface = "net.connman.iwd.KnownNetwork",
    default_service = "net.connman.iwd",
    gen_blocking = false
)]
pub trait KnownNetwork {
    /// Remove this network from saved networks
    fn forget(&self) -> zbus::Result<()>;

    /// Network SSID
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;

    /// Security type
    #[zbus(property, name = "Type")]
    fn network_type(&self) -> zbus::Result<String>;

    /// Whether to auto-connect when in range
    #[zbus(property)]
    fn auto_connect(&self) -> zbus::Result<bool>;

    /// Set auto-connect preference
    #[zbus(property)]
    fn set_auto_connect(&self, value: bool) -> zbus::Result<()>;

    /// Unix timestamp of last successful connection
    #[zbus(property)]
    fn last_connected_time(&self) -> zbus::Result<String>;
}

/// net.connman.iwd.Device interface
/// Object path: /net/connman/iwd/{phy}/{dev}
#[proxy(
    interface = "net.connman.iwd.Device",
    default_service = "net.connman.iwd",
    gen_blocking = false
)]
pub trait Device {
    /// Device name (e.g., "wlan0")
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;

    /// MAC address
    #[zbus(property)]
    fn address(&self) -> zbus::Result<String>;

    /// Whether the adapter is powered on
    #[zbus(property)]
    fn powered(&self) -> zbus::Result<bool>;

    /// Set adapter power state
    #[zbus(property)]
    fn set_powered(&self, value: bool) -> zbus::Result<()>;

    /// Current mode: "station", "ap", "ad-hoc"
    #[zbus(property)]
    fn mode(&self) -> zbus::Result<String>;

    /// Object path to the parent Adapter
    #[zbus(property)]
    fn adapter(&self) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

/// net.connman.iwd.Adapter interface
/// Object path: /net/connman/iwd/{phy}
#[proxy(
    interface = "net.connman.iwd.Adapter",
    default_service = "net.connman.iwd",
    gen_blocking = false
)]
pub trait Adapter {
    /// Adapter name (e.g., "phy0")
    #[zbus(property)]
    fn name(&self) -> zbus::Result<String>;

    /// Human-readable model name
    #[zbus(property)]
    fn model(&self) -> zbus::Result<String>;

    /// Vendor name
    #[zbus(property)]
    fn vendor(&self) -> zbus::Result<String>;

    /// Supported modes
    #[zbus(property)]
    fn supported_modes(&self) -> zbus::Result<Vec<String>>;
}

/// net.connman.iwd.AgentManager interface
/// Used to register our agent for password prompts
#[proxy(
    interface = "net.connman.iwd.AgentManager",
    default_service = "net.connman.iwd",
    default_path = "/net/connman/iwd",
    gen_blocking = false
)]
pub trait AgentManager {
    /// Register an agent at the given path
    fn register_agent(&self, path: zbus::zvariant::ObjectPath<'_>) -> zbus::Result<()>;

    /// Unregister the agent
    fn unregister_agent(&self, path: zbus::zvariant::ObjectPath<'_>) -> zbus::Result<()>;
}
