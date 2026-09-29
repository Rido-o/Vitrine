//! The keyboard shortcuts window. As Shortcuts.ts, listing only what the
//! spike has so far: properties (i) and wallpaper (w) join when they're
//! ported. Keep in step with `Window::key` and the view's keys.

use gtk::{gdk, glib, prelude::*};

const SECTIONS: [(&str, &[(&str, &str)]); 3] = [
    (
        "Grid and full-screen view",
        &[
            ("Ctrl+C", "Copy image"),
            ("Ctrl+Shift+C", "Copy path"),
            ("r", "Rescan folder"),
            ("Delete", "Move to trash"),
            ("Ctrl+Z", "Undo delete"),
            ("Ctrl+W, Ctrl+Q", "Close the window"),
            ("?", "Keyboard shortcuts"),
        ],
    ),
    (
        "Grid",
        &[
            ("Enter, e, double-click", "Open in the full-screen view"),
            ("Esc, q", "Quit"),
        ],
    ),
    (
        "Full-screen view",
        &[
            ("← →, click the edges", "Previous/next image"),
            ("Scroll, drag", "Zoom around the cursor, pan"),
            ("Double-click", "Toggle fit/100%"),
            ("+ − 0", "Zoom in, out, fit"),
            ("s", "Sharp pixels"),
            ("[ ]", "Rotate left, right (view only)"),
            ("h v", "Flip horizontally, vertically (view only)"),
            ("f", "Toggle fullscreen"),
            ("Esc, q", "Back to the grid"),
        ],
    ),
];

/// A modal window listing the keys; Esc, q or ? closes it.
pub fn show(parent: &impl IsA<gtk::Window>) {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .css_classes(["viewer-shortcuts-content"])
        .build();
    for (title, rows) in SECTIONS {
        content.append(
            &gtk::Label::builder()
                .label(title)
                .xalign(0.0)
                .css_classes(["viewer-shortcuts-heading"])
                .build(),
        );
        let grid = gtk::Grid::builder()
            .column_spacing(16)
            .row_spacing(6)
            .build();
        for (row, (keys, description)) in rows.iter().enumerate() {
            let row = row as i32;
            grid.attach(
                &gtk::Label::builder()
                    .label(*keys)
                    .xalign(0.0)
                    .css_classes(["key"])
                    .build(),
                0,
                row,
                1,
                1,
            );
            grid.attach(
                &gtk::Label::builder()
                    .label(*description)
                    .xalign(0.0)
                    .hexpand(true)
                    .build(),
                1,
                row,
                1,
                1,
            );
        }
        content.append(&grid);
    }

    let window = gtk::Window::builder()
        .title("Keyboard shortcuts")
        .transient_for(parent)
        .modal(true)
        .default_width(520)
        .default_height(620)
        .css_classes(["viewer-shortcuts"])
        .child(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&content)
                .build(),
        )
        .build();
    let keys = gtk::EventControllerKey::new();
    let window_ = window.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        if !matches!(key, gdk::Key::Escape | gdk::Key::q | gdk::Key::question) {
            return glib::Propagation::Proceed;
        }
        window_.close();
        glib::Propagation::Stop
    });
    window.add_controller(keys);
    window.present();
}
