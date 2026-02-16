use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::{OnceCell, RefCell};
use std::rc::Rc;

use crate::backend::bluetooth::{BtDevice, BtDeviceState};
use crate::backend::WlcontrolManager;
use crate::ui::BluetoothDeviceRow;

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/neoden/wlcontrol/ui/bluetooth-page.ui")]
    pub struct BluetoothPage {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub adapter_switch: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub discoverable_switch: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub connected_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub connected_listbox: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub paired_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub paired_listbox: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub scan_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub discovered_listbox: TemplateChild<gtk::ListBox>,

        pub manager: OnceCell<WlcontrolManager>,

        // Filter models
        pub connected_filter: OnceCell<gtk::FilterListModel>,
        pub paired_filter: OnceCell<gtk::FilterListModel>,
        pub discovered_filter: OnceCell<gtk::FilterListModel>,

        // Custom filters (for invalidation on property changes)
        pub connected_custom_filter: OnceCell<gtk::CustomFilter>,
        pub paired_custom_filter: OnceCell<gtk::CustomFilter>,
        pub discovered_custom_filter: OnceCell<gtk::CustomFilter>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BluetoothPage {
        const NAME: &'static str = "BluetoothPage";
        type Type = super::BluetoothPage;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            BluetoothDeviceRow::ensure_type();
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[gtk::template_callbacks]
    impl BluetoothPage {
        #[template_callback]
        fn on_scan_clicked(&self, _button: &gtk::Button) {
            if let Some(manager) = self.manager.get() {
                if manager.bt_discovering() {
                    manager.request_bt_stop_scan();
                } else {
                    manager.request_bt_scan();
                }
            }
        }
    }

    impl ObjectImpl for BluetoothPage {
        fn constructed(&self) {
            self.parent_constructed();

            self.connected_listbox
                .set_placeholder(Some(&Self::create_placeholder("No connected devices")));
            self.paired_listbox
                .set_placeholder(Some(&Self::create_placeholder("No paired devices")));
            self.discovered_listbox
                .set_placeholder(Some(&Self::create_placeholder("No devices found")));
        }
    }

    impl BluetoothPage {
        fn create_placeholder(text: &str) -> gtk::Label {
            let label = gtk::Label::new(Some(text));
            label.add_css_class("dim-label");
            label.set_margin_top(24);
            label.set_margin_bottom(24);
            label
        }
    }

    impl WidgetImpl for BluetoothPage {}
    impl BinImpl for BluetoothPage {}
}

glib::wrapper! {
    pub struct BluetoothPage(ObjectSubclass<imp::BluetoothPage>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl BluetoothPage {
    pub fn set_manager(&self, manager: &WlcontrolManager) {
        let imp = self.imp();
        imp.manager.set(manager.clone()).unwrap();

        // Spin the refresh icon while discovering
        let scan_button = imp.scan_button.clone();
        manager.connect_notify_local(
            Some("bt-discovering"),
            move |manager, _| {
                if manager.bt_discovering() {
                    scan_button.add_css_class("scanning");
                } else {
                    scan_button.remove_css_class("scanning");
                }
            },
        );

        // Bind adapter power state
        manager
            .bind_property("bt-powered", &*imp.adapter_switch, "active")
            .sync_create()
            .bidirectional()
            .build();

        // Disable controls when BT is off
        manager
            .bind_property("bt-powered", &*imp.discoverable_switch, "sensitive")
            .sync_create()
            .build();
        manager
            .bind_property("bt-powered", &*imp.scan_button, "sensitive")
            .sync_create()
            .build();

        // Bind discoverable state
        manager
            .bind_property("bt-discoverable", &*imp.discoverable_switch, "active")
            .sync_create()
            .bidirectional()
            .build();

        // Create filtered models for different device states
        let devices = manager.bt_devices();

        // Connected devices filter
        let connected_filter =
            gtk::CustomFilter::new(|item| item.downcast_ref::<BtDevice>().unwrap().connected());
        let connected_model =
            gtk::FilterListModel::new(Some(devices.clone()), Some(connected_filter.clone()));
        imp.connected_filter.set(connected_model.clone()).unwrap();
        imp.connected_custom_filter
            .set(connected_filter.clone())
            .unwrap();

        // Paired (but not connected) devices filter
        let paired_filter = gtk::CustomFilter::new(|item| {
            let device = item.downcast_ref::<BtDevice>().unwrap();
            device.paired() && !device.connected()
        });
        let paired_model =
            gtk::FilterListModel::new(Some(devices.clone()), Some(paired_filter.clone()));
        imp.paired_filter.set(paired_model.clone()).unwrap();
        imp.paired_custom_filter
            .set(paired_filter.clone())
            .unwrap();

        // Discovered (not paired) devices filter
        let discovered_filter =
            gtk::CustomFilter::new(|item| !item.downcast_ref::<BtDevice>().unwrap().paired());
        let discovered_model =
            gtk::FilterListModel::new(Some(devices.clone()), Some(discovered_filter.clone()));
        imp.discovered_filter.set(discovered_model.clone()).unwrap();
        imp.discovered_custom_filter
            .set(discovered_filter.clone())
            .unwrap();

        // Invalidate filters when device properties change
        manager.connect_closure(
            "bt-device-updated",
            false,
            glib::closure_local!(
                #[weak]
                connected_filter,
                #[weak]
                paired_filter,
                #[weak]
                discovered_filter,
                move |_manager: WlcontrolManager| {
                    connected_filter.changed(gtk::FilterChange::Different);
                    paired_filter.changed(gtk::FilterChange::Different);
                    discovered_filter.changed(gtk::FilterChange::Different);
                }
            ),
        );

        // Bind visibility of groups to whether they have items
        connected_model.connect_items_changed(glib::clone!(
            #[weak(rename_to = group)]
            imp.connected_group,
            move |model, _, _, _| {
                group.set_visible(model.n_items() > 0);
            }
        ));

        paired_model.connect_items_changed(glib::clone!(
            #[weak(rename_to = group)]
            imp.paired_group,
            move |model, _, _, _| {
                group.set_visible(model.n_items() > 0);
            }
        ));

        // Bind device lists
        Self::bind_device_list(&imp.connected_listbox, &connected_model, manager);
        Self::bind_device_list(&imp.paired_listbox, &paired_model, manager);
        Self::bind_device_list(&imp.discovered_listbox, &discovered_model, manager);

        // Handle errors
        manager.connect_closure(
            "error",
            false,
            glib::closure_local!(
                #[weak(rename_to = page)]
                self,
                move |_manager: WlcontrolManager, message: String| {
                    page.show_toast(&message);
                }
            ),
        );

        // Handle all BT pairing interactions
        let page = self.clone();
        let pairing_dialog: Rc<RefCell<Option<adw::AlertDialog>>> = Rc::new(RefCell::new(None));
        let pd = pairing_dialog.clone();
        manager.connect_closure(
            "bt-pairing",
            false,
            glib::closure_local!(
                #[watch]
                page,
                move |manager: WlcontrolManager, kind: String, _address: String, code: String| {
                    let (heading, body, responses, needs_input) = match kind.as_str() {
                        "confirm-passkey" => (
                            "Bluetooth Pairing",
                            format!("Confirm that the other device is showing this code:\n\n<big><b>{}</b></big>", code),
                            vec![("cancel", "Cancel", false), ("confirm", "Confirm", true)],
                            None,
                        ),
                        "request-pin" => (
                            "Bluetooth Pairing",
                            "Enter PIN code for the device".to_string(),
                            vec![("cancel", "Cancel", false), ("confirm", "Pair", true)],
                            Some("PIN Code"),
                        ),
                        "request-passkey" => (
                            "Bluetooth Pairing",
                            "Enter the passkey shown on the device (0\u{2013}999999)".to_string(),
                            vec![("cancel", "Cancel", false), ("confirm", "Pair", true)],
                            Some("Passkey"),
                        ),
                        "display-passkey" => (
                            "Bluetooth Pairing",
                            format!("Enter this passkey on the device:\n\n<big><b>{}</b></big>", code),
                            vec![("ok", "OK", true)],
                            None,
                        ),
                        "display-pin" => (
                            "Bluetooth Pairing",
                            format!("Enter this PIN on the device:\n\n<big><b>{}</b></big>",
                                glib::markup_escape_text(&code)),
                            vec![("ok", "OK", true)],
                            None,
                        ),
                        "authorize" => (
                            "Bluetooth Pairing",
                            "Allow this device to connect?".to_string(),
                            vec![("cancel", "Deny", false), ("allow", "Allow", true)],
                            None,
                        ),
                        _ => return,
                    };

                    let dialog = adw::AlertDialog::builder()
                        .heading(heading)
                        .body(&body)
                        .body_use_markup(true)
                        .close_response(responses[0].0)
                        .build();
                    for &(id, label, suggested) in &responses {
                        dialog.add_response(id, label);
                        if suggested {
                            dialog.set_response_appearance(id, adw::ResponseAppearance::Suggested);
                            dialog.set_default_response(Some(id));
                        }
                    }

                    let entry = if let Some(title) = needs_input {
                        let entry = adw::EntryRow::builder().title(title).build();
                        let group = adw::PreferencesGroup::new();
                        group.add(&entry);
                        dialog.set_extra_child(Some(&group));
                        dialog.set_response_enabled("confirm", false);
                        let kind2 = kind.clone();
                        entry.connect_changed(glib::clone!(
                            #[weak]
                            dialog,
                            move |entry| {
                                let valid = match kind2.as_str() {
                                    "request-passkey" => {
                                        entry.text().parse::<u32>().is_ok() && entry.text().len() <= 6
                                    }
                                    _ => !entry.text().is_empty(),
                                };
                                dialog.set_response_enabled("confirm", valid);
                            }
                        ));
                        Some(entry)
                    } else {
                        None
                    };

                    pd.replace(Some(dialog.clone()));
                    let pd2 = pd.clone();

                    glib::spawn_future_local(glib::clone!(
                        #[weak]
                        manager,
                        #[weak]
                        page,
                        async move {
                            let response = dialog.choose_future(Some(&page)).await;
                            pd2.replace(None);
                            match kind.as_str() {
                                "confirm-passkey" => {
                                    manager.send_bt_pairing_response(response == "confirm");
                                }
                                "authorize" => {
                                    manager.send_bt_pairing_response(response == "allow");
                                }
                                "request-pin" => {
                                    let pin = if response == "confirm" {
                                        entry.map(|e| e.text().to_string())
                                    } else {
                                        None
                                    };
                                    manager.send_bt_pairing_pin(pin);
                                }
                                "request-passkey" => {
                                    let passkey = if response == "confirm" {
                                        entry.and_then(|e| e.text().parse::<u32>().ok())
                                    } else {
                                        None
                                    };
                                    manager.send_bt_pairing_passkey(passkey);
                                }
                                _ => {} // display-only, no response needed
                            }
                        }
                    ));
                }
            ),
        );

        // Close pairing dialog when BT is turned off
        manager.connect_notify_local(
            Some("bt-powered"),
            move |manager, _| {
                if !manager.bt_powered() {
                    if let Some(dialog) = pairing_dialog.take() {
                        dialog.force_close();
                    }
                }
            },
        );
    }

    fn bind_device_list(
        listbox: &gtk::ListBox,
        model: &gtk::FilterListModel,
        manager: &WlcontrolManager,
    ) {
        listbox.bind_model(
            Some(model),
            glib::clone!(
                #[weak]
                manager,
                #[upgrade_or_panic]
                move |item| {
                    let device = item.downcast_ref::<BtDevice>().unwrap();
                    let row = BluetoothDeviceRow::new(device);

                    row.connect_activated(glib::clone!(
                        #[weak]
                        manager,
                        #[weak]
                        device,
                        move |_| {
                            match device.state() {
                                BtDeviceState::Discovered => manager.request_bt_pair(&device.path()),
                                BtDeviceState::Paired => manager.request_bt_connect(&device.path()),
                                BtDeviceState::Connected => manager.request_bt_disconnect(&device.path()),
                                // In-progress states: ignore clicks
                                BtDeviceState::Pairing
                                | BtDeviceState::Connecting
                                | BtDeviceState::Disconnecting
                                | BtDeviceState::Removing => {}
                            }
                        }
                    ));

                    row.connect_closure(
                        "settings-clicked",
                        false,
                        glib::closure_local!(
                            #[weak]
                            manager,
                            #[weak]
                            device,
                            move |row: BluetoothDeviceRow| {
                                BluetoothPage::show_device_settings(&row, &manager, &device);
                            }
                        ),
                    );

                    row.upcast()
                }
            ),
        );
    }

    fn show_device_settings(row: &BluetoothDeviceRow, manager: &WlcontrolManager, device: &BtDevice) {
        let display_name = device.display_name();
        let address = device.address();
        let device_type = device.device_type_name();

        let dialog = adw::AlertDialog::builder()
            .heading(&display_name)
            .body(&format!("{}\n{}", address, device_type))
            .close_response("close")
            .default_response("close")
            .build();

        // Extra child: preferences group with alias entry and trusted toggle
        let alias_entry = adw::EntryRow::builder()
            .title("Name")
            .text(&display_name)
            .build();

        let trusted_switch = adw::SwitchRow::builder()
            .title("Auto-connect")
            .active(device.trusted())
            .build();

        // Send trusted change immediately
        trusted_switch.connect_active_notify(glib::clone!(
            #[weak]
            manager,
            #[weak]
            device,
            move |switch| {
                manager.request_bt_set_trusted(&device.path(), switch.is_active());
            }
        ));

        let group = adw::PreferencesGroup::new();
        group.add(&alias_entry);
        group.add(&trusted_switch);
        dialog.set_extra_child(Some(&group));

        dialog.add_response("forget", "Forget Device");
        dialog.set_response_appearance("forget", adw::ResponseAppearance::Destructive);
        dialog.add_response("close", "Close");
        dialog.set_response_appearance("close", adw::ResponseAppearance::Suggested);

        let original_alias = display_name.clone();

        glib::spawn_future_local(glib::clone!(
            #[weak]
            manager,
            #[weak]
            device,
            #[weak]
            row,
            async move {
                let response = dialog.choose_future(Some(&row)).await;
                match response.as_str() {
                    "forget" => {
                        manager.request_bt_remove(&device.path());
                    }
                    _ => {
                        // Check if alias changed on close
                        let new_alias = alias_entry.text().to_string();
                        if !new_alias.is_empty() && new_alias != original_alias {
                            manager.request_bt_set_alias(&device.path(), &new_alias);
                        }
                    }
                }
            }
        ));
    }

    pub fn show_toast(&self, message: &str) {
        let toast = adw::Toast::new(message);
        self.imp().toast_overlay.add_toast(toast);
    }
}
