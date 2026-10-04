//! The keyboard shortcuts window. Keep in step with `Window::key` and the
//! view's keys.

use gtk::{gdk, glib, prelude::*};

// Each row's keys (or pointer actions) and what they do.
type Row = (&'static [&'static str], &'static str);

const SECTIONS: [(&str, &[Row]); 3] = [
    (
        "Grid and full-screen view",
        &[
            (&["i"], "Image properties"),
            (&["Ctrl+C"], "Copy image"),
            (&["Ctrl+Shift+C"], "Copy path"),
            (&["Menu", "Shift+F10", "right-click"], "Image menu"),
            (&["w"], "Set as wallpaper"),
            (&["r"], "Rescan folder"),
            (&["Delete"], "Move to trash"),
            (&["Ctrl+Z"], "Undo delete"),
            (&["Ctrl+W", "Ctrl+Q"], "Close the window"),
            (&["?"], "Keyboard shortcuts"),
        ],
    ),
    (
        "Grid",
        &[(
            &["Enter", "e", "double-click"],
            "Open in the full-screen view",
        )],
    ),
    (
        "Full-screen view",
        &[
            (&["←", "→", "click the edges"], "Previous/next image"),
            (&["scroll", "drag"], "Zoom around the cursor, pan"),
            (&["double-click"], "Toggle fit/100%"),
            (&["+", "−", "0"], "Zoom in, out, fit"),
            (&["s"], "Sharp pixels"),
            (&["[", "]"], "Rotate left, right (view only)"),
            (&["h", "v"], "Flip horizontally, vertically (view only)"),
            (&["b"], "Colour assessment (grey surround, white frame)"),
            (&["f"], "Toggle fullscreen"),
            (&["Esc", "q"], "Back to the grid"),
        ],
    ),
];

// The window's content may take this much of the monitor's height before it
// scrolls.
const MAX_HEIGHT_FRACTION: f64 = 0.85;

/// A modal window listing the keys (w only with a wallpaper command); Esc, q
/// or ? closes it.
pub fn show(parent: &impl IsA<gtk::Window>, wallpaper: bool) {
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
        let rows = rows.iter().filter(|(keys, _)| wallpaper || *keys != ["w"]);
        for (row, (keys, description)) in rows.enumerate() {
            let chips = gtk::Box::builder().spacing(4).build();
            for key in *keys {
                let chip = gtk::Label::builder()
                    .label(*key)
                    .css_classes(["key"])
                    .build();
                // Lowercase words are pointer actions, not keys.
                if key.len() > 1 && key.starts_with(|c: char| c.is_ascii_lowercase()) {
                    chip.add_css_class("pointer");
                }
                chips.append(&chip);
            }
            grid.attach(&chips, 0, row as i32, 1, 1);
            grid.attach(
                &gtk::Label::builder()
                    .label(*description)
                    .xalign(0.0)
                    .hexpand(true)
                    .build(),
                1,
                row as i32,
                1,
                1,
            );
        }
        content.append(&grid);
    }

    let max_height = parent
        .as_ref()
        .surface()
        .and_then(|surface| surface.display().monitor_at_surface(&surface))
        .map_or(800, |monitor| {
            (monitor.geometry().height() as f64 * MAX_HEIGHT_FRACTION) as i32
        });
    let window = gtk::Window::builder()
        .title("Keyboard shortcuts")
        .transient_for(parent)
        .modal(true)
        .resizable(false)
        .css_classes(["viewer-shortcuts"])
        .titlebar(&gtk::HeaderBar::new())
        .child(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .propagate_natural_width(true)
                .propagate_natural_height(true)
                .max_content_height(max_height)
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
