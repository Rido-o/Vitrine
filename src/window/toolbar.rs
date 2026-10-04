//! The grid's toolbar: the folder entry (after the open button, `menu.rs`),
//! the sort pill, Subfolders and, at the end, the i and ⋯ buttons (`menu.rs`)
//! and close.

use super::Window;
use crate::{
    icon,
    library::{Folder, SortKey},
};
use gtk::prelude::*;
use std::rc::Rc;

const SORTS: [(SortKey, &str, &str); 4] = [
    (SortKey::Name, "Name", "Sort by path"),
    (SortKey::Date, "Date", "Sort by date modified"),
    (SortKey::Size, "Size", "Sort by file size"),
    (
        SortKey::Random,
        "Random",
        "Shuffle (click again to reshuffle)",
    ),
];

pub struct Toolbar {
    pub root: gtk::Box,
    pub entry: gtk::Entry,
    sort_buttons: Vec<(SortKey, gtk::Button)>,
    direction: gtk::Button,
    subfolders: gtk::ToggleButton,
    close: gtk::Button,
    // The i and ⋯ buttons go here, before close.
    pub end: gtk::Box,
}

impl Toolbar {
    pub fn new(subfolders: bool) -> Self {
        let entry = gtk::Entry::builder()
            .css_classes(["viewer-directory"])
            .tooltip_text("Folder (Ctrl+L; Enter opens it)")
            .width_chars(36)
            .hexpand(true)
            .build();

        let sorts = gtk::Box::builder()
            .css_classes(["viewer-pill"])
            .spacing(2)
            .build();
        let sort_buttons: Vec<(SortKey, gtk::Button)> = SORTS
            .iter()
            .map(|&(key, label, tooltip)| {
                let button = gtk::Button::builder()
                    .label(label)
                    .tooltip_text(tooltip)
                    .build();
                sorts.append(&button);
                (key, button)
            })
            .collect();
        // The direction isn't one more key.
        sorts.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        let direction = gtk::Button::with_label("↑");
        sorts.append(&direction);

        let subfolders = gtk::ToggleButton::builder()
            .label("Subfolders")
            .active(subfolders)
            .tooltip_text("Include images in subfolders")
            .build();
        let subfolders_pill = gtk::Box::builder().css_classes(["viewer-pill"]).build();
        subfolders_pill.append(&subfolders);

        let close = gtk::Button::builder()
            .css_classes(["viewer-toolbar-button"])
            .tooltip_text("Close (Ctrl+W)")
            .child(&icon("xmark-awesome-symbolic", 16))
            .build();
        let end = gtk::Box::builder().spacing(8).build();
        end.append(&close);

        let root = gtk::Box::builder()
            .css_classes(["viewer-toolbar"])
            .spacing(8)
            .build();
        root.append(&entry);
        root.append(&sorts);
        root.append(&subfolders_pill);
        root.append(&end);
        Self {
            root,
            entry,
            sort_buttons,
            direction,
            subfolders,
            close,
            end,
        }
    }
}

impl Window {
    pub(super) fn connect_toolbar(self: &Rc<Self>) {
        let toolbar = &self.toolbar;
        // Typing again after a path that wasn't a folder.
        toolbar
            .entry
            .connect_changed(|entry| entry.remove_css_class("error"));
        // Leaving the entry (a click in the grid) puts the folder shown back.
        // By the window's focus widget, which stays while another window has
        // the focus, so what's typed survives a look elsewhere.
        let weak = Rc::downgrade(self);
        self.window.connect_focus_widget_notify(move |window| {
            if let Some(this) = weak.upgrade()
                && gtk::prelude::GtkWindowExt::focus(window).is_some()
                && !this.editing_directory()
                && *this.toolbar.entry.text() != *this.folder.directory().to_string_lossy()
            {
                this.reset_entry();
            }
        });
        let weak = Rc::downgrade(self);
        toolbar.entry.connect_activate(move |entry| {
            if let Some(this) = weak.upgrade() {
                this.open_directory(&entry.text(), None);
            }
        });

        for (key, button) in &toolbar.sort_buttons {
            let (weak, key) = (Rc::downgrade(self), *key);
            button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.keeping_selection(|folder| folder.set_sort(key));
                }
            });
        }
        let weak = Rc::downgrade(self);
        toolbar.direction.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.keeping_selection(Folder::toggle_direction);
            }
        });
        let weak = Rc::downgrade(self);
        toolbar.subfolders.connect_toggled(move |button| {
            if let Some(this) = weak.upgrade() {
                this.set_recursive(button.is_active());
            }
        });
        let window = self.window.clone();
        toolbar.close.connect_clicked(move |_| window.close());
    }

    pub(super) fn sync_sort_buttons(&self) {
        let current = self.folder.sort_key();
        for (key, button) in &self.toolbar.sort_buttons {
            if *key == current {
                button.add_css_class("active");
            } else {
                button.remove_css_class("active");
            }
        }
        let descending = self.folder.descending();
        let direction = &self.toolbar.direction;
        direction.set_label(if descending { "↓" } else { "↑" });
        direction.set_sensitive(current != SortKey::Random);
        direction.set_tooltip_text(Some(if descending {
            "Descending"
        } else {
            "Ascending"
        }));
    }

    // Sorting again keeps the selected image selected (and in view).
    fn keeping_selection(&self, sort: impl FnOnce(&Folder)) {
        let selected = self.selected_path();
        sort(&self.folder);
        let position = selected
            .and_then(|path| self.folder.position(&path))
            .unwrap_or(0);
        if self.selection.n_items() > 0 {
            self.select(position, false);
        }
        self.sync_sort_buttons();
    }

    pub(super) fn reset_entry(&self) {
        let entry = &self.toolbar.entry;
        entry.set_text(&self.folder.directory().to_string_lossy());
        // A path longer than the entry shows its end, the folder's name.
        entry.set_position(-1);
        entry.remove_css_class("error");
    }

    pub(super) fn editing_directory(&self) -> bool {
        let entry = &self.toolbar.entry;
        gtk::prelude::GtkWindowExt::focus(&self.window)
            .is_some_and(|focus| focus == *entry || focus.is_ancestor(entry))
    }
}
