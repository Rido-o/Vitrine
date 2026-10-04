//! The info bar under the grid (the selected image's name, resolution, size
//! and position in the folder, and View), and what else follows the selection: the window's title
//! and the empty folder's message.

use super::{APP_TITLE, Window};
use crate::library::{Image, image};
use gtk::{gio, glib, pango, prelude::*};
use std::{path::Path, rc::Rc};

pub struct InfoBar {
    pub root: gtk::Box,
    filename: gtk::Label,
    filename_button: gtk::Button,
    resolution: gtk::Label,
    file_size: gtk::Label,
    position: gtk::Label,
    view_button: gtk::Button,
}

impl InfoBar {
    pub fn new() -> Self {
        let filename = gtk::Label::builder()
            .label("No image selected")
            .ellipsize(pango::EllipsizeMode::End)
            .build();
        let filename_button = gtk::Button::builder()
            .css_classes(["viewer-filename-button"])
            .tooltip_text("Show in file manager")
            .child(&filename)
            .build();
        let resolution = gtk::Label::builder()
            .css_classes(["viewer-chip"])
            .visible(false)
            .build();
        let file_size = gtk::Label::builder()
            .css_classes(["viewer-chip"])
            .visible(false)
            .build();
        let position = gtk::Label::builder()
            .css_classes(["viewer-chip"])
            .tooltip_text("Position in the folder, as sorted")
            .visible(false)
            .build();
        let labels = gtk::Box::builder()
            .hexpand(true)
            .halign(gtk::Align::Start)
            .spacing(8)
            .build();
        labels.append(&filename_button);
        labels.append(&resolution);
        labels.append(&file_size);
        labels.append(&position);
        let view_button = gtk::Button::builder()
            .label("View")
            .tooltip_text("Full-screen view (Enter)")
            .halign(gtk::Align::End)
            .build();
        let root = gtk::Box::builder()
            .css_classes(["viewer-info"])
            .spacing(8)
            .build();
        root.append(&labels);
        root.append(&view_button);
        Self {
            root,
            filename,
            filename_button,
            resolution,
            file_size,
            position,
            view_button,
        }
    }
}

impl Window {
    pub(super) fn connect_info(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.info.filename_button.connect_clicked(move |_| {
            if let Some(path) = weak.upgrade().and_then(|this| this.selected_path()) {
                crate::desktop::show_in_file_manager(&path);
            }
        });
        let weak = Rc::downgrade(self);
        self.info.view_button.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade()
                && this.selection.selected_item().is_some()
            {
                this.show_at(this.selection.selected() as i64);
            }
        });
        let weak = Rc::downgrade(self);
        self.selection.connect_selected_item_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                this.remember_selection();
                this.sync_info();
            }
        });
        // Also when the same image moves (images added before it).
        let weak = Rc::downgrade(self);
        self.selection.connect_selected_notify(move |_| {
            if let Some(this) = weak.upgrade() {
                this.remember_selection();
            }
        });
    }

    fn remember_selection(&self) {
        if self.folder.is_updating() {
            return;
        }
        if let Some(path) = self.selected_path() {
            *self.last_selected.borrow_mut() = Some((path, self.selection.selected()));
        }
    }

    pub(super) fn sync_info(self: &Rc<Self>) {
        let path = self.selected_path();
        let name = path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned());
        let info = &self.info;
        info.filename
            .set_label(name.as_deref().unwrap_or("No image selected"));
        // With nothing selected, only the name's place says so.
        info.resolution.set_visible(path.is_some());
        if let Some(object) = self.selection.selected_item() {
            info.resolution
                .set_label(&self.resolution_of(&image(&object)));
        }
        info.view_button.set_sensitive(path.is_some());
        let size = self
            .selection
            .selected_item()
            .map(|object| image(&object).size);
        let size = size.map(glib::format_size);
        info.file_size.set_visible(size.is_some());
        info.file_size.set_label(size.as_deref().unwrap_or(""));
        // "3 of 120", here and in the view.
        let position = path.is_some().then(|| {
            format!(
                "{} of {}",
                self.selection.selected() + 1,
                self.selection.n_items()
            )
        });
        info.position.set_visible(position.is_some());
        info.position.set_label(position.as_deref().unwrap_or(""));
        self.view.set_details(size.as_deref(), position.as_deref());
        self.window.set_title(Some(&match &name {
            Some(name) => format!("{name} — {APP_TITLE}"),
            None => APP_TITLE.into(),
        }));
        self.empty.set_visible(self.selection.n_items() == 0);
        self.empty.set_label(if self.folder.is_loading() {
            "Scanning…"
        } else if self.folder.recursive() {
            "No images in this folder or its subfolders"
        } else {
            "No images in this folder — turn on Subfolders to include them"
        });
    }

    // The size that came with the image's thumbnail, or else the one in the
    // file's header, read on a worker the first time (a slow disk mustn't
    // hold up moving the selection). The thumbnail's is there for most tiles
    // in view, and still wanted once the read has started (`thumbnail_sized`):
    // on a network folder, opening the image can block for seconds.
    fn resolution_of(self: &Rc<Self>, image: &Image) -> String {
        let path = &image.path;
        if let Some(size) = self.resolutions.borrow().get(path) {
            return size_label(*size);
        }
        if let Some((width, height)) = self.tiles.size(image) {
            return size_label(Some((width as i32, height as i32)));
        }
        // One read each, however often the bar is synced meanwhile.
        if !self.resolving.borrow_mut().insert(path.to_owned()) {
            return "…".into();
        }
        let (weak, path) = (Rc::downgrade(self), path.to_owned());
        glib::spawn_future_local(async move {
            let read = path.clone();
            let size = gio::spawn_blocking(move || crate::properties::shown_size(&read))
                .await
                .ok()
                .flatten();
            let Some(this) = weak.upgrade() else { return };
            this.resolving.borrow_mut().remove(&path);
            this.resolutions.borrow_mut().insert(path.clone(), size);
            if this.selected_path().as_ref() == Some(&path) {
                this.info.resolution.set_label(&size_label(size));
            }
        });
        "…".into()
    }

    // The selected image's thumbnail arrived before its header was read.
    pub(super) fn thumbnail_sized(&self, path: &Path, (width, height): (u32, u32)) {
        if self.selected_path().as_deref() == Some(path) {
            self.info
                .resolution
                .set_label(&size_label(Some((width as i32, height as i32))));
        }
    }
}

fn size_label(size: Option<(i32, i32)>) -> String {
    match size {
        Some((width, height)) => format!("{width} × {height}"),
        None => "0 × 0".into(),
    }
}
