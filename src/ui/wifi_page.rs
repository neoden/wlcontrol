use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use std::cell::OnceCell;

use crate::backend::wifi::WifiNetwork;
use crate::backend::WlcontrolManager;
use crate::ui::{PasswordDialog, WifiNetworkRow};

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/neoden/wlcontrol/ui/wifi-page.ui")]
    pub struct WifiPage {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub adapter_switch: TemplateChild<adw::SwitchRow>,
        #[template_child]
        pub scan_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub networks_listbox: TemplateChild<gtk::ListBox>,

        pub manager: OnceCell<WlcontrolManager>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WifiPage {
        const NAME: &'static str = "WifiPage";
        type Type = super::WifiPage;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            WifiNetworkRow::ensure_type();
            klass.bind_template();
            klass.bind_template_callbacks();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[gtk::template_callbacks]
    impl WifiPage {
        #[template_callback]
        fn on_scan_clicked(&self, _button: &gtk::Button) {
            if let Some(manager) = self.manager.get() {
                manager.request_wifi_scan();
            }
        }
    }

    impl ObjectImpl for WifiPage {
        fn constructed(&self) {
            self.parent_constructed();

            // Set up network list factory
            self.networks_listbox
                .set_placeholder(Some(&Self::create_placeholder("No networks found")));
        }
    }

    impl WifiPage {
        fn create_placeholder(text: &str) -> gtk::Label {
            let label = gtk::Label::new(Some(text));
            label.add_css_class("dim-label");
            label.set_margin_top(24);
            label.set_margin_bottom(24);
            label
        }
    }

    impl WidgetImpl for WifiPage {}
    impl BinImpl for WifiPage {}
}

glib::wrapper! {
    pub struct WifiPage(ObjectSubclass<imp::WifiPage>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl WifiPage {
    pub fn set_manager(&self, manager: &WlcontrolManager) {
        let imp = self.imp();
        imp.manager.set(manager.clone()).unwrap();

        // Add spinning animation and disable scan button while scanning
        let scan_button = imp.scan_button.clone();
        manager.connect_notify_local(
            Some("wifi-scanning"),
            move |manager, _| {
                let scanning = manager.wifi_scanning();
                if scanning {
                    scan_button.add_css_class("scanning");
                } else {
                    scan_button.remove_css_class("scanning");
                }
                // Disable button during scan to prevent multiple clicks
                scan_button.set_sensitive(!scanning);
            },
        );

        // Bind adapter power state
        manager
            .bind_property("wifi-powered", &*imp.adapter_switch, "active")
            .sync_create()
            .bidirectional()
            .build();

        // Bind network list
        imp.networks_listbox.bind_model(
            Some(&manager.wifi_networks()),
            glib::clone!(
                #[weak]
                manager,
                #[upgrade_or_panic]
                move |item| {
                    let network = item.downcast_ref::<WifiNetwork>().unwrap();
                    let row = WifiNetworkRow::new(network);
                    row.connect_activated(glib::clone!(
                        #[weak]
                        manager,
                        #[weak]
                        network,
                        move |_| {
                            if network.connected() {
                                manager.request_wifi_disconnect();
                            } else {
                                manager.request_wifi_connect(&network.path());
                            }
                        }
                    ));
                    row.connect_closure(
                        "forget-clicked",
                        false,
                        glib::closure_local!(
                            #[weak]
                            manager,
                            #[weak]
                            network,
                            move |_row: WifiNetworkRow| {
                                manager.request_wifi_forget(&network.path());
                            }
                        ),
                    );
                    row.upcast()
                }
            ),
        );

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

        // Handle passphrase requests
        let page = self.clone();
        manager.connect_closure(
            "passphrase-requested",
            false,
            glib::closure_local!(
                #[watch]
                page,
                move |manager: WlcontrolManager, _network_path: String, network_name: String| {
                    let dialog = PasswordDialog::new(&network_name);
                    glib::spawn_future_local(glib::clone!(
                        #[weak]
                        manager,
                        #[weak]
                        page,
                        async move {
                            let response = dialog.run(&page).await;
                            manager.send_passphrase_response(response);
                        }
                    ));
                }
            ),
        );
    }

    pub fn show_toast(&self, message: &str) {
        let toast = adw::Toast::new(message);
        self.imp().toast_overlay.add_toast(toast);
    }
}
