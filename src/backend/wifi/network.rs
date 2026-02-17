use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

/// Canonical WiFi network state, derived from iwd properties + local operation flags.
/// Local flags (connecting/disconnecting/forgetting) take priority over iwd state,
/// giving instant UI feedback before the backend confirms.
pub enum WifiNetworkState {
    Available,
    Saved,
    SavedOffline,
    Connecting,
    Connected,
    Disconnecting,
    Forgetting,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct WifiNetwork {
        // iwd properties
        pub path: RefCell<String>,
        pub name: RefCell<String>,
        pub network_type: RefCell<String>, // "open", "psk", "8021x"
        pub signal_strength: Cell<i16>,    // cBm from iwd
        pub connected: Cell<bool>,
        pub known: Cell<bool>,
        pub offline: Cell<bool>, // saved network not in range
        // Local operation flags
        pub connecting: Cell<bool>,
        pub disconnecting: Cell<bool>,
        pub forgetting: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WifiNetwork {
        const NAME: &'static str = "WifiNetwork";
        type Type = super::WifiNetwork;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for WifiNetwork {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: OnceLock<Vec<glib::ParamSpec>> = OnceLock::new();
            PROPERTIES.get_or_init(|| {
                vec![
                    glib::ParamSpecString::builder("path").read_only().build(),
                    glib::ParamSpecString::builder("name").read_only().build(),
                    glib::ParamSpecString::builder("network-type")
                        .read_only()
                        .build(),
                    glib::ParamSpecInt::builder("signal-strength")
                        .minimum(-120)
                        .maximum(0)
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("connected")
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("known")
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("offline")
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("connecting")
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("disconnecting")
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("forgetting")
                        .read_only()
                        .build(),
                ]
            })
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "path" => self.path.borrow().to_value(),
                "name" => self.name.borrow().to_value(),
                "network-type" => self.network_type.borrow().to_value(),
                "signal-strength" => (self.signal_strength.get() as i32).to_value(),
                "connected" => self.connected.get().to_value(),
                "known" => self.known.get().to_value(),
                "offline" => self.offline.get().to_value(),
                "connecting" => self.connecting.get().to_value(),
                "disconnecting" => self.disconnecting.get().to_value(),
                "forgetting" => self.forgetting.get().to_value(),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    pub struct WifiNetwork(ObjectSubclass<imp::WifiNetwork>);
}

impl WifiNetwork {
    pub fn new(
        path: &str,
        name: &str,
        network_type: &str,
        signal_strength: i16,
        connected: bool,
        known: bool,
    ) -> Self {
        let network: Self = glib::Object::new();
        let imp = network.imp();
        imp.path.replace(path.to_string());
        imp.name.replace(name.to_string());
        imp.network_type.replace(network_type.to_string());
        imp.signal_strength.set(signal_strength);
        imp.connected.set(connected);
        imp.known.set(known);
        network
    }

    /// Create a saved-offline network (from KnownNetwork, not in scan results)
    pub fn new_saved_offline(path: &str, name: &str, network_type: &str) -> Self {
        let network: Self = glib::Object::new();
        let imp = network.imp();
        imp.path.replace(path.to_string());
        imp.name.replace(name.to_string());
        imp.network_type.replace(network_type.to_string());
        imp.known.set(true);
        imp.offline.set(true);
        network
    }

    /// Derive canonical state from iwd booleans + local operation flags.
    /// Local flags take priority — they represent user-initiated actions
    /// that haven't been confirmed by the backend yet.
    pub fn state(&self) -> WifiNetworkState {
        if self.forgetting() {
            return WifiNetworkState::Forgetting;
        }
        if self.disconnecting() {
            return WifiNetworkState::Disconnecting;
        }
        if self.connecting() {
            return WifiNetworkState::Connecting;
        }
        if self.connected() {
            return WifiNetworkState::Connected;
        }
        if self.known() && self.offline() {
            return WifiNetworkState::SavedOffline;
        }
        if self.known() {
            return WifiNetworkState::Saved;
        }
        WifiNetworkState::Available
    }

    pub fn path(&self) -> String {
        self.imp().path.borrow().clone()
    }

    pub fn name(&self) -> String {
        self.imp().name.borrow().clone()
    }

    pub fn network_type(&self) -> String {
        self.imp().network_type.borrow().clone()
    }

    pub fn signal_strength(&self) -> i16 {
        self.imp().signal_strength.get()
    }

    pub fn connected(&self) -> bool {
        self.imp().connected.get()
    }

    pub fn known(&self) -> bool {
        self.imp().known.get()
    }

    pub fn connecting(&self) -> bool {
        self.imp().connecting.get()
    }

    pub fn disconnecting(&self) -> bool {
        self.imp().disconnecting.get()
    }

    pub fn offline(&self) -> bool {
        self.imp().offline.get()
    }

    pub fn forgetting(&self) -> bool {
        self.imp().forgetting.get()
    }

    pub fn set_connected(&self, connected: bool) {
        if self.imp().connected.get() != connected {
            self.imp().connected.set(connected);
            self.notify("connected");
        }
    }

    pub fn set_known(&self, known: bool) {
        if self.imp().known.get() != known {
            self.imp().known.set(known);
            self.notify("known");
        }
    }

    pub fn set_connecting(&self, connecting: bool) {
        if self.imp().connecting.get() != connecting {
            self.imp().connecting.set(connecting);
            self.notify("connecting");
        }
    }

    pub fn set_disconnecting(&self, disconnecting: bool) {
        if self.imp().disconnecting.get() != disconnecting {
            self.imp().disconnecting.set(disconnecting);
            self.notify("disconnecting");
        }
    }

    pub fn set_forgetting(&self, forgetting: bool) {
        if self.imp().forgetting.get() != forgetting {
            self.imp().forgetting.set(forgetting);
            self.notify("forgetting");
        }
    }

    pub fn set_signal_strength(&self, strength: i16) {
        if self.imp().signal_strength.get() != strength {
            self.imp().signal_strength.set(strength);
            self.notify("signal-strength");
        }
    }

    pub fn is_secured(&self) -> bool {
        self.network_type() != "open"
    }

    /// Returns icon name based on signal strength (iwd returns cBm, i.e. dBm * 100)
    pub fn signal_icon(&self) -> &'static str {
        // Convert from cBm to dBm for comparison
        let dbm = self.signal_strength() / 100;
        match dbm {
            -50..=0 => "network-wireless-signal-excellent-symbolic",
            -60..=-51 => "network-wireless-signal-good-symbolic",
            -70..=-61 => "network-wireless-signal-ok-symbolic",
            _ => "network-wireless-signal-weak-symbolic",
        }
    }

    /// Returns signal strength in dBm (iwd stores in cBm)
    pub fn signal_dbm(&self) -> i16 {
        self.signal_strength() / 100
    }
}
