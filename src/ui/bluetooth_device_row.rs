use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::OnceCell;

use crate::backend::bluetooth::BtDevice;

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
        pub connected_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub connecting_spinner: TemplateChild<gtk::Spinner>,
        #[template_child]
        pub action_button: TemplateChild<gtk::Button>,

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

    impl ObjectImpl for BluetoothDeviceRow {}
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

        // Set title and subtitle
        row.set_title(&device.display_name());
        if device.paired() {
            row.set_subtitle(&device.address());
        }

        // Set device icon
        imp.device_icon.set_icon_name(Some(device.device_icon()));

        // Show connected indicator
        imp.connected_icon.set_visible(device.connected());

        // Show battery info if available
        row.update_battery_display();

        // Bind property changes
        device.connect_notify_local(
            Some("connected"),
            glib::clone!(
                #[weak]
                row,
                move |device, _| {
                    row.imp().connected_icon.set_visible(device.connected());
                }
            ),
        );

        device.connect_notify_local(
            Some("name"),
            glib::clone!(
                #[weak]
                row,
                move |device, _| {
                    row.set_title(&device.display_name());
                }
            ),
        );

        device.connect_notify_local(
            Some("alias"),
            glib::clone!(
                #[weak]
                row,
                move |device, _| {
                    row.set_title(&device.display_name());
                }
            ),
        );

        device.connect_notify_local(
            Some("battery-percentage"),
            glib::clone!(
                #[weak]
                row,
                move |_, _| {
                    row.update_battery_display();
                }
            ),
        );

        row
    }

    pub fn device(&self) -> &BtDevice {
        self.imp().device.get().unwrap()
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

    pub fn set_connecting(&self, connecting: bool) {
        self.imp().connecting_spinner.set_visible(connecting);
        self.imp().connecting_spinner.set_spinning(connecting);
        self.imp()
            .connected_icon
            .set_visible(!connecting && self.device().connected());
    }
}
