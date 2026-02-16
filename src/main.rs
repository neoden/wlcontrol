mod application;
mod backend;
mod ui;

use application::WlcontrolApplication;
use gtk::prelude::*;
use gtk::{gio, glib};

const APP_ID: &str = "dev.neoden.wlcontrol";

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    gio::resources_register_include!("wlcontrol.gresource")
        .expect("Failed to register resources");

    let app = WlcontrolApplication::new(APP_ID, &gio::ApplicationFlags::empty());
    app.run()
}
