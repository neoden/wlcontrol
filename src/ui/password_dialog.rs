use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/dev/neoden/wlcontrol/ui/password-dialog.ui")]
    pub struct PasswordDialog {
        #[template_child]
        pub password_entry: TemplateChild<adw::PasswordEntryRow>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PasswordDialog {
        const NAME: &'static str = "PasswordDialog";
        type Type = super::PasswordDialog;
        type ParentType = adw::AlertDialog;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for PasswordDialog {
        fn constructed(&self) {
            self.parent_constructed();

            // WPA/WPA2 requires minimum 8 characters
            const MIN_PASSWORD_LENGTH: usize = 8;

            // Enable connect button only when password meets minimum length
            let dialog = self.obj();
            self.password_entry.connect_text_notify(glib::clone!(
                #[weak]
                dialog,
                move |entry| {
                    let valid = entry.text().len() >= MIN_PASSWORD_LENGTH;
                    dialog.set_response_enabled("connect", valid);
                }
            ));

            // Initially disable connect button
            dialog.set_response_enabled("connect", false);
        }
    }

    impl WidgetImpl for PasswordDialog {}
    impl AdwDialogImpl for PasswordDialog {}
    impl AdwAlertDialogImpl for PasswordDialog {}
}

glib::wrapper! {
    pub struct PasswordDialog(ObjectSubclass<imp::PasswordDialog>)
        @extends gtk::Widget, adw::Dialog, adw::AlertDialog,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl PasswordDialog {
    pub fn new(network_name: &str) -> Self {
        let dialog: Self = glib::Object::new();
        dialog.set_body(&format!("Enter password for \"{}\"", network_name));
        dialog
    }

    pub fn password(&self) -> String {
        self.imp().password_entry.text().to_string()
    }

    pub async fn run(self, parent: &impl IsA<gtk::Widget>) -> Option<String> {
        let entry = self.imp().password_entry.clone();
        let response = self.choose_future(Some(parent)).await;
        if response == "connect" {
            Some(entry.text().to_string())
        } else {
            None
        }
    }
}
