use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};

use crate::backend::WlcontrolManager;
use crate::ui::WlcontrolWindow;

mod imp {
    use super::*;
    use std::cell::OnceCell;

    #[derive(Default)]
    pub struct WlcontrolApplication {
        pub manager: OnceCell<WlcontrolManager>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for WlcontrolApplication {
        const NAME: &'static str = "WlcontrolApplication";
        type Type = super::WlcontrolApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for WlcontrolApplication {}

    impl ApplicationImpl for WlcontrolApplication {
        fn startup(&self) {
            self.parent_startup();

            // Load CSS
            let provider = gtk::CssProvider::new();
            provider.load_from_resource("/dev/neoden/wlcontrol/style.css");
            gtk::style_context_add_provider_for_display(
                &gtk::gdk::Display::default().unwrap(),
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        fn activate(&self) {
            let app = self.obj();

            // Initialize manager on first activation
            let manager = self.manager.get_or_init(|| {
                let manager = WlcontrolManager::new();
                manager.start();
                manager
            });

            let window = if let Some(window) = app.active_window() {
                window
            } else {
                let window = WlcontrolWindow::new(app.upcast_ref(), manager);
                window.upcast()
            };

            window.present();
        }
    }

    impl GtkApplicationImpl for WlcontrolApplication {}
    impl AdwApplicationImpl for WlcontrolApplication {}
}

glib::wrapper! {
    pub struct WlcontrolApplication(ObjectSubclass<imp::WlcontrolApplication>)
        @extends gio::Application, gtk::Application, adw::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl WlcontrolApplication {
    pub fn new(application_id: &str, flags: &gio::ApplicationFlags) -> Self {
        glib::Object::builder()
            .property("application-id", application_id)
            .property("flags", flags)
            .build()
    }
}
