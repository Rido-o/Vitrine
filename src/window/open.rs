//! The open button's choosers (a folder, or an image), drops on the window,
//! and what a chosen or dropped path opens, in the grid and in the full-screen view.

use super::Window;
use gtk::{gdk, gio, glib, prelude::*};
use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

impl Window {
    // GTK has no chooser for either a folder or a file, hence the two. Each
    // is the desktop's, through the portal, where there is one, and starts
    // in the folder shown.
    fn chooser(&self, title: &str) -> gtk::FileDialog {
        gtk::FileDialog::builder()
            .title(title)
            .modal(true)
            .initial_folder(&gio::File::for_path(self.folder.directory()))
            .build()
    }

    pub(super) fn choose_folder(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.chooser("Open a folder").select_folder(
            Some(&self.window),
            gio::Cancellable::NONE,
            move |result| {
                if let Some(this) = weak.upgrade() {
                    this.chosen(result);
                }
            },
        );
    }

    pub(super) fn choose_image(self: &Rc<Self>) {
        let images = gtk::FileFilter::new();
        images.set_name(Some("Images"));
        for extension in crate::library::EXTENSIONS {
            images.add_suffix(extension);
        }
        let dialog = self.chooser("Open an image");
        dialog.set_default_filter(Some(&images));
        let weak = Rc::downgrade(self);
        dialog.open(Some(&self.window), gio::Cancellable::NONE, move |result| {
            if let Some(this) = weak.upgrade() {
                this.chosen(result);
            }
        });
    }

    fn chosen(self: &Rc<Self>, result: Result<gio::File, glib::Error>) {
        match result.map(|file| file.path()) {
            Ok(Some(path)) => self.open_chosen(&path),
            Ok(None) => self.toast.show("Not a local file", false),
            Err(error) if closed(&error) => {}
            Err(error) => {
                eprintln!("Could not choose a file: {error}");
                self.toast.show("Could not open the chooser", false);
            }
        }
    }

    // A folder or an image dropped on the window (the first, of several)
    // opens as a chosen one does.
    pub(super) fn connect_drop(self: &Rc<Self>) {
        // A move is accepted too, though copy is asked for (nothing is moved):
        // where the compositor names the source's preferred action before
        // this side's (Hyprland, a move from Thunar), GTK takes that for the
        // only one on offer, and a copy-only target refuses the drop.
        let drop = gtk::DropTarget::new(
            gdk::FileList::static_type(),
            gdk::DragAction::COPY | gdk::DragAction::MOVE,
        );
        drop.connect_enter(|_, _, _| gdk::DragAction::COPY);
        drop.connect_motion(|_, _, _| gdk::DragAction::COPY);
        let weak = Rc::downgrade(self);
        drop.connect_drop(move |_, value, _, _| {
            let path = value
                .get::<gdk::FileList>()
                .ok()
                .and_then(|list| list.files().first().and_then(|file| file.path()));
            match (weak.upgrade(), path) {
                (Some(this), Some(path)) => {
                    this.open_chosen(&path);
                    true
                }
                _ => false,
            }
        });
        self.window.add_controller(drop);
    }

    // A folder opens with its first image selected, an image opens its
    // folder with the image selected; from the view, that image is shown
    // there (a folder's first once it's scanned, see `finished`).
    fn open_chosen(self: &Rc<Self>, path: &Path) {
        if path.is_dir() {
            self.open_path(path, None);
            return;
        }
        if !crate::library::is_image(path) {
            self.toast.show("Not an image Vitrine can open", false);
            return;
        }
        let directory = path.parent().unwrap_or(Path::new("/"));
        if self.open_path(directory, Some(path.to_owned())) && !self.in_grid() {
            self.show_file(path.to_owned());
        }
    }

    // "win.open-path", for the probe, which can't choose in a chooser.
    pub(super) fn add_probe_open(self: &Rc<Self>) {
        let open = gio::SimpleAction::new("open-path", Some(&PathBuf::static_variant_type()));
        let weak = Rc::downgrade(self);
        open.connect_activate(move |_, parameter| {
            if let Some(this) = weak.upgrade()
                && let Some(path) = parameter.and_then(|p| p.get::<PathBuf>())
            {
                this.open_chosen(&path);
            }
        });
        self.window.add_action(&open);
    }
}

// The chooser was closed without anything chosen.
fn closed(error: &glib::Error) -> bool {
    matches!(
        error.kind::<gtk::DialogError>(),
        Some(gtk::DialogError::Cancelled | gtk::DialogError::Dismissed)
    )
}
