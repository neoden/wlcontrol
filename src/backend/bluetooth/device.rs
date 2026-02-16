use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

/// Canonical device state, derived from BlueZ properties + local operation flags.
/// Local operations take priority: if user clicked "forget", state is Removing
/// even though BlueZ still reports paired=true.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BtDeviceState {
    Discovered,
    Pairing,
    Paired,
    Connecting,
    Connected,
    Disconnecting,
    Removing,
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct BtDevice {
        pub path: RefCell<String>,
        pub address: RefCell<String>,
        pub name: RefCell<String>,
        pub alias: RefCell<String>,
        pub icon: RefCell<String>,
        pub paired: Cell<bool>,
        pub trusted: Cell<bool>,
        pub connected: Cell<bool>,
        pub connecting: Cell<bool>,
        pub disconnecting: Cell<bool>,
        pub removing: Cell<bool>,
        pub battery_percentage: Cell<i32>, // -1 if not available
        pub rssi: Cell<i16>,                // i16::MIN = no data
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BtDevice {
        const NAME: &'static str = "BtDevice";
        type Type = super::BtDevice;
        type ParentType = glib::Object;
    }

    impl ObjectImpl for BtDevice {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: OnceLock<Vec<glib::ParamSpec>> = OnceLock::new();
            PROPERTIES.get_or_init(|| {
                vec![
                    glib::ParamSpecString::builder("path").read_only().build(),
                    glib::ParamSpecString::builder("address").read_only().build(),
                    glib::ParamSpecString::builder("name").read_only().build(),
                    glib::ParamSpecString::builder("alias").read_only().build(),
                    glib::ParamSpecString::builder("icon").read_only().build(),
                    glib::ParamSpecBoolean::builder("paired").read_only().build(),
                    glib::ParamSpecBoolean::builder("trusted")
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("connected")
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("connecting")
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("disconnecting")
                        .read_only()
                        .build(),
                    glib::ParamSpecBoolean::builder("removing")
                        .read_only()
                        .build(),
                    glib::ParamSpecInt::builder("battery-percentage")
                        .minimum(-1)
                        .maximum(100)
                        .default_value(-1)
                        .read_only()
                        .build(),
                    glib::ParamSpecInt::builder("rssi")
                        .minimum(i16::MIN as i32)
                        .maximum(i16::MAX as i32)
                        .default_value(i16::MIN as i32)
                        .read_only()
                        .build(),
                ]
            })
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            match pspec.name() {
                "path" => self.path.borrow().to_value(),
                "address" => self.address.borrow().to_value(),
                "name" => self.name.borrow().to_value(),
                "alias" => self.alias.borrow().to_value(),
                "icon" => self.icon.borrow().to_value(),
                "paired" => self.paired.get().to_value(),
                "trusted" => self.trusted.get().to_value(),
                "connected" => self.connected.get().to_value(),
                "connecting" => self.connecting.get().to_value(),
                "disconnecting" => self.disconnecting.get().to_value(),
                "removing" => self.removing.get().to_value(),
                "battery-percentage" => self.battery_percentage.get().to_value(),
                "rssi" => (self.rssi.get() as i32).to_value(),
                _ => unimplemented!(),
            }
        }
    }
}

glib::wrapper! {
    pub struct BtDevice(ObjectSubclass<imp::BtDevice>);
}

impl BtDevice {
    pub fn new(
        path: &str,
        address: &str,
        name: &str,
        icon: &str,
        paired: bool,
        connected: bool,
    ) -> Self {
        let device: Self = glib::Object::new();
        let imp = device.imp();
        imp.path.replace(path.to_string());
        imp.address.replace(address.to_string());
        imp.name.replace(name.to_string());
        imp.alias.replace(name.to_string());
        imp.icon.replace(icon.to_string());
        imp.paired.set(paired);
        imp.connected.set(connected);
        imp.battery_percentage.set(-1);
        imp.rssi.set(i16::MIN);
        device
    }

    pub fn path(&self) -> String {
        self.imp().path.borrow().clone()
    }

    pub fn address(&self) -> String {
        self.imp().address.borrow().clone()
    }

    pub fn name(&self) -> String {
        self.imp().name.borrow().clone()
    }

    pub fn display_name(&self) -> String {
        let alias = self.imp().alias.borrow();
        if !alias.is_empty() {
            alias.clone()
        } else {
            let name = self.imp().name.borrow();
            if !name.is_empty() {
                name.clone()
            } else {
                self.imp().address.borrow().clone()
            }
        }
    }

    pub fn icon(&self) -> String {
        self.imp().icon.borrow().clone()
    }

    pub fn paired(&self) -> bool {
        self.imp().paired.get()
    }

    pub fn trusted(&self) -> bool {
        self.imp().trusted.get()
    }

    pub fn connected(&self) -> bool {
        self.imp().connected.get()
    }

    pub fn connecting(&self) -> bool {
        self.imp().connecting.get()
    }

    pub fn battery_percentage(&self) -> i32 {
        self.imp().battery_percentage.get()
    }

    pub fn has_battery(&self) -> bool {
        self.battery_percentage() >= 0
    }

    pub fn set_name(&self, name: &str) {
        if *self.imp().name.borrow() != name {
            self.imp().name.replace(name.to_string());
            self.notify("name");
        }
    }

    pub fn set_alias(&self, alias: &str) {
        if *self.imp().alias.borrow() != alias {
            self.imp().alias.replace(alias.to_string());
            self.notify("alias");
        }
    }

    pub fn set_icon(&self, icon: &str) {
        if *self.imp().icon.borrow() != icon {
            self.imp().icon.replace(icon.to_string());
            self.notify("icon");
        }
    }

    pub fn set_paired(&self, paired: bool) {
        if self.imp().paired.get() != paired {
            self.imp().paired.set(paired);
            self.notify("paired");
        }
    }

    pub fn set_trusted(&self, trusted: bool) {
        if self.imp().trusted.get() != trusted {
            self.imp().trusted.set(trusted);
            self.notify("trusted");
        }
    }

    pub fn set_connected(&self, connected: bool) {
        if self.imp().connected.get() != connected {
            self.imp().connected.set(connected);
            self.notify("connected");
        }
    }

    pub fn set_connecting(&self, connecting: bool) {
        if self.imp().connecting.get() != connecting {
            self.imp().connecting.set(connecting);
            self.notify("connecting");
        }
    }

    pub fn disconnecting(&self) -> bool {
        self.imp().disconnecting.get()
    }

    pub fn set_disconnecting(&self, disconnecting: bool) {
        if self.imp().disconnecting.get() != disconnecting {
            self.imp().disconnecting.set(disconnecting);
            self.notify("disconnecting");
        }
    }

    pub fn removing(&self) -> bool {
        self.imp().removing.get()
    }

    pub fn set_removing(&self, removing: bool) {
        if self.imp().removing.get() != removing {
            self.imp().removing.set(removing);
            self.notify("removing");
        }
    }

    /// Derive canonical state from BlueZ properties + local operation flags.
    /// Priority: local operations > BlueZ state.
    pub fn state(&self) -> BtDeviceState {
        if self.removing() {
            return BtDeviceState::Removing;
        }
        if self.disconnecting() {
            return BtDeviceState::Disconnecting;
        }
        if self.connecting() {
            return if self.paired() {
                BtDeviceState::Connecting
            } else {
                BtDeviceState::Pairing
            };
        }
        if self.connected() {
            return BtDeviceState::Connected;
        }
        if self.paired() {
            return BtDeviceState::Paired;
        }
        BtDeviceState::Discovered
    }

    pub fn set_battery_percentage(&self, percentage: i32) {
        if self.imp().battery_percentage.get() != percentage {
            self.imp().battery_percentage.set(percentage);
            self.notify("battery-percentage");
        }
    }

    pub fn rssi(&self) -> i16 {
        self.imp().rssi.get()
    }

    pub fn set_rssi(&self, rssi: i16) {
        if self.imp().rssi.get() != rssi {
            self.imp().rssi.set(rssi);
            self.notify("rssi");
        }
    }

    /// Returns RSSI signal strength icon, or None if no RSSI data
    pub fn rssi_icon(&self) -> Option<&'static str> {
        let rssi = self.rssi();
        if rssi == i16::MIN {
            return None;
        }
        Some(match rssi {
            -50..=0 => "network-wireless-signal-excellent-symbolic",
            -70..=-51 => "network-wireless-signal-good-symbolic",
            -80..=-71 => "network-wireless-signal-ok-symbolic",
            _ => "network-wireless-signal-weak-symbolic",
        })
    }

    /// Returns appropriate icon name based on device type
    pub fn device_icon(&self) -> &'static str {
        let icon = self.imp().icon.borrow();
        match icon.as_str() {
            "audio-card" | "audio-headphones" | "audio-headset" => "audio-headphones-symbolic",
            "input-keyboard" => "input-keyboard-symbolic",
            "input-mouse" => "input-mouse-symbolic",
            "input-gaming" => "input-gaming-symbolic",
            "phone" => "phone-symbolic",
            "computer" => "computer-symbolic",
            _ => "bluetooth-symbolic",
        }
    }

    /// Returns a human-readable device type string based on icon
    pub fn device_type_name(&self) -> &'static str {
        let icon = self.imp().icon.borrow();
        match icon.as_str() {
            "audio-card" => "Audio",
            "audio-headphones" => "Headphones",
            "audio-headset" => "Headset",
            "input-keyboard" => "Keyboard",
            "input-mouse" => "Mouse",
            "input-gaming" => "Gamepad",
            "phone" => "Phone",
            "computer" => "Computer",
            _ => "Bluetooth Device",
        }
    }

    /// Returns battery icon based on percentage
    pub fn battery_icon(&self) -> &'static str {
        match self.battery_percentage() {
            90..=100 => "battery-level-100-symbolic",
            70..=89 => "battery-level-80-symbolic",
            50..=69 => "battery-level-60-symbolic",
            30..=49 => "battery-level-40-symbolic",
            10..=29 => "battery-level-20-symbolic",
            0..=9 => "battery-level-0-symbolic",
            _ => "battery-missing-symbolic",
        }
    }
}
