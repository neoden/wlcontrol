use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::{Cell, OnceCell};

use crate::backend::wifi::WifiNetwork;

mod imp {
    use super::*;
    use std::sync::OnceLock;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/neoden/wlcontrol/ui/wifi-network-row.ui")]
    pub struct WifiNetworkRow {
        #[template_child]
        pub signal_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub security_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub connected_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub connecting_indicator: TemplateChild<gtk::Image>,
        #[template_child]
        pub forget_button: TemplateChild<gtk::Button>,

        pub network: OnceCell<WifiNetwork>,
        pub saved_mode: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WifiNetworkRow {
        const NAME: &'static str = "WifiNetworkRow";
        type Type = super::WifiNetworkRow;
        type ParentType = adw::ActionRow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for WifiNetworkRow {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: OnceLock<Vec<glib::subclass::Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![glib::subclass::Signal::builder("forget-clicked").build()]
            })
        }

        fn constructed(&self) {
            self.parent_constructed();

            // Connect forget button click to signal
            self.forget_button.connect_clicked(glib::clone!(
                #[weak(rename_to = row)]
                self,
                move |_| {
                    row.obj().emit_by_name::<()>("forget-clicked", &[]);
                }
            ));
        }
    }
    impl WidgetImpl for WifiNetworkRow {}
    impl ListBoxRowImpl for WifiNetworkRow {}
    impl PreferencesRowImpl for WifiNetworkRow {}
    impl ActionRowImpl for WifiNetworkRow {}
}

glib::wrapper! {
    pub struct WifiNetworkRow(ObjectSubclass<imp::WifiNetworkRow>)
        @extends gtk::Widget, gtk::ListBoxRow, adw::PreferencesRow, adw::ActionRow,
        @implements gtk::Accessible, gtk::Actionable, gtk::Buildable, gtk::ConstraintTarget;
}

impl WifiNetworkRow {
    pub fn new(network: &WifiNetwork) -> Self {
        let row: Self = glib::Object::new();
        let imp = row.imp();

        imp.network.set(network.clone()).unwrap();

        // Set title and subtitle
        row.set_title(&network.name());
        if network.known() {
            row.set_subtitle("Saved");
            imp.forget_button.set_visible(true);
        }

        // Set signal icon
        imp.signal_icon.set_icon_name(Some(network.signal_icon()));

        // Show security icon for secured networks
        imp.security_icon.set_visible(network.is_secured());

        // Show connected indicator
        imp.connected_icon.set_visible(network.connected());

        // Bind property changes
        network.connect_notify_local(
            Some("connected"),
            glib::clone!(
                #[weak]
                row,
                move |network, _| {
                    row.imp().connected_icon.set_visible(network.connected());
                }
            ),
        );

        network.connect_notify_local(
            Some("signal-strength"),
            glib::clone!(
                #[weak]
                row,
                move |network, _| {
                    row.imp()
                        .signal_icon
                        .set_icon_name(Some(network.signal_icon()));
                }
            ),
        );

        network.connect_notify_local(
            Some("connecting"),
            glib::clone!(
                #[weak]
                row,
                move |network, _| {
                    row.set_connecting(network.connecting());
                }
            ),
        );

        network.connect_notify_local(
            Some("known"),
            glib::clone!(
                #[weak]
                row,
                move |network, _| {
                    row.set_known(network.known());
                }
            ),
        );

        row
    }

    pub fn network(&self) -> &WifiNetwork {
        self.imp().network.get().unwrap()
    }

    pub fn set_saved_mode(&self, saved: bool) {
        self.imp().saved_mode.set(saved);
        if saved {
            // In saved mode, hide signal-related icons and show last connected time
            self.imp().signal_icon.set_visible(false);
            self.imp().security_icon.set_visible(false);
        }
    }

    pub fn set_connecting(&self, connecting: bool) {
        self.imp().connecting_indicator.set_visible(connecting);
        self.imp().connected_icon.set_visible(!connecting && self.network().connected());
    }

    pub fn set_known(&self, known: bool) {
        self.set_subtitle(if known { "Saved" } else { "" });
        self.imp().forget_button.set_visible(known);
    }
}
