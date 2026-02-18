use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use std::cell::OnceCell;

use crate::backend::wifi::{WifiNetwork, WifiNetworkState};
use crate::backend::WlcontrolManager;

mod imp {
    use super::*;

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
        pub menu_button: TemplateChild<gtk::MenuButton>,

        pub network: OnceCell<WifiNetwork>,
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
        fn constructed(&self) {
            self.parent_constructed();
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
        row.imp().network.set(network.clone()).unwrap();

        // Initial UI sync
        row.sync_ui_to_state();

        // Single handler for ALL property changes
        network.connect_notify_local(
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

    pub fn network(&self) -> &WifiNetwork {
        self.imp().network.get().unwrap()
    }

    pub fn setup_actions(
        &self,
        manager: &WlcontrolManager,
        network: &WifiNetwork,
        is_saved_offline: bool,
    ) {
        let group = gio::SimpleActionGroup::new();

        // copy-name
        let copy_name = gio::SimpleAction::new("copy-name", None);
        copy_name.connect_activate(glib::clone!(
            #[weak(rename_to = row)]
            self,
            #[weak]
            network,
            move |_, _| {
                let clipboard = row.display().clipboard();
                clipboard.set_text(&network.name());
            }
        ));
        group.add_action(&copy_name);

        // forget
        let forget = gio::SimpleAction::new("forget", None);
        forget.connect_activate(glib::clone!(
            #[weak(rename_to = row)]
            self,
            #[weak]
            manager,
            #[weak]
            network,
            move |_, _| {
                Self::show_forget_dialog(&row, &manager, &network, is_saved_offline);
            }
        ));
        group.add_action(&forget);

        self.insert_action_group("row", Some(&group));
    }

    fn show_forget_dialog(
        row: &WifiNetworkRow,
        manager: &WlcontrolManager,
        network: &WifiNetwork,
        is_saved_offline: bool,
    ) {
        let dialog = adw::AlertDialog::builder()
            .heading("Forget Network?")
            .body(format!(
                "\"{}\" will be removed and you will need to enter the password again.",
                network.name()
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
            network,
            #[weak]
            row,
            async move {
                let response = dialog.choose_future(Some(&row)).await;
                if response == "forget" {
                    if is_saved_offline {
                        manager.request_wifi_forget_known(&network.path());
                    } else {
                        manager.request_wifi_forget(&network.path());
                    }
                }
            }
        ));
    }

    /// Derive all UI widget states from the network's canonical state.
    /// Exhaustive match ensures adding a new state is a compile error
    /// until every UI element is accounted for.
    fn sync_ui_to_state(&self) {
        let network = self.network();
        let state = network.state();
        let imp = self.imp();

        // Orthogonal to state: always update
        self.set_title(&network.name());
        imp.signal_icon.set_icon_name(Some(network.signal_icon()));
        imp.security_icon.set_visible(network.is_secured());

        // Busy states
        let busy = matches!(
            state,
            WifiNetworkState::Connecting
                | WifiNetworkState::Disconnecting
                | WifiNetworkState::Forgetting
        );
        if busy {
            self.add_css_class("wifi-busy");
        } else {
            self.remove_css_class("wifi-busy");
        }

        match state {
            WifiNetworkState::Available => {
                imp.connected_icon.set_visible(false);
                imp.menu_button.set_visible(false);
                self.set_subtitle("");
                self.set_activatable(true);
            }
            WifiNetworkState::Saved => {
                imp.connected_icon.set_visible(false);
                imp.menu_button.set_visible(true);
                imp.signal_icon.set_visible(true);
                self.set_subtitle("Saved");
                self.set_activatable(true);
            }
            WifiNetworkState::SavedOffline => {
                imp.connected_icon.set_visible(false);
                imp.menu_button.set_visible(true);
                imp.signal_icon.set_visible(false);
                self.set_subtitle("Saved");
                self.set_activatable(false);
            }
            WifiNetworkState::Connecting => {
                imp.connected_icon.set_visible(false);
                imp.menu_button.set_visible(false);
                self.set_subtitle("");
                self.set_activatable(false);
            }
            WifiNetworkState::Connected => {
                imp.connected_icon.set_visible(true);
                imp.menu_button.set_visible(true);
                self.set_subtitle("Connected");
                self.set_activatable(true);
            }
            WifiNetworkState::Disconnecting => {
                imp.connected_icon.set_visible(false);
                imp.menu_button.set_visible(false);
                self.set_subtitle("");
                self.set_activatable(false);
            }
            WifiNetworkState::Forgetting => {
                imp.connected_icon.set_visible(false);
                imp.menu_button.set_visible(false);
                self.set_subtitle("");
                self.set_activatable(false);
            }
        }
    }
}
