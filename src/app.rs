use adw::prelude::*;
use gtk::{gdk, gio};

use crate::ui;

pub const APP_ID: &str = "io.github.dandiccf.Ankunft";

pub fn run() {
    let application = adw::Application::builder().application_id(APP_ID).build();

    application.connect_startup(|app| {
        load_css();
        install_actions(app);
    });
    application.connect_activate(ui::build_window);
    application.run();
}

fn install_actions(app: &adw::Application) {
    let open_web = gio::SimpleAction::new("open-web", None);
    open_web.connect_activate(|_, _| {
        let _ = gio::AppInfo::launch_default_for_uri(
            "https://web.parcelapp.net/",
            gio::AppLaunchContext::NONE,
        );
    });
    app.add_action(&open_web);

    let quit = gio::SimpleAction::new("quit", None);
    let app_weak = app.downgrade();
    quit.connect_activate(move |_, _| {
        if let Some(app) = app_weak.upgrade() {
            app.quit();
        }
    });
    app.add_action(&quit);

    app.set_accels_for_action("app.quit", &["<primary>q"]);
}

fn load_css() {
    let Some(display) = gdk::Display::default() else {
        return;
    };

    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("../data/style.css"));

    gtk::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}
