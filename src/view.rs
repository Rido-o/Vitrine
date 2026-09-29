//! The full-screen view's page: the image, with the controls over it
//! (fullscreen and close at the top right, the file's name and resolution at
//! the bottom) that fade out with the cursor in fullscreen; the fullscreen
//! toggle, and the view's own keys.

use crate::{autohide::AutoHide, preview::Preview};
use gtk::{gdk, prelude::*};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

const HIDE_CONTROLS_AFTER: Duration = Duration::from_secs(2);

pub struct View {
    pub page: gtk::Overlay,
    window: gtk::ApplicationWindow,
    preview: Rc<Preview>,
    fullscreen_button: gtk::Button,
    fullscreen_icon: gtk::Image,
    close_button: gtk::Button,
    controls: gtk::Box,
    menu_buttons: Rc<RefCell<Vec<gtk::MenuButton>>>,
    // The view went fullscreen (f or the button), so leaving it restores the
    // window.
    fullscreened_by_view: Cell<bool>,
    autohide: Rc<AutoHide>,
}

impl View {
    /// `stack` holds the page (as "preview").
    pub fn new(
        window: &gtk::ApplicationWindow,
        stack: &gtk::Stack,
        preview: &Rc<Preview>,
    ) -> Rc<Self> {
        let page = gtk::Overlay::builder()
            .css_classes(["viewer-preview"])
            .child(&preview.image)
            .build();

        let fullscreen_icon = gtk::Image::builder()
            .icon_name("expand-awesome-symbolic")
            .pixel_size(20)
            .build();
        let fullscreen_button = gtk::Button::builder().child(&fullscreen_icon).build();
        let close_button = gtk::Button::builder()
            .tooltip_text("Back to grid (Esc)")
            .child(
                &gtk::Image::builder()
                    .icon_name("xmark-awesome-symbolic")
                    .pixel_size(20)
                    .build(),
            )
            .build();
        let controls = gtk::Box::builder()
            .css_classes(["preview-controls"])
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .margin_top(16)
            .margin_end(16)
            .spacing(8)
            .build();
        controls.append(&fullscreen_button);
        controls.append(&close_button);
        page.add_overlay(&controls);

        let filename = gtk::Label::builder()
            .label("No image selected")
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .max_width_chars(60)
            .build();
        let filename_button = gtk::Button::builder()
            .css_classes(["viewer-filename-button"])
            .tooltip_text("Show in file manager")
            .child(&filename)
            .build();
        let resolution = gtk::Label::builder().label("0 × 0").build();
        let info = gtk::Box::builder()
            .css_classes(["preview-image-info", "preview-controls"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::End)
            .margin_bottom(24)
            .spacing(8)
            .build();
        info.append(&filename_button);
        info.append(&resolution);
        page.add_overlay(&info);

        let (window_, stack_) = (window.clone(), stack.clone());
        let image = preview.image.clone();
        let autohide = AutoHide::new(
            HIDE_CONTROLS_AFTER,
            move || {
                window_.is_fullscreen() && stack_.visible_child_name().as_deref() == Some("preview")
            },
            move |hidden| image.set_cursor_hidden(hidden),
        );
        let menu_buttons: Rc<RefCell<Vec<gtk::MenuButton>>> = Rc::default();
        let open = menu_buttons.clone();
        autohide.set_busy(move || open.borrow().iter().any(|button| button.is_active()));
        autohide.add(&controls);
        autohide.add(&info);
        autohide.watch(&page);

        let shown: Rc<RefCell<Option<PathBuf>>> = Rc::default();
        let shown_ = shown.clone();
        filename_button.connect_clicked(move |_| {
            if let Some(path) = shown_.borrow().as_deref() {
                crate::window::show_in_file_manager(path);
            }
        });
        preview.connect_info(move |path: &Path, full: Option<(u32, u32)>| {
            *shown.borrow_mut() = Some(path.to_owned());
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned());
            filename.set_label(name.as_deref().unwrap_or(""));
            match full {
                Some((width, height)) => resolution.set_label(&format!("{width} × {height}")),
                None => resolution.set_label("…"),
            }
        });

        let view = Rc::new(Self {
            page,
            window: window.clone(),
            preview: preview.clone(),
            fullscreen_button,
            fullscreen_icon,
            close_button,
            controls,
            menu_buttons: menu_buttons.clone(),
            fullscreened_by_view: Cell::new(false),
            autohide,
        });
        let weak = Rc::downgrade(&view);
        view.fullscreen_button.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                view.toggle_fullscreen();
            }
        });
        let weak = Rc::downgrade(&view);
        window.connect_fullscreened_notify(move |_| {
            if let Some(view) = weak.upgrade() {
                view.sync_fullscreen();
                view.autohide.show();
            }
        });
        let weak = Rc::downgrade(&view);
        stack.connect_visible_child_name_notify(move |_| {
            if let Some(view) = weak.upgrade() {
                view.autohide.show();
            }
        });
        view.sync_fullscreen();
        view
    }

    /// A menu or popover button, first in the controls; the controls stay
    /// while any of these is open.
    pub fn add_menu_button(&self, button: &gtk::MenuButton) {
        self.controls.prepend(button);
        self.menu_buttons.borrow_mut().push(button.clone());
    }

    /// What the close button does (leaving the view).
    pub fn connect_close(&self, close: impl Fn() + 'static) {
        self.close_button.connect_clicked(move |_| close());
    }

    pub fn toggle_fullscreen(&self) {
        if self.window.is_fullscreen() {
            self.window.unfullscreen();
        } else {
            self.fullscreened_by_view.set(true);
            self.window.fullscreen();
        }
    }

    /// Leaving the view: back to a window if the view made it fullscreen.
    pub fn leave(&self) {
        if self.fullscreened_by_view.get() {
            self.window.unfullscreen();
        }
    }

    fn sync_fullscreen(&self) {
        let fullscreen = self.window.is_fullscreen();
        if !fullscreen {
            self.fullscreened_by_view.set(false);
        }
        self.fullscreen_icon.set_icon_name(Some(if fullscreen {
            "compress-awesome-symbolic"
        } else {
            "expand-awesome-symbolic"
        }));
        self.fullscreen_button.set_tooltip_text(Some(if fullscreen {
            "Leave fullscreen (f)"
        } else {
            "Fullscreen (f)"
        }));
    }

    /// The view's own keys (zoom, sharp pixels, rotation, flips, fullscreen);
    /// whether `key` was one.
    pub fn key(&self, key: gdk::Key) -> bool {
        let image = &self.preview.image;
        match key {
            gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => image.zoom_in(),
            gdk::Key::minus | gdk::Key::KP_Subtract => image.zoom_out(),
            gdk::Key::_0 | gdk::Key::KP_0 => image.reset_zoom(),
            gdk::Key::s | gdk::Key::S => image.toggle_sharp(),
            gdk::Key::f | gdk::Key::F => self.toggle_fullscreen(),
            gdk::Key::bracketleft => image.rotate(false),
            gdk::Key::bracketright => image.rotate(true),
            gdk::Key::h | gdk::Key::H => image.flip(true),
            gdk::Key::v | gdk::Key::V => image.flip(false),
            _ => return false,
        }
        true
    }
}
