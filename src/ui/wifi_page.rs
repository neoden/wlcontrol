use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use std::cell::OnceCell;

use crate::backend::wifi::{WifiNetwork, WifiNetworkState};
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
        #[template_child]
        pub saved_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub saved_toggle: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub saved_listbox: TemplateChild<gtk::ListBox>,

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

        // Spinning animation and disable scan button while scanning
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
                scan_button.set_sensitive(!scanning);
            },
        );

        // Bind adapter power state
        manager
            .bind_property("wifi-powered", &*imp.adapter_switch, "active")
            .sync_create()
            .bidirectional()
            .build();

        // Bind main network list (all scan results)
        imp.networks_listbox.bind_model(
            Some(&manager.wifi_networks()),
            glib::clone!(
                #[weak]
                manager,
                #[upgrade_or_panic]
                move |item| {
                    let network = item.downcast_ref::<WifiNetwork>().unwrap();
                    Self::create_network_row(network, &manager, false).upcast()
                }
            ),
        );

        // Bind saved networks list (known networks not in range)
        imp.saved_listbox.bind_model(
            Some(&manager.saved_networks()),
            glib::clone!(
                #[weak]
                manager,
                #[upgrade_or_panic]
                move |item| {
                    let network = item.downcast_ref::<WifiNetwork>().unwrap();
                    Self::create_network_row(network, &manager, true).upcast()
                }
            ),
        );

        // Show/hide saved group based on item count
        let saved_group = imp.saved_group.clone();
        let update_saved_visibility = move |store: &gio::ListStore| {
            saved_group.set_visible(store.n_items() > 0);
        };
        let saved_store = manager.saved_networks();
        update_saved_visibility(&saved_store);
        saved_store.connect_items_changed(move |store, _, _, _| {
            update_saved_visibility(store);
        });

        // Toggle button expands/collapses saved listbox
        let saved_listbox = imp.saved_listbox.clone();
        imp.saved_toggle.connect_toggled(move |button| {
            let expanded = button.is_active();
            saved_listbox.set_visible(expanded);
            button.set_icon_name(if expanded {
                "pan-down-symbolic"
            } else {
                "pan-end-symbolic"
            });
        });

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

        // Handle captive portal
        manager.connect_closure(
            "captive-portal",
            false,
            glib::closure_local!(
                #[weak(rename_to = page)]
                self,
                move |_manager: WlcontrolManager, url: String| {
                    page.show_toast("Login required — opening browser");
                    let launcher = gtk::UriLauncher::new(&url);
                    let window = page.root().and_downcast::<gtk::Window>();
                    launcher.launch(window.as_ref(), gio::Cancellable::NONE, |result| {
                        if let Err(e) = result {
                            tracing::error!("Failed to open browser: {}", e);
                        }
                    });
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

    fn create_network_row(
        network: &WifiNetwork,
        manager: &WlcontrolManager,
        is_saved_offline: bool,
    ) -> WifiNetworkRow {
        let row = WifiNetworkRow::new(network);
        row.connect_activated(glib::clone!(
            #[weak]
            manager,
            #[weak]
            network,
            move |_| {
                match network.state() {
                    WifiNetworkState::Connected => {
                        manager.request_wifi_disconnect();
                    }
                    WifiNetworkState::Available | WifiNetworkState::Saved => {
                        manager.request_wifi_connect(&network.path());
                    }
                    // SavedOffline and in-progress states: ignore clicks
                    WifiNetworkState::SavedOffline
                    | WifiNetworkState::Connecting
                    | WifiNetworkState::Disconnecting
                    | WifiNetworkState::Forgetting => {}
                }
            }
        ));
        row.setup_actions(&manager, network, is_saved_offline);
        row
    }

    pub fn show_toast(&self, message: &str) {
        let toast = adw::Toast::new(message);
        self.imp().toast_overlay.add_toast(toast);
    }
}
