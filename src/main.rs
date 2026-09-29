mod actions;
mod autohide;
mod color;
mod decode;
mod history;
mod library;
mod preview;
mod probe;
mod properties;
mod shortcuts;
mod thumbnails;
mod tiles;
mod trash;
mod view;
mod window;
mod zoomable;

use gtk::{gdk, gio, glib, prelude::*};
use std::{cell::Cell, path::Path, rc::Rc};

pub const APP_ID: &str = "io.github.Rido_o.Vitrine";
pub const APP_NAME: &str = "vitrine";
// style/style.scss, compiled by build.rs.
const CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/style.css"));

// The symbolic icons (icons/), installed by package.nix.
// Pruning the thumbnail cache waits this long after startup.
const HOUSEKEEPING_DELAY_SECONDS: u32 = 5;

const ICONS_DIR: &str = match option_env!("VITRINE_ICONS_DIR") {
    Some(dir) => dir,
    None => concat!(env!("CARGO_MANIFEST_DIR"), "/icons"),
};

fn main() -> glib::ExitCode {
    tune_malloc();
    prefer_gl_on_nvidia();
    glib::set_prgname(Some(APP_NAME));
    glib::set_application_name("Vitrine");
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE | gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    app.add_main_option(
        "subfolders",
        glib::Char::from(b'r'),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Include images in subfolders",
        None,
    );
    let subfolders = Rc::new(Cell::new(false));
    let subfolders_ = subfolders.clone();
    app.connect_handle_local_options(move |_, options| {
        subfolders_.set(options.contains("subfolders"));
        std::ops::ControlFlow::Continue(())
    });
    app.connect_startup(|_| {
        let display = gdk::Display::default().expect("a display");
        // Ours, then the user's own style.css over it; the colour scheme is
        // set before loading, as changing it parses a provider again.
        let ours = gtk::CssProvider::new();
        let user_css = glib::user_config_dir().join(APP_NAME).join("style.css");
        let user = user_css.is_file().then(gtk::CssProvider::new);
        follow_color_scheme(
            [Some(ours.clone()), user.clone()]
                .into_iter()
                .flatten()
                .collect(),
        );
        ours.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(
            &display,
            &ours,
            gtk::STYLE_PROVIDER_PRIORITY_USER,
        );
        if let Some(user) = user {
            user.connect_parsing_error(|_, section, error| {
                eprintln!("vitrine: style.css: {section}: {error}");
            });
            user.load_from_path(&user_css);
            gtk::style_context_add_provider_for_display(
                &display,
                &user,
                gtk::STYLE_PROVIDER_PRIORITY_USER + 1,
            );
        }
        gtk::IconTheme::for_display(&display).add_search_path(ICONS_DIR);
        // After the first thumbnails.
        glib::timeout_add_seconds_local_once(HOUSEKEEPING_DELAY_SECONDS, thumbnails::housekeeping);
    });
    // No argument: ~/Pictures, or the current folder without one.
    let subfolders_ = subfolders.clone();
    app.connect_activate(move |app| {
        let directory = glib::user_special_dir(glib::UserDirectory::Pictures)
            .filter(|pictures| pictures.is_dir())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(glib::home_dir);
        window::Window::new(app, &directory, None, subfolders_.get());
    });
    // A folder opens its grid; a file opens its folder with the file in the
    // full-screen view. Only the first argument is used.
    app.connect_open(move |app, files, _| {
        let Some(path) = files.first().and_then(|file| file.path()) else {
            return app.activate();
        };
        if path.is_dir() {
            window::Window::new(app, &path, None, subfolders.get());
        } else {
            let directory = path.parent().unwrap_or(Path::new("/")).to_owned();
            window::Window::new(app, &directory, Some(path), subfolders.get());
        }
    });
    app.run()
}

// The styles' light palette only when the desktop prefers light: GTK treats
// no preference as light, and Vitrine has always been dark.
fn follow_color_scheme(providers: Vec<gtk::CssProvider>) {
    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    let apply = move |settings: &gtk::Settings| {
        let scheme = match settings.gtk_interface_color_scheme() {
            gtk::InterfaceColorScheme::Light => gtk::InterfaceColorScheme::Light,
            _ => gtk::InterfaceColorScheme::Dark,
        };
        for provider in &providers {
            provider.set_prefers_color_scheme(scheme);
        }
    };
    apply(&settings);
    settings.connect_gtk_interface_color_scheme_notify(apply);
}

// glibc raises its mmap threshold each time a large block is freed, so the
// workers' multi-MB decode buffers ended up in per-thread arenas that never
// shrink (~230 MB after generating a 3,000-image folder). A fixed 1 MB
// threshold returns them to the system on free (thumbnails, ~0.5 MB, stay in
// the arenas), and 4 arenas bound the rest: peak RSS 844 → 718 MB, no
// measurable slowdown.
fn tune_malloc() {
    // SAFETY: mallopt only sets allocator parameters; called before any
    // other thread exists.
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 1 << 20);
        libc::mallopt(libc::M_ARENA_MAX, 4);
    }
}

// With NVIDIA's driver, GTK's default Vulkan renderer spends several ms of
// main-thread time on each new texture (a row of thumbnails, or each tile
// panned into view: 76 ms frames panning at 100%); its GL renderer doesn't.
// Only when the driver is loaded, and never over an explicit GSK_RENDERER.
fn prefer_gl_on_nvidia() {
    if std::env::var_os("GSK_RENDERER").is_none() && Path::new("/proc/driver/nvidia").is_dir() {
        // SAFETY: before any other thread exists.
        unsafe { std::env::set_var("GSK_RENDERER", "gl") };
    }
}
