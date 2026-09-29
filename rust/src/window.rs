//! A viewer window: the grid's page (the folder entry and its history, sort
//! and subfolder buttons at the top; the selected image's name and
//! resolution and View at the bottom) and the full-screen view's;
//! opening, closing, the keys. As Window.tsx.

use crate::{
    actions::{self, ActionsMenu, MenuAction},
    history::History,
    library::{Finished, Folder, SortKey, image},
    preview::Preview,
    tiles::Tiles,
    trash::{self, TrashedItem},
    view::View,
};
use gtk::{gdk, gio, glib, pango, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
};

const APP_TITLE: &str = "Vitrine (spike)";
// As in the TypeScript app (Window.tsx, style.scss).
const TILE_WIDTH: i32 = 272;
const TILE_HEIGHT: i32 = 153;
const TOAST_DURATION: std::time::Duration = std::time::Duration::from_secs(2);
const UNDO_TOAST_DURATION: std::time::Duration = std::time::Duration::from_secs(5);
const NO_GVFS: &str = "Moving to the trash needs GVfs, which isn't available";
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

pub struct Window {
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    grid: gtk::GridView,
    selection: gtk::SingleSelection,
    folder: Rc<Folder>,
    tiles: Rc<Tiles>,
    preview: Rc<Preview>,
    view: Rc<View>,
    history: RefCell<History>,
    entry: gtk::Entry,
    history_panel: gtk::Popover,
    history_list: gtk::Box,
    empty: gtk::Label,
    filename: gtk::Label,
    resolution: gtk::Label,
    sort_buttons: Vec<(SortKey, gtk::Button)>,
    direction: gtk::Button,
    // An image to select once the scan finds it: the file opened, or the
    // selection kept while the subfolders are added or left out.
    pending: RefCell<Option<PathBuf>>,
    // The user clicked, typed or scrolled in the grid since the folder began
    // loading (see `keep_first_while_loading`).
    touched: Cell<bool>,
    // The last image selected (not by a rescan) and its position: if a
    // rescan removes or replaces it, the same image, or the one now in its
    // place, is selected again.
    last_selected: RefCell<Option<(PathBuf, u32)>>,
    // The images the running rescan found changed on disk.
    modified: RefCell<HashSet<PathBuf>>,
    // Width × height from each file's header, read off the main thread.
    resolutions: RefCell<HashMap<PathBuf, Option<(i32, i32)>>>,
    // The ⋯ menu (its actions are the window's), made once the window is.
    actions: RefCell<Option<ActionsMenu>>,
    toast: gtk::Box,
    toast_label: gtk::Label,
    toast_undo: gtk::Button,
    can_trash: bool,
    // The deletes to undo, most recent last; undos run one at a time, in
    // order (`undo_requests`).
    undo_stack: RefCell<Vec<TrashedItem>>,
    undo_requests: async_channel::Sender<()>,
    undo_receiver: async_channel::Receiver<()>,
    toast_timeout: RefCell<Option<glib::SourceId>>,
}

impl Window {
    /// A window on `directory` (and its subfolders with `subfolders`); with
    /// `file`, that image opens in the full-screen view.
    pub fn new(
        app: &gtk::Application,
        directory: &Path,
        file: Option<PathBuf>,
        subfolders: bool,
    ) -> Rc<Self> {
        let folder = Folder::new(subfolders);
        let tiles = Tiles::new(glib::user_cache_dir().join("vitrine-spike/thumbnails"));
        let selection = gtk::SingleSelection::new(Some(folder.sorted.clone()));
        let grid = build_grid(&selection, &tiles);
        let preview = Preview::new();

        // The toolbar.
        let entry = gtk::Entry::builder()
            .css_classes(["viewer-directory"])
            .primary_icon_name("folder-awesome-symbolic")
            .secondary_icon_name("chevron-down-awesome-symbolic")
            .secondary_icon_tooltip_text("Recent folders (↓)")
            .tooltip_text("Folder (Enter to open)")
            .width_chars(36)
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
        let direction = gtk::Button::with_label("↑");
        sorts.append(&direction);
        let subfolders_button = gtk::ToggleButton::builder()
            .label("Subfolders")
            .active(subfolders)
            .tooltip_text("Include images in subfolders")
            .build();
        let subfolders_pill = gtk::Box::builder().css_classes(["viewer-pill"]).build();
        subfolders_pill.append(&subfolders_button);
        let close_button = gtk::Button::builder()
            .css_classes(["viewer-toolbar-button"])
            .tooltip_text("Close (Esc)")
            .child(&icon("xmark-awesome-symbolic", 16))
            .build();
        let end = gtk::Box::builder()
            .hexpand(true)
            .halign(gtk::Align::End)
            .spacing(8)
            .build();
        end.append(&close_button);
        let toolbar = gtk::Box::builder()
            .css_classes(["viewer-toolbar"])
            .spacing(8)
            .build();
        toolbar.append(&entry);
        toolbar.append(&sorts);
        toolbar.append(&subfolders_pill);
        toolbar.append(&end);

        // The grid, the history panel over it, and the empty folder's label.
        let scrolled = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&grid)
            .build();
        // The recent folders, under the entry. A popover closes itself on a
        // click outside or Esc.
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
        let empty = gtk::Label::builder()
            .css_classes(["viewer-empty"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        let grid_overlay = gtk::Overlay::builder()
            .hexpand(true)
            .vexpand(true)
            .child(&scrolled)
            .build();
        grid_overlay.add_overlay(&empty);

        // The info bar.
        let filename = gtk::Label::builder()
            .label("No image selected")
            .ellipsize(pango::EllipsizeMode::End)
            .build();
        let filename_button = gtk::Button::builder()
            .css_classes(["flat", "viewer-filename-button"])
            .tooltip_text("Show in file manager")
            .child(&filename)
            .build();
        let resolution = gtk::Label::new(Some("0 × 0"));
        let labels = gtk::Box::builder()
            .hexpand(true)
            .halign(gtk::Align::Start)
            .spacing(8)
            .build();
        labels.append(&filename_button);
        labels.append(&resolution);
        let view_button = gtk::Button::builder()
            .label("View")
            .tooltip_text("Full-screen view (Enter)")
            .build();
        let actions = gtk::Box::builder()
            .halign(gtk::Align::End)
            .spacing(6)
            .build();
        actions.append(&view_button);
        let info = gtk::Box::builder()
            .css_classes(["viewer-info"])
            .spacing(8)
            .build();
        info.append(&labels);
        info.append(&actions);

        let library_page = gtk::Box::builder()
            .css_classes(["viewer-library"])
            .orientation(gtk::Orientation::Vertical)
            .build();
        library_page.append(&toolbar);
        library_page.append(
            &gtk::Separator::builder()
                .css_classes(["viewer-toolbar-separator"])
                .orientation(gtk::Orientation::Horizontal)
                .build(),
        );
        library_page.append(&grid_overlay);
        library_page.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        library_page.append(&info);

        let stack = gtk::Stack::builder().hexpand(true).vexpand(true).build();
        stack.add_named(&library_page, Some("grid"));
        // Brief messages (copied, failed) over both pages.
        let toast_label = gtk::Label::new(None);
        let toast = gtk::Box::builder()
            .css_classes(["viewer-toast"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Start)
            .margin_top(72)
            .spacing(12)
            .visible(false)
            .build();
        toast.append(&toast_label);
        let toast_undo = gtk::Button::builder()
            .label("Undo")
            .tooltip_text("Undo (Ctrl+Z)")
            .visible(false)
            .build();
        toast.append(&toast_undo);
        let (undo_requests, undo_receiver) = async_channel::unbounded();
        let overlay = gtk::Overlay::builder().child(&stack).build();
        overlay.add_overlay(&toast);
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title(APP_TITLE)
            .default_width(1600)
            .default_height(1000)
            .child(&overlay)
            .build();
        let view = View::new(&window, &stack, &preview);
        stack.add_named(&view.page, Some("preview"));

        let this = Rc::new(Self {
            window,
            stack,
            grid,
            selection,
            folder,
            tiles,
            preview,
            view,
            history: RefCell::new(History::load()),
            entry,
            history_panel,
            history_list,
            empty,
            filename,
            resolution,
            sort_buttons,
            direction,
            pending: RefCell::default(),
            touched: Cell::new(false),
            last_selected: RefCell::default(),
            modified: RefCell::default(),
            resolutions: RefCell::default(),
            actions: RefCell::default(),
            toast,
            toast_label,
            toast_undo,
            can_trash: trash::available(),
            undo_stack: RefCell::default(),
            undo_requests,
            undo_receiver,
            toast_timeout: RefCell::default(),
        });

        this.connect_toolbar(&subfolders_button, &close_button);
        this.connect_history();
        this.connect_info(&filename_button, &view_button);
        this.connect_preview();
        this.connect_keys();
        this.connect_trash();
        this.build_menu(&end);
        this.keep_first_while_loading();
        // A file changed on disk: its decoded image and size are out of date.
        let weak = Rc::downgrade(&this);
        this.folder.connect_modified(move |path| {
            if let Some(this) = weak.upgrade() {
                this.preview.forget(path);
                this.resolutions.borrow_mut().remove(path);
                this.modified.borrow_mut().insert(path.to_owned());
            }
        });
        let weak = Rc::downgrade(&this);
        this.folder.connect_finished(move |finished| {
            if let Some(this) = weak.upgrade() {
                this.finished(finished);
            }
        });
        let weak = Rc::downgrade(&this);
        this.selection.connect_items_changed(move |_, _, _, _| {
            if let Some(this) = weak.upgrade() {
                this.sync_info();
            }
        });

        // The handlers hold weak references; the window keeps this alive
        // until it's destroyed.
        let keep = RefCell::new(Some(this.clone()));
        this.window.connect_destroy(move |_| {
            if let Some(this) = keep.take() {
                this.history_panel.unparent();
                this.folder.dispose();
            }
        });

        this.render_history();
        this.sync_sort_buttons();
        // A file opens in the full-screen view straight away; the scan
        // selects it in the grid when it turns up.
        if let Some(file) = &file {
            let (width, height) = view_size(&this.window);
            this.preview
                .show(file.clone(), Vec::new(), None, width, height);
            this.stack.set_visible_child_name("preview");
        }
        let directory = directory.to_string_lossy();
        if !this.open_directory(&directory, file) {
            this.open_directory(&glib::home_dir().to_string_lossy(), None);
        }
        this.window.present();
        this.grid.grab_focus();
        match std::env::var("VITRINE_PROBE").as_deref() {
            Ok("ui") => crate::probe::ui(&this.window),
            Ok(_) => crate::probe::run(&this.window),
            Err(_) => {}
        }
        this
    }

    fn selected_path(&self) -> Option<PathBuf> {
        self.selection
            .selected_item()
            .map(|object| image(&object).path.clone())
    }

    // Selects `position` and scrolls to it (focusing it with `focus`).
    fn select(&self, position: u32, focus: bool) {
        self.selection.set_selected(position);
        let flags = if focus {
            gtk::ListScrollFlags::FOCUS
        } else {
            gtk::ListScrollFlags::NONE
        };
        self.grid.scroll_to(position, flags, None);
    }

    fn in_grid(&self) -> bool {
        self.stack.visible_child_name().as_deref() == Some("grid")
    }

    // --- folder --------------------------------------------------------------

    fn open_directory(self: &Rc<Self>, input: &str, select: Option<PathBuf>) -> bool {
        let path = normalize_directory(input);
        if !path.is_dir() {
            self.entry.add_css_class("error");
            return false;
        }
        *self.pending.borrow_mut() = select;
        self.touched.set(false);
        self.folder.load(path.clone());
        self.reset_entry();
        self.history.borrow_mut().remember(&path);
        self.render_history();
        self.grid.grab_focus();
        self.sync_info();
        true
    }

    fn set_recursive(self: &Rc<Self>, recursive: bool) {
        self.folder.set_recursive(recursive);
        *self.pending.borrow_mut() = self.selected_path();
        self.touched.set(false);
        self.folder.load(self.folder.directory());
        self.sync_info();
    }

    fn rescan(&self) {
        self.folder.rescan();
    }

    fn finished(self: &Rc<Self>, finished: Finished) {
        let count = self.selection.n_items();
        match finished {
            Finished::Load => {
                let pending = self.pending.borrow_mut().take();
                match pending.and_then(|path| self.folder.position(&path)) {
                    Some(position) => self.select(position, self.in_grid()),
                    None if count > 0 && !self.touched.get() => self.select(0, self.in_grid()),
                    None => {}
                }
            }
            Finished::Rescan => {
                let modified = std::mem::take(&mut *self.modified.borrow_mut());
                let last = self.last_selected.borrow().clone();
                if let Some((path, index)) = last
                    && count > 0
                    && self.selected_path().as_ref() != Some(&path)
                {
                    let position = self.folder.position(&path).unwrap_or(index.min(count - 1));
                    // Without scrolling, so browsing the grid isn't
                    // interrupted.
                    self.selection.set_selected(position);
                }
                if !self.in_grid() {
                    if count == 0 {
                        self.close_view();
                    } else if self.preview.shown() != self.selected_path()
                        || self
                            .preview
                            .shown()
                            .is_some_and(|shown| modified.contains(&shown))
                    {
                        // Only when its image went or changed: rescans after
                        // changes elsewhere in the folder mustn't reset the
                        // zoom.
                        self.show_at(self.selection.selected() as i64);
                    }
                }
            }
        }
        self.tiles.set_background(&self.folder.images());
        self.sync_info();
    }

    fn reset_entry(&self) {
        self.entry
            .set_text(&self.folder.directory().to_string_lossy());
        self.entry.remove_css_class("error");
    }

    fn editing_directory(&self) -> bool {
        gtk::prelude::GtkWindowExt::focus(&self.window)
            .is_some_and(|focus| focus == self.entry || focus.is_ancestor(&self.entry))
    }

    // --- toolbar -------------------------------------------------------------

    fn connect_toolbar(self: &Rc<Self>, subfolders: &gtk::ToggleButton, close: &gtk::Button) {
        let weak = Rc::downgrade(self);
        self.entry.connect_activate(move |entry| {
            if let Some(this) = weak.upgrade() {
                this.open_directory(&entry.text(), None);
            }
        });
        let weak = Rc::downgrade(self);
        self.entry.connect_icon_release(move |_, position| {
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
        self.entry.add_controller(keys);

        for (key, button) in &self.sort_buttons {
            let (weak, key) = (Rc::downgrade(self), *key);
            button.connect_clicked(move |_| {
                if let Some(this) = weak.upgrade() {
                    this.keeping_selection(|folder| folder.set_sort(key));
                }
            });
        }
        let weak = Rc::downgrade(self);
        self.direction.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.keeping_selection(Folder::toggle_direction);
            }
        });
        let weak = Rc::downgrade(self);
        subfolders.connect_toggled(move |button| {
            if let Some(this) = weak.upgrade() {
                this.set_recursive(button.is_active());
            }
        });
        let window = self.window.clone();
        close.connect_clicked(move |_| window.close());
    }

    fn sync_sort_buttons(&self) {
        let current = self.folder.sort_key();
        for (key, button) in &self.sort_buttons {
            if *key == current {
                button.add_css_class("active");
            } else {
                button.remove_css_class("active");
            }
        }
        let descending = self.folder.descending();
        self.direction.set_label(if descending { "↓" } else { "↑" });
        self.direction.set_sensitive(current != SortKey::Random);
        self.direction.set_tooltip_text(Some(if descending {
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

    // --- history panel ---------------------------------------------------------

    fn connect_history(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        let list = self.history_list.clone();
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
        self.history_list.add_controller(keys);

        // Back to the entry (Esc, a click outside); a chosen folder moves
        // the focus on to the grid afterwards.
        let entry = self.entry.clone();
        self.history_panel.connect_closed(move |_| {
            entry.grab_focus();
        });
    }

    fn render_history(self: &Rc<Self>) {
        while let Some(child) = self.history_list.first_child() {
            self.history_list.remove(&child);
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
                    this.hide_history();
                    this.open_directory(&text, None);
                }
            });
            self.history_list.append(&button);
        }
    }

    fn hide_history(&self) {
        self.history_panel.popdown();
    }

    // Opens the panel with the current folder's entry focused.
    fn show_history(&self) {
        self.history_list.set_width_request(self.entry.width());
        self.history_panel.popup();
        let directory = self.folder.directory();
        let current = self
            .history
            .borrow()
            .entries
            .iter()
            .position(|entry| *entry == directory)
            .unwrap_or(0);
        let mut button = self.history_list.first_child();
        for _ in 0..current {
            button = button.and_then(|button| button.next_sibling());
        }
        if let Some(button) = button.or_else(|| self.history_list.first_child()) {
            button.grab_focus();
        }
    }

    fn toggle_history(&self) {
        if self.history_panel.is_visible() {
            self.hide_history();
        } else {
            self.show_history();
        }
    }

    // --- info bar ------------------------------------------------------------

    fn connect_info(self: &Rc<Self>, filename: &gtk::Button, view: &gtk::Button) {
        let weak = Rc::downgrade(self);
        filename.connect_clicked(move |_| {
            if let Some(path) = weak.upgrade().and_then(|this| this.selected_path()) {
                show_in_file_manager(&path);
            }
        });
        let weak = Rc::downgrade(self);
        view.connect_clicked(move |_| {
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

    fn sync_info(self: &Rc<Self>) {
        let path = self.selected_path();
        let name = path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned());
        self.filename
            .set_label(name.as_deref().unwrap_or("No image selected"));
        self.resolution.set_label(&match &path {
            Some(path) => self.resolution_of(path),
            None => "0 × 0".into(),
        });
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

    // The size in the file's header, read on a worker the first time (a
    // slow disk mustn't hold up moving the selection).
    fn resolution_of(self: &Rc<Self>, path: &Path) -> String {
        let format = |size: Option<(i32, i32)>| match size {
            Some((width, height)) => format!("{width} × {height}"),
            None => "0 × 0".into(),
        };
        if let Some(size) = self.resolutions.borrow().get(path) {
            return format(*size);
        }
        let (weak, path) = (Rc::downgrade(self), path.to_owned());
        glib::spawn_future_local(async move {
            let read = path.clone();
            let size = gio::spawn_blocking(move || {
                gtk::gdk_pixbuf::Pixbuf::file_info(&read).map(|(_, width, height)| (width, height))
            })
            .await
            .ok()
            .flatten();
            let Some(this) = weak.upgrade() else { return };
            this.resolutions.borrow_mut().insert(path.clone(), size);
            if this.selected_path().as_ref() == Some(&path) {
                this.resolution.set_label(&format(size));
            }
        });
        "…".into()
    }

    // --- full-screen view ------------------------------------------------------

    // Shows the image at `position` (wrapping around) in the view.
    fn show_at(&self, position: i64) {
        let count = self.selection.n_items() as i64;
        if count == 0 {
            return;
        }
        let at = |offset: i64| {
            self.selection
                .item((position + offset).rem_euclid(count) as u32)
                .map(|object| image(&object).clone())
        };
        let Some(shown) = at(0) else { return };
        self.selection
            .set_selected(position.rem_euclid(count) as u32);
        // Nearest first, the way ←/→ would reach them.
        let neighbours = [1, -1, 2, -2]
            .into_iter()
            .filter_map(at)
            .map(|image| image.path)
            .collect();
        let (width, height) = view_size(&self.window);
        self.preview.show(
            shown.path.clone(),
            neighbours,
            self.tiles.cached(&shown),
            width,
            height,
        );
        self.stack.set_visible_child_name("preview");
    }

    fn close_view(&self) {
        self.view.leave();
        self.stack.set_visible_child_name("grid");
        let selected = self.selection.selected();
        // Nothing selected when the last image was deleted.
        if selected < self.selection.n_items() {
            self.grid
                .scroll_to(selected, gtk::ListScrollFlags::FOCUS, None);
        } else {
            self.grid.grab_focus();
        }
        hide_after_paint(&self.window, &self.stack, &self.selection, &self.preview);
    }

    // Opening (Enter, double-click, View), the close button, and clicks at
    // the view's edges.
    fn connect_preview(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.grid.connect_activate(move |_, position| {
            if let Some(this) = weak.upgrade() {
                this.show_at(position as i64);
            }
        });

        // The image selected in the grid is decoded in the background, so
        // opening it shows it sharp straight away. (In the view, `show`
        // preloads the neighbours instead.)
        let weak = Rc::downgrade(self);
        self.selection
            .connect_selected_item_notify(move |selection| {
                let Some(this) = weak.upgrade() else { return };
                if !this.in_grid() {
                    return;
                }
                if let Some(object) = selection.selected_item() {
                    let (width, height) = view_size(&this.window);
                    this.preview
                        .preload(image(&object).path.clone(), width, height);
                }
            });

        let weak = Rc::downgrade(self);
        self.view.connect_close(move || {
            if let Some(this) = weak.upgrade() {
                this.close_view();
            }
        });

        // At fit, a click in the left/right edge moves to the previous/next.
        let weak = Rc::downgrade(self);
        self.preview.image.connect_navigate(move |side| {
            if let Some(this) = weak.upgrade() {
                this.show_at(this.selection.selected() as i64 + side as i64);
            }
        });
    }

    // --- menu and actions ----------------------------------------------------

    // The ⋯ menu, in the toolbar and in the view; the view's own actions
    // (rotate, flip) only there.
    fn build_menu(self: &Rc<Self>, toolbar_end: &gtk::Box) {
        let action = |name, label, run: fn(&Rc<Self>)| {
            let weak = Rc::downgrade(self);
            MenuAction::new(name, label, move || {
                if let Some(this) = weak.upgrade() {
                    run(&this);
                }
            })
        };
        let menu = ActionsMenu::new(
            &self.window,
            vec![
                vec![
                    action("copy-image", "Copy image", Self::copy_image).accel("<Control>c"),
                    action("copy-path", "Copy path", Self::copy_path).accel("<Control><Shift>c"),
                ],
                vec![
                    action("rotate-left", "Rotate left", |this| {
                        this.preview.image.rotate(false)
                    })
                    .accel("bracketleft")
                    .view_only(),
                    action("rotate-right", "Rotate right", |this| {
                        this.preview.image.rotate(true)
                    })
                    .accel("bracketright")
                    .view_only(),
                    action("flip-horizontally", "Flip horizontally", |this| {
                        this.preview.image.flip(true)
                    })
                    .accel("h")
                    .view_only(),
                    action("flip-vertically", "Flip vertically", |this| {
                        this.preview.image.flip(false)
                    })
                    .accel("v")
                    .view_only(),
                ],
                vec![
                    action("show-in-file-manager", "Show in file manager", |this| {
                        if let Some(path) = this.selected_path() {
                            show_in_file_manager(&path);
                        }
                    }),
                    action("rescan", "Rescan folder", |this| this.rescan()).accel("r"),
                ],
                vec![
                    action("shortcuts", "Keyboard shortcuts", |this| {
                        crate::shortcuts::show(&this.window)
                    })
                    .accel("question"),
                ],
            ],
        );
        let button = actions::more_button(&menu.model, 16);
        button.add_css_class("viewer-toolbar-menu");
        toolbar_end.prepend(&button);
        self.view.add_menu(&menu.model);
        let weak = Rc::downgrade(self);
        self.stack.connect_visible_child_name_notify(move |_| {
            if let Some(this) = weak.upgrade()
                && let Some(menu) = this.actions.borrow().as_ref()
            {
                menu.set_in_view(!this.in_grid());
            }
        });
        *self.actions.borrow_mut() = Some(menu);
    }

    // Decodes the whole image (a GIF's first frame) on a worker for the
    // clipboard.
    fn copy_image(self: &Rc<Self>) {
        let Some(path) = self.selected_path() else {
            return;
        };
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let read = path.clone();
            let decoded = gio::spawn_blocking(move || {
                crate::decode::to_fit(&read, u32::MAX, u32::MAX)
                    .map(|fitted| fitted.rgba.premultiplied())
            })
            .await
            .unwrap_or_else(|_| Err("the decode panicked".into()));
            let Some(this) = weak.upgrade() else { return };
            match decoded {
                Ok(pixels) => {
                    this.window.clipboard().set_texture(&pixels.texture());
                    this.show_toast("Copied image", false);
                }
                Err(error) => {
                    eprintln!("Could not copy {}: {error}", path.display());
                    this.show_toast("Could not copy image", false);
                }
            }
        });
    }

    fn copy_path(self: &Rc<Self>) {
        if let Some(path) = self.selected_path() {
            self.window.clipboard().set_text(&path.to_string_lossy());
            self.show_toast("Copied path", false);
        }
    }

    // With `undoable`, the Undo button, and longer.
    fn show_toast(self: &Rc<Self>, text: &str, undoable: bool) {
        self.toast_label.set_label(text);
        self.toast_undo.set_visible(undoable);
        self.toast.set_visible(true);
        if let Some(id) = self.toast_timeout.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let duration = if undoable {
            UNDO_TOAST_DURATION
        } else {
            TOAST_DURATION
        };
        let id = glib::timeout_add_local_once(duration, move || {
            if let Some(this) = weak.upgrade() {
                this.toast_timeout.borrow_mut().take();
                this.toast.set_visible(false);
            }
        });
        *self.toast_timeout.borrow_mut() = Some(id);
    }

    // --- trash and undo --------------------------------------------------------

    fn connect_trash(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.toast_undo.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.undo_delete();
            }
        });
        // Undos one at a time: repeated presses go back through the deletes
        // in order.
        let (weak, requests) = (Rc::downgrade(self), self.undo_receiver.clone());
        glib::spawn_future_local(async move {
            while requests.recv().await.is_ok() {
                let Some(this) = weak.upgrade() else { return };
                this.undo_one().await;
            }
        });
    }

    // Takes the image out of the grid straight away (so repeated Delete keeps
    // going) and puts it back if trashing fails.
    fn delete_selected(self: &Rc<Self>) {
        if !self.can_trash {
            return self.show_toast(NO_GVFS, false);
        }
        let Some(path) = self.selected_path() else {
            return;
        };
        if let Some(position) = self.folder.remove(&path) {
            let count = self.selection.n_items();
            if count > 0 {
                self.select(position.min(count - 1), self.in_grid());
            }
        }
        self.refresh_view_if_open();
        self.sync_info();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = trash::move_to_trash(&path).await;
            let Some(this) = weak.upgrade() else { return };
            let name = file_name(&path);
            match result {
                Ok(Some(item)) => {
                    this.undo_stack.borrow_mut().push(item);
                    this.show_toast(&format!("Moved {name} to the trash"), true);
                }
                Ok(None) => this.show_toast(
                    &format!("Moved {name} to the trash (can't be undone)"),
                    false,
                ),
                Err(error) => {
                    eprintln!("Could not delete {}: {error}", path.display());
                    this.show_toast(&format!("Could not move {name} to the trash"), false);
                    this.reinsert(&path);
                }
            }
        });
    }

    fn undo_delete(&self) {
        if !self.can_trash {
            return;
        }
        let _ = self.undo_requests.try_send(());
    }

    // Restores the most recent delete.
    async fn undo_one(self: &Rc<Self>) {
        if !self.can_trash {
            return self.show_toast(NO_GVFS, false);
        }
        let Some(item) = self.undo_stack.borrow_mut().pop() else {
            return self.show_toast("Nothing to undo", false);
        };
        let name = file_name(&item.original);
        if let Err(reason) = trash::restore(&item).await {
            return self.show_toast(&format!("Couldn't restore {name}: {reason}"), false);
        }
        if self.reinsert(&item.original).is_some() {
            self.show_toast(&format!("Restored {name}"), false);
        } else {
            let folder = item.original.parent().unwrap_or(Path::new("/"));
            self.show_toast(&format!("Restored {name} to {}", folder.display()), false);
        }
    }

    // Puts `path` back in the grid, selected, if it belongs to this folder.
    fn reinsert(self: &Rc<Self>, path: &Path) -> Option<u32> {
        let position = self.folder.add(path);
        if let Some(position) = position {
            self.select(position, self.in_grid());
        }
        self.refresh_view_if_open();
        self.sync_info();
        position
    }

    // The view follows the selection after a delete or restore; with nothing
    // left, back to the grid.
    fn refresh_view_if_open(&self) {
        if self.in_grid() {
            return;
        }
        if self.selection.n_items() == 0 {
            self.close_view();
        } else if self.preview.shown() != self.selected_path() {
            self.show_at(self.selection.selected() as i64);
        }
    }

    // --- keys ------------------------------------------------------------------

    fn connect_keys(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::builder()
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| match weak.upgrade() {
            Some(this) if this.key(key, state) => glib::Propagation::Stop,
            _ => glib::Propagation::Proceed,
        });
        self.window.add_controller(keys);
    }

    // Whether `key` was handled.
    fn key(self: &Rc<Self>, key: gdk::Key, state: gdk::ModifierType) -> bool {
        if self.editing_directory() {
            if key != gdk::Key::Escape {
                return false;
            }
            self.reset_entry();
            self.grid.grab_focus();
            return true;
        }
        if state.contains(gdk::ModifierType::CONTROL_MASK)
            && matches!(key, gdk::Key::z | gdk::Key::Z)
        {
            self.undo_delete();
            return true;
        }
        if state.contains(gdk::ModifierType::CONTROL_MASK)
            && matches!(key, gdk::Key::c | gdk::Key::C)
        {
            if state.contains(gdk::ModifierType::SHIFT_MASK) {
                self.copy_path();
            } else {
                self.copy_image();
            }
            return true;
        }
        if state.contains(gdk::ModifierType::CONTROL_MASK)
            && matches!(key, gdk::Key::w | gdk::Key::W | gdk::Key::q | gdk::Key::Q)
        {
            self.window.close();
            return true;
        }
        // Everything else is a plain key (Shift is allowed, for + and ?), so
        // e.g. Ctrl+R stays free.
        let held = gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::ALT_MASK
            | gdk::ModifierType::SUPER_MASK;
        if state.intersects(held) {
            return false;
        }
        if matches!(key, gdk::Key::r | gdk::Key::R) {
            self.rescan();
            return true;
        }
        if matches!(key, gdk::Key::Delete | gdk::Key::KP_Delete) {
            self.delete_selected();
            return true;
        }
        if key == gdk::Key::question {
            crate::shortcuts::show(&self.window);
            return true;
        }
        let selected = self.selection.selected() as i64;
        if self.in_grid() {
            match key {
                gdk::Key::e | gdk::Key::E if self.selection.selected_item().is_some() => {
                    self.show_at(selected)
                }
                gdk::Key::Escape | gdk::Key::q => self.window.close(),
                _ => return false,
            }
            return true;
        }
        match key {
            gdk::Key::Right => self.show_at(selected + 1),
            gdk::Key::Left => self.show_at(selected - 1),
            gdk::Key::Escape | gdk::Key::q => self.close_view(),
            _ => return self.view.key(key),
        }
        true
    }

    // While a folder loads, keeps the first image selected and the grid at
    // the top, until the user clicks, types or scrolls in it. Batches arrive
    // in any order and are sorted as they come; the automatic selection
    // would otherwise stay on whichever image arrived first, wherever sorting
    // put it, and the grid kept it in view (opening halfway down a big
    // folder). An image to select (`pending`) is selected once the load
    // finishes (`finished`).
    fn keep_first_while_loading(self: &Rc<Self>) {
        // Any press, key or scroll in the grid (seen, not consumed).
        let events = gtk::EventControllerLegacy::new();
        events.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        events.connect_event(move |_, event| {
            use gdk::EventType::*;
            if let Some(this) = weak.upgrade()
                && matches!(
                    event.event_type(),
                    ButtonPress | KeyPress | Scroll | TouchBegin
                )
            {
                this.touched.set(true);
            }
            glib::Propagation::Proceed
        });
        self.grid.add_controller(events);
        let weak = Rc::downgrade(self);
        self.selection
            .connect_items_changed(move |selection, _, _, _| {
                let Some(this) = weak.upgrade() else { return };
                if !this.folder.is_loading() || this.touched.get() || selection.n_items() == 0 {
                    return;
                }
                if selection.selected() != 0 {
                    selection.set_selected(0);
                }
                this.grid.scroll_to(0, gtk::ListScrollFlags::NONE, None);
            });
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

fn icon(name: &str, size: i32) -> gtk::Image {
    gtk::Image::builder()
        .icon_name(name)
        .pixel_size(size)
        .build()
}

fn build_grid(selection: &gtk::SingleSelection, tiles: &Rc<Tiles>) -> gtk::GridView {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let picture = gtk::Picture::builder()
            .css_classes(["viewer-thumbnail"])
            .width_request(TILE_WIDTH)
            .height_request(TILE_HEIGHT)
            .content_fit(gtk::ContentFit::Contain)
            .can_shrink(true)
            .build();
        item.downcast_ref::<gtk::ListItem>()
            .expect("a ListItem")
            .set_child(Some(&picture));
    });
    let bind_tiles = tiles.clone();
    factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a ListItem");
        let picture = item
            .child()
            .and_downcast::<gtk::Picture>()
            .expect("a Picture");
        let object = item.item().expect("a bound item");
        bind_tiles.bind(&picture, &image(&object));
    });
    let unbind_tiles = tiles.clone();
    factory.connect_unbind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a ListItem");
        if let Some(picture) = item.child().and_downcast::<gtk::Picture>() {
            unbind_tiles.unbind(&picture);
        }
    });
    gtk::GridView::builder()
        .css_classes(["viewer-grid"])
        .model(selection)
        .factory(&factory)
        .min_columns(1)
        .max_columns(12)
        .build()
}

// As util.ts: surrounding space trimmed, ~ expanded, relative to the current
// folder.
fn normalize_directory(input: &str) -> PathBuf {
    let input = input.trim();
    let expanded = match input.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            format!("{}{rest}", glib::home_dir().to_string_lossy())
        }
        _ => input.to_owned(),
    };
    let current = std::env::current_dir().unwrap_or_else(|_| glib::home_dir());
    // Like g_canonicalize_filename: . and .. resolved, symlinks kept.
    let mut path = PathBuf::new();
    for component in current.join(expanded).components() {
        match component {
            std::path::Component::ParentDir => {
                path.pop();
            }
            std::path::Component::CurDir => {}
            component => path.push(component),
        }
    }
    path
}

pub fn show_in_file_manager(path: &Path) {
    let uri = gio::File::for_path(path).uri().to_string();
    let path = path.to_owned();
    glib::spawn_future_local(async move {
        let shown = async {
            let bus = gio::bus_get_future(gio::BusType::Session).await?;
            bus.call_future(
                Some("org.freedesktop.FileManager1"),
                "/org/freedesktop/FileManager1",
                "org.freedesktop.FileManager1",
                "ShowItems",
                Some(&(vec![uri], "").to_variant()),
                None,
                gio::DBusCallFlags::NONE,
                -1,
            )
            .await
        };
        if let Err(error) = shown.await {
            eprintln!("Could not show {} in file manager: {error}", path.display());
        }
    });
}

// The size the view decodes images at: the window's, in device pixels
// (fractional scales included), so at fit an image is drawn at exactly its
// decoded pixels. Before the window is shown (the grid's first selection),
// its default size; the view decodes again when its size changes.
fn view_size(window: &gtk::ApplicationWindow) -> (u32, u32) {
    let (width, height) = if window.width() > 0 {
        (window.width(), window.height())
    } else {
        (window.default_width(), window.default_height())
    };
    let scale = window
        .surface()
        .map_or(window.scale_factor() as f64, |surface| surface.scale());
    (
        (width as f64 * scale).round().max(1.0) as u32,
        (height as f64 * scale).round().max(1.0) as u32,
    )
}

// Dropping the view's textures (all but the selected image's) once the grid's
// first frame is drawn, so freeing them doesn't delay it.
fn hide_after_paint(
    window: &gtk::ApplicationWindow,
    stack: &gtk::Stack,
    selection: &gtk::SingleSelection,
    preview: &Rc<Preview>,
) {
    let hide = {
        let (window, stack, selection, preview) = (
            window.clone(),
            stack.clone(),
            selection.clone(),
            preview.clone(),
        );
        move || {
            if stack.visible_child_name().as_deref() != Some("grid") {
                return;
            }
            let keep = selection
                .selected_item()
                .map(|object| image(&object).path.clone());
            let (width, height) = view_size(&window);
            preview.hide(keep, width, height);
        }
    };
    let Some(clock) = window.frame_clock() else {
        hide();
        return;
    };
    let handler = Rc::new(RefCell::new(None));
    let handler_ = handler.clone();
    let id = clock.connect_after_paint(move |clock| {
        if let Some(id) = handler_.borrow_mut().take() {
            clock.disconnect(id);
        }
        hide();
    });
    *handler.borrow_mut() = Some(id);
}
