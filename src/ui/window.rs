use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use std::cell::OnceCell;

use crate::backend::WlcontrolManager;
use crate::ui::{BluetoothPage, WifiPage};

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/neoden/wlcontrol/ui/window.ui")]
    pub struct WlcontrolWindow {
        #[template_child]
        pub wifi_page: TemplateChild<WifiPage>,
        #[template_child]
        pub bluetooth_page: TemplateChild<BluetoothPage>,

        pub manager: OnceCell<WlcontrolManager>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WlcontrolWindow {
        const NAME: &'static str = "WlcontrolWindow";
        type Type = super::WlcontrolWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            WifiPage::ensure_type();
            BluetoothPage::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for WlcontrolWindow {
        fn constructed(&self) {
            self.parent_constructed();
        }
    }

    impl WidgetImpl for WlcontrolWindow {}
    impl WindowImpl for WlcontrolWindow {}
    impl ApplicationWindowImpl for WlcontrolWindow {}
    impl AdwApplicationWindowImpl for WlcontrolWindow {}
}

glib::wrapper! {
    pub struct WlcontrolWindow(ObjectSubclass<imp::WlcontrolWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl WlcontrolWindow {
    pub fn new(app: &adw::Application, manager: &WlcontrolManager) -> Self {
        let window: Self = glib::Object::builder()
            .property("application", app)
            .build();

        window.imp().manager.set(manager.clone()).unwrap();
        window.imp().wifi_page.set_manager(manager);
        window.imp().bluetooth_page.set_manager(manager);

        window
    }

    pub fn manager(&self) -> &WlcontrolManager {
        self.imp().manager.get().unwrap()
    }
}
