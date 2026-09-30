//! The grid's toolbar: the folder entry with its history panel, the sort
//! pill, Subfolders and, at the end, the i and ⋯ buttons (`menu.rs`) and
//! close.

use super::{Window, icon};
use crate::library::{Folder, SortKey};
use gtk::{gdk, glib, pango, prelude::*};
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
    // The recent folders, under the entry. A popover closes itself on a
    // click outside or Esc.
    pub history_panel: gtk::Popover,
    history_list: gtk::Box,
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
            .primary_icon_name("folder-awesome-symbolic")
            .secondary_icon_name("chevron-down-awesome-symbolic")
            .secondary_icon_tooltip_text("Recent folders (↓)")
            .tooltip_text("Folder (Enter to open)")
            .width_chars(36)
            .build();
        let history_list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .build();
        let history_panel = gtk::Popover::builder()
            .css_classes(["viewer-history"])
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .child(&history_list)
            .build();
        history_panel.set_parent(&entry);

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
            .tooltip_text("Close (Esc)")
            .child(&icon("xmark-awesome-symbolic", 16))
            .build();
        let end = gtk::Box::builder()
            .hexpand(true)
            .halign(gtk::Align::End)
            .spacing(8)
            .build();
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
            history_panel,
            history_list,
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
        let weak = Rc::downgrade(self);
        toolbar.entry.connect_activate(move |entry| {
            if let Some(this) = weak.upgrade() {
                this.open_directory(&entry.text(), None);
            }
        });
        let weak = Rc::downgrade(self);
        toolbar.entry.connect_icon_release(move |_, position| {
            if let Some(this) = weak.upgrade()
                && position == gtk::EntryIconPosition::Secondary
            {
                this.toggle_history();
            }
        });
        let keys = gtk::EventControllerKey::builder()
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key != gdk::Key::Down {
                return glib::Propagation::Proceed;
            }
            if let Some(this) = weak.upgrade() {
                this.show_history();
            }
            glib::Propagation::Stop
        });
        toolbar.entry.add_controller(keys);

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
        self.connect_history();
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
        entry.remove_css_class("error");
    }

    pub(super) fn editing_directory(&self) -> bool {
        let entry = &self.toolbar.entry;
        gtk::prelude::GtkWindowExt::focus(&self.window)
            .is_some_and(|focus| focus == *entry || focus.is_ancestor(entry))
    }

    // --- history panel ---------------------------------------------------------

    fn connect_history(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        let list = self.toolbar.history_list.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            let direction = match key {
                gdk::Key::Down => gtk::DirectionType::TabForward,
                gdk::Key::Up => gtk::DirectionType::TabBackward,
                _ => return glib::Propagation::Proceed,
            };
            if list.child_focus(direction) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        self.toolbar.history_list.add_controller(keys);

        // Back to the entry (Esc, a click outside); a chosen folder moves
        // the focus on to the grid afterwards.
        let entry = self.toolbar.entry.clone();
        self.toolbar.history_panel.connect_closed(move |_| {
            entry.grab_focus();
        });
    }

    pub(super) fn render_history(self: &Rc<Self>) {
        let list = &self.toolbar.history_list;
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        for entry in &self.history.borrow().entries {
            let text = entry.to_string_lossy();
            let button = gtk::Button::builder()
                .child(
                    &gtk::Label::builder()
                        .label(&*text)
                        .xalign(0.0)
                        .ellipsize(pango::EllipsizeMode::Start)
                        .build(),
                )
                .tooltip_text(&*text)
                .build();
            let (weak, text) = (Rc::downgrade(self), text.into_owned());
            button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.toolbar.history_panel.popdown();
                    this.open_directory(&text, None);
                }
            });
            list.append(&button);
        }
    }

    // Opens the panel, as wide as the entry, with the current folder's entry
    // marked and focused.
    fn show_history(&self) {
        // The popover's padding and border (style.scss) on each side.
        const INSET: i32 = 2 * (6 + 1);
        let list = &self.toolbar.history_list;
        list.set_width_request(self.toolbar.entry.width() - INSET);
        self.toolbar.history_panel.popup();
        let directory = self.folder.directory();
        let current = self
            .history
            .borrow()
            .entries
            .iter()
            .position(|entry| *entry == directory);
        let mut button = list.first_child();
        let mut focus = None;
        for index in 0.. {
            let Some(widget) = button else { break };
            if Some(index) == current {
                widget.add_css_class("current");
                focus = Some(widget.clone());
            } else {
                widget.remove_css_class("current");
            }
            button = widget.next_sibling();
        }
        if let Some(button) = focus.or_else(|| list.first_child()) {
            button.grab_focus();
        }
    }

    fn toggle_history(&self) {
        if self.toolbar.history_panel.is_visible() {
            self.toolbar.history_panel.popdown();
        } else {
            self.show_history();
        }
    }
}
