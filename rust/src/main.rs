mod probe;

use gtk::{gio, glib, prelude::*};
use std::path::PathBuf;

const APP_ID: &str = "io.github.Rido_o.Vitrine.Spike";

fn main() -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE | gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    app.connect_activate(|app| {
        let dir = std::env::current_dir().unwrap_or_else(|_| glib::home_dir());
        build_window(app, dir);
    });
    app.connect_open(|app, files, _| {
        if let Some(dir) = files.first().and_then(|file| file.path()) {
            build_window(app, dir);
        }
    });
    app.run()
}

fn build_window(app: &gtk::Application, dir: PathBuf) {
    let stack = gtk::Stack::new();
    stack.add_named(
        &gtk::Label::new(Some(&dir.display().to_string())),
        Some("grid"),
    );
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Vitrine (spike)")
        .default_width(1600)
        .default_height(1000)
        .child(&stack)
        .build();
    window.present();
    if std::env::var_os("VITRINE_PROBE").is_some() {
        probe::run(&window);
    }
}
