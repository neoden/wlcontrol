use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::OnceCell;

use crate::backend::bluetooth::BtDevice;
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
        pub scan_spinner: TemplateChild<gtk::Spinner>,
        #[template_child]
        pub scan_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub discovered_listbox: TemplateChild<gtk::ListBox>,

        pub manager: OnceCell<WlcontrolManager>,

        // Filter models
        pub connected_filter: OnceCell<gtk::FilterListModel>,
        pub paired_filter: OnceCell<gtk::FilterListModel>,
        pub discovered_filter: OnceCell<gtk::FilterListModel>,
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
                manager.request_bt_scan();
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

        // Bind discovering state to spinner
        manager
            .bind_property("bt-discovering", &*imp.scan_spinner, "visible")
            .sync_create()
            .build();

        manager
            .bind_property("bt-discovering", &*imp.scan_spinner, "spinning")
            .sync_create()
            .build();

        // Hide scan button while discovering
        manager
            .bind_property("bt-discovering", &*imp.scan_button, "visible")
            .sync_create()
            .invert_boolean()
            .build();

        // Bind adapter power state
        manager
            .bind_property("bt-powered", &*imp.adapter_switch, "active")
            .sync_create()
            .bidirectional()
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
        let connected_model = gtk::FilterListModel::new(Some(devices.clone()), Some(connected_filter));
        imp.connected_filter.set(connected_model.clone()).unwrap();

        // Paired (but not connected) devices filter
        let paired_filter = gtk::CustomFilter::new(|item| {
            let device = item.downcast_ref::<BtDevice>().unwrap();
            device.paired() && !device.connected()
        });
        let paired_model = gtk::FilterListModel::new(Some(devices.clone()), Some(paired_filter));
        imp.paired_filter.set(paired_model.clone()).unwrap();

        // Discovered (not paired) devices filter
        let discovered_filter =
            gtk::CustomFilter::new(|item| !item.downcast_ref::<BtDevice>().unwrap().paired());
        let discovered_model = gtk::FilterListModel::new(Some(devices.clone()), Some(discovered_filter));
        imp.discovered_filter.set(discovered_model.clone()).unwrap();

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
                            if device.connected() {
                                manager.request_bt_disconnect(&device.path());
                            } else if device.paired() {
                                manager.request_bt_connect(&device.path());
                            } else {
                                manager.request_bt_pair(&device.path());
                            }
                        }
                    ));

                    row.upcast()
                }
            ),
        );
    }

    pub fn show_toast(&self, message: &str) {
        let toast = adw::Toast::new(message);
        self.imp().toast_overlay.add_toast(toast);
    }
}
