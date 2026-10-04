//! The full-screen view's page: the image, with the controls over it
//! (open at the top left; properties, the ⋯ menu and close at the top right;
//! the file's name, resolution, size and position in the folder at the bottom,
//! between fullscreen and back to the grid) that fade out with the cursor in
//! fullscreen;
//! the image's context menu, the fullscreen toggle, and the view's own keys.

mod autohide;
pub mod preview;
pub mod zoomable;

use self::{autohide::AutoHide, preview::Preview};
use crate::icon;
use gtk::{gdk, gio, prelude::*};
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
    grid_button: gtk::Button,
    file_size: gtk::Label,
    position: gtk::Label,
    controls: gtk::Box,
    open_slot: gtk::Box,
    menu_buttons: Rc<RefCell<Vec<gtk::MenuButton>>>,
    // The image's context menu.
    context_menu: gtk::PopoverMenu,
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

        let fullscreen_icon = icon("expand-awesome-symbolic", 20);
        let fullscreen_button = gtk::Button::builder().child(&fullscreen_icon).build();
        let grid_button = gtk::Button::builder()
            .tooltip_text("Back to grid (Esc)")
            .child(&icon("table-cells-large-awesome-symbolic", 20))
            .build();
        let close_button = gtk::Button::builder()
            .tooltip_text("Close (Ctrl+W)")
            .child(&icon("xmark-awesome-symbolic", 20))
            .build();
        // Top right: the image's i and ⋯ (`add_menu_button`), then close,
        // apart from them.
        let controls = gtk::Box::builder()
            .css_classes(["preview-controls"])
            .halign(gtk::Align::End)
            .valign(gtk::Align::Start)
            .margin_top(16)
            .margin_end(16)
            .spacing(8)
            .build();
        close_button.set_margin_start(12);
        controls.append(&close_button);
        // Top left, as in the grid's toolbar: the open button.
        let open_slot = gtk::Box::builder()
            .css_classes(["preview-controls"])
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .margin_top(16)
            .margin_start(16)
            .build();
        page.add_overlay(&open_slot);
        let window_ = window.clone();
        close_button.connect_clicked(move |_| window_.close());
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
        let file_size = gtk::Label::builder().visible(false).build();
        let position = gtk::Label::builder().visible(false).build();
        let info = gtk::Box::builder()
            .css_classes(["preview-image-info"])
            .spacing(8)
            .build();
        info.append(&filename_button);
        info.append(&resolution);
        info.append(&file_size);
        info.append(&position);
        // At the bottom: fullscreen, the info, back to the grid.
        let bottom = gtk::Box::builder()
            .css_classes(["preview-controls"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::End)
            .margin_bottom(24)
            .spacing(8)
            .build();
        bottom.append(&fullscreen_button);
        bottom.append(&info);
        bottom.append(&grid_button);
        page.add_overlay(&bottom);

        let (window_, stack_) = (window.clone(), stack.clone());
        let image = preview.image.clone();
        // The controls, wherever the view is shown; the cursor only in
        // fullscreen and in colour assessment, not over a window among others.
        let autohide = AutoHide::new(
            HIDE_CONTROLS_AFTER,
            move || stack_.visible_child_name().as_deref() == Some("preview"),
            move |hidden| {
                image.set_cursor_hidden(hidden && (window_.is_fullscreen() || image.assessment()))
            },
        );
        let menu_buttons: Rc<RefCell<Vec<gtk::MenuButton>>> = Rc::default();
        let context_menu = gtk::PopoverMenu::builder()
            .has_arrow(false)
            .halign(gtk::Align::Start)
            .build();
        // On the image, which can take the focus: when the menu closes, the
        // window gives the focus to its nearest ancestor that takes it. With
        // none it searches the window instead, and GTK warns
        // (gtk_widget_is_ancestor) when that search meets the properties
        // popover the menu has just opened.
        context_menu.set_parent(&preview.image);
        let (open, context) = (menu_buttons.clone(), context_menu.clone());
        autohide.set_busy(move || {
            context.is_visible() || open.borrow().iter().any(|button| button.is_active())
        });
        autohide.add(&controls);
        autohide.add(&open_slot);
        autohide.add(&bottom);
        autohide.watch(&page);

        let shown: Rc<RefCell<Option<PathBuf>>> = Rc::default();
        let shown_ = shown.clone();
        filename_button.connect_clicked(move |_| {
            if let Some(path) = shown_.borrow().as_deref() {
                crate::desktop::show_in_file_manager(path);
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
            grid_button,
            file_size,
            position,
            controls,
            open_slot,
            menu_buttons: menu_buttons.clone(),
            context_menu,
            fullscreened_by_view: Cell::new(false),
            autohide,
        });
        let weak = Rc::downgrade(&view);
        view.fullscreen_button.connect_clicked(move |_| {
            if let Some(view) = weak.upgrade() {
                view.toggle_fullscreen();
            }
        });
        // A right click on the image opens its context menu, at the pointer.
        let click = gtk::GestureClick::builder()
            .button(gdk::BUTTON_SECONDARY)
            .build();
        let weak = Rc::downgrade(&view);
        click.connect_pressed(move |click, _, x, y| {
            if let Some(view) = weak.upgrade() {
                click.set_state(gtk::EventSequenceState::Claimed);
                view.show_context_menu(Some((x, y)));
            }
        });
        preview.image.add_controller(click);
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

    /// The open button, alone at the top left; the controls stay while its
    /// menu is open too.
    pub fn add_open_button(&self, button: &gtk::MenuButton) {
        self.open_slot.append(button);
        self.menu_buttons.borrow_mut().push(button.clone());
    }

    /// The image's context menu (a right click, or the Menu key).
    pub fn set_context_menu(&self, model: &gio::Menu) {
        self.context_menu.set_menu_model(Some(model));
    }

    /// Opens the context menu at a point of the image's widget, or in its
    /// middle; the controls come back, for the menu's Properties (the i
    /// button's popover).
    pub fn show_context_menu(&self, at: Option<(f64, f64)>) {
        let image = &self.preview.image;
        if !image.has_image() {
            return;
        }
        let (x, y) = at.unwrap_or((image.width() as f64 / 2.0, image.height() as f64 / 2.0));
        self.context_menu
            .set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        self.context_menu.popup();
        self.autohide.show();
    }

    /// Before the window goes: the context menu was parented by hand.
    pub fn dispose(&self) {
        self.context_menu.unparent();
    }

    /// The image's file size and position in the folder ("3 of 120"), after
    /// its resolution; none with nothing selected.
    pub fn set_details(&self, file_size: Option<&str>, position: Option<&str>) {
        for (label, text) in [(&self.file_size, file_size), (&self.position, position)] {
            label.set_visible(text.is_some());
            label.set_label(text.unwrap_or(""));
        }
    }

    /// What the grid button does (leaving the view).
    pub fn connect_back(&self, back: impl Fn() + 'static) {
        self.grid_button.connect_clicked(move |_| back());
    }

    pub fn toggle_fullscreen(&self) {
        if self.window.is_fullscreen() {
            self.window.unfullscreen();
        } else {
            self.fullscreened_by_view.set(true);
            self.window.fullscreen();
        }
    }

    /// Colour assessment on or off, for this window until it closes.
    pub fn toggle_assessment(&self) {
        let image = &self.preview.image;
        image.set_assessment(!image.assessment());
        self.autohide.show();
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
        let key = key.to_lower();
        let image = &self.preview.image;
        match key {
            gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => image.zoom_in(),
            gdk::Key::minus | gdk::Key::KP_Subtract => image.zoom_out(),
            gdk::Key::_0 | gdk::Key::KP_0 => image.reset_zoom(),
            gdk::Key::s => image.toggle_sharp(),
            gdk::Key::f => self.toggle_fullscreen(),
            gdk::Key::bracketleft => image.rotate(false),
            gdk::Key::bracketright => image.rotate(true),
            gdk::Key::h => image.flip(true),
            gdk::Key::v => image.flip(false),
            _ => return false,
        }
        true
    }
}
