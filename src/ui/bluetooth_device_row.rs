use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::OnceCell;
use std::sync::OnceLock;

use crate::backend::bluetooth::{BtDevice, BtDeviceState};

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/neoden/wlcontrol/ui/bluetooth-device-row.ui")]
    pub struct BluetoothDeviceRow {
        #[template_child]
        pub device_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub battery_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub battery_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub battery_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub rssi_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub connected_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub connecting_indicator: TemplateChild<gtk::Image>,
        #[template_child]
        pub settings_button: TemplateChild<gtk::Button>,

        pub device: OnceCell<BtDevice>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BluetoothDeviceRow {
        const NAME: &'static str = "BluetoothDeviceRow";
        type Type = super::BluetoothDeviceRow;
        type ParentType = adw::ActionRow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for BluetoothDeviceRow {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![glib::subclass::Signal::builder("settings-clicked").build()]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();

            self.settings_button.connect_clicked(glib::clone!(
                #[weak(rename_to = row)]
                self,
                move |_| {
                    row.obj().emit_by_name::<()>("settings-clicked", &[]);
                }
            ));
        }
    }
    impl WidgetImpl for BluetoothDeviceRow {}
    impl ListBoxRowImpl for BluetoothDeviceRow {}
    impl PreferencesRowImpl for BluetoothDeviceRow {}
    impl ActionRowImpl for BluetoothDeviceRow {}
}

glib::wrapper! {
    pub struct BluetoothDeviceRow(ObjectSubclass<imp::BluetoothDeviceRow>)
        @extends gtk::Widget, gtk::ListBoxRow, adw::PreferencesRow, adw::ActionRow,
        @implements gtk::Accessible, gtk::Actionable, gtk::Buildable, gtk::ConstraintTarget;
}

impl BluetoothDeviceRow {
    pub fn new(device: &BtDevice) -> Self {
        let row: Self = glib::Object::new();
        let imp = row.imp();

        imp.device.set(device.clone()).unwrap();

        // Set subtitle for paired devices
        if device.paired() {
            row.set_subtitle(&device.address());
        }

        // Set device icon
        imp.device_icon.set_icon_name(Some(device.device_icon()));

        // Initial UI sync
        row.sync_ui_to_state();

        // Single handler for ALL property changes
        device.connect_notify_local(
            None,
            glib::clone!(
                #[weak]
                row,
                move |_, _| {
                    row.sync_ui_to_state();
                }
            ),
        );

        row
    }

    pub fn device(&self) -> &BtDevice {
        self.imp().device.get().unwrap()
    }

    /// Derive all UI widget states from the device's canonical state.
    /// Exhaustive match ensures adding a new state is a compile error
    /// until every UI element is accounted for.
    fn sync_ui_to_state(&self) {
        let device = self.device();
        let state = device.state();
        let imp = self.imp();

        // Orthogonal to state: always update name and battery
        self.set_title(&device.display_name());
        self.update_battery_display();

        match state {
            BtDeviceState::Discovered => {
                imp.connected_icon.set_visible(false);
                imp.connecting_indicator.set_visible(false);
                imp.settings_button.set_visible(false);
                self.set_activatable(true);
                // Show RSSI for discovered devices
                if let Some(icon_name) = device.rssi_icon() {
                    imp.rssi_icon.set_icon_name(Some(icon_name));
                    imp.rssi_icon.set_visible(true);
                } else {
                    imp.rssi_icon.set_visible(false);
                }
            }
            BtDeviceState::Pairing => {
                imp.connected_icon.set_visible(false);
                imp.connecting_indicator.set_visible(true);
                imp.settings_button.set_visible(false);
                imp.rssi_icon.set_visible(false);
                self.set_activatable(false);
            }
            BtDeviceState::Paired => {
                imp.connected_icon.set_visible(false);
                imp.connecting_indicator.set_visible(false);
                imp.settings_button.set_visible(true);
                imp.rssi_icon.set_visible(false);
                self.set_activatable(true);
            }
            BtDeviceState::Connecting => {
                imp.connected_icon.set_visible(false);
                imp.connecting_indicator.set_visible(true);
                imp.settings_button.set_visible(false);
                imp.rssi_icon.set_visible(false);
                self.set_activatable(false);
            }
            BtDeviceState::Connected => {
                imp.connected_icon.set_visible(true);
                imp.connecting_indicator.set_visible(false);
                imp.settings_button.set_visible(true);
                imp.rssi_icon.set_visible(false);
                self.set_activatable(true);
            }
            BtDeviceState::Disconnecting => {
                imp.connected_icon.set_visible(false);
                imp.connecting_indicator.set_visible(true);
                imp.settings_button.set_visible(false);
                imp.rssi_icon.set_visible(false);
                self.set_activatable(false);
            }
            BtDeviceState::Removing => {
                imp.connected_icon.set_visible(false);
                imp.connecting_indicator.set_visible(true);
                imp.settings_button.set_visible(false);
                imp.rssi_icon.set_visible(false);
                self.set_activatable(false);
            }
        }
    }

    fn update_battery_display(&self) {
        let device = self.device();
        let imp = self.imp();

        if device.has_battery() {
            imp.battery_box.set_visible(true);
            imp.battery_icon.set_icon_name(Some(device.battery_icon()));
            imp.battery_label
                .set_label(&format!("{}%", device.battery_percentage()));
        } else {
            imp.battery_box.set_visible(false);
        }
    }
}
