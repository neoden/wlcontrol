use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use std::cell::OnceCell;

use crate::backend::bluetooth::{BtDevice, BtDeviceState};
use crate::backend::WlcontrolManager;

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
        pub menu_button: TemplateChild<gtk::MenuButton>,

        pub device: OnceCell<BtDevice>,
        pub action_group: OnceCell<gio::SimpleActionGroup>,
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
        fn constructed(&self) {
            self.parent_constructed();
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

    pub fn setup_actions(&self, manager: &WlcontrolManager, device: &BtDevice) {
        let group = gio::SimpleActionGroup::new();

        // rename
        let rename = gio::SimpleAction::new("rename", None);
        rename.connect_activate(glib::clone!(
            #[weak(rename_to = row)]
            self,
            #[weak]
            manager,
            #[weak]
            device,
            move |_, _| {
                Self::show_rename_dialog(&row, &manager, &device);
            }
        ));
        group.add_action(&rename);

        // auto-connect (stateful toggle)
        let auto_connect = gio::SimpleAction::new_stateful(
            "auto-connect",
            None,
            &device.trusted().to_variant(),
        );
        auto_connect.connect_change_state(glib::clone!(
            #[weak]
            manager,
            #[weak]
            device,
            move |action, value| {
                if let Some(trusted) = value.and_then(|v| v.get::<bool>()) {
                    action.set_state(&trusted.to_variant());
                    manager.request_bt_set_trusted(&device.path(), trusted);
                }
            }
        ));
        group.add_action(&auto_connect);

        // Keep auto-connect state in sync with device property
        device.connect_notify_local(
            Some("trusted"),
            glib::clone!(
                #[weak]
                auto_connect,
                move |device, _| {
                    auto_connect.set_state(&device.trusted().to_variant());
                }
            ),
        );

        // copy-address
        let copy_address = gio::SimpleAction::new("copy-address", None);
        copy_address.connect_activate(glib::clone!(
            #[weak(rename_to = row)]
            self,
            #[weak]
            device,
            move |_, _| {
                let clipboard = row.display().clipboard();
                clipboard.set_text(&device.address());
            }
        ));
        group.add_action(&copy_address);

        // forget
        let forget = gio::SimpleAction::new("forget", None);
        forget.connect_activate(glib::clone!(
            #[weak(rename_to = row)]
            self,
            #[weak]
            manager,
            #[weak]
            device,
            move |_, _| {
                Self::show_forget_dialog(&row, &manager, &device);
            }
        ));
        group.add_action(&forget);

        self.insert_action_group("row", Some(&group));
        self.imp().action_group.set(group).unwrap();
    }

    fn show_rename_dialog(row: &BluetoothDeviceRow, manager: &WlcontrolManager, device: &BtDevice) {
        let dialog = adw::AlertDialog::builder()
            .heading("Rename Device")
            .build();

        dialog.add_response("cancel", "Cancel");
        dialog.add_response("apply", "Apply");
        dialog.set_response_appearance("apply", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("apply"));
        dialog.set_close_response("cancel");

        let entry = adw::EntryRow::builder()
            .title("Name")
            .text(&device.display_name())
            .build();

        let group = adw::PreferencesGroup::new();
        group.add(&entry);
        dialog.set_extra_child(Some(&group));

        glib::spawn_future_local(glib::clone!(
            #[weak]
            manager,
            #[weak]
            device,
            #[weak]
            row,
            async move {
                let response = dialog.choose_future(Some(&row)).await;
                if response == "apply" {
                    let new_name = entry.text().to_string();
                    if !new_name.is_empty() && new_name != device.display_name() {
                        manager.request_bt_set_alias(&device.path(), &new_name);
                    }
                }
            }
        ));
    }

    fn show_forget_dialog(row: &BluetoothDeviceRow, manager: &WlcontrolManager, device: &BtDevice) {
        let dialog = adw::AlertDialog::builder()
            .heading("Forget Device?")
            .body(&format!(
                "\"{}\" will be removed and you will need to pair again.",
                device.display_name()
            ))
            .build();

        dialog.add_response("cancel", "Cancel");
        dialog.add_response("forget", "Forget");
        dialog.set_response_appearance("forget", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        glib::spawn_future_local(glib::clone!(
            #[weak]
            manager,
            #[weak]
            device,
            #[weak]
            row,
            async move {
                let response = dialog.choose_future(Some(&row)).await;
                if response == "forget" {
                    manager.request_bt_remove(&device.path());
                }
            }
        ));
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

        // Busy states pulse the whole row
        let busy = matches!(
            state,
            BtDeviceState::Pairing
                | BtDeviceState::Connecting
                | BtDeviceState::Disconnecting
                | BtDeviceState::Removing
        );
        if busy {
            self.add_css_class("bt-busy");
        } else {
            self.remove_css_class("bt-busy");
        }

        match state {
            BtDeviceState::Discovered => {
                imp.connected_icon.set_visible(false);
                imp.menu_button.set_visible(false);
                self.set_activatable(true);
                if let Some(icon_name) = device.rssi_icon() {
                    imp.rssi_icon.set_icon_name(Some(icon_name));
                    imp.rssi_icon.set_visible(true);
                } else {
                    imp.rssi_icon.set_visible(false);
                }
            }
            BtDeviceState::Pairing | BtDeviceState::Connecting => {
                imp.connected_icon.set_visible(false);
                imp.menu_button.set_visible(false);
                imp.rssi_icon.set_visible(false);
                self.set_activatable(false);
            }
            BtDeviceState::Paired => {
                imp.connected_icon.set_visible(false);
                imp.menu_button.set_visible(true);
                imp.rssi_icon.set_visible(false);
                self.set_activatable(true);
            }
            BtDeviceState::Connected => {
                imp.connected_icon.set_visible(true);
                imp.menu_button.set_visible(true);
                imp.rssi_icon.set_visible(false);
                self.set_activatable(true);
            }
            BtDeviceState::Disconnecting | BtDeviceState::Removing => {
                imp.connected_icon.set_visible(false);
                imp.menu_button.set_visible(false);
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
