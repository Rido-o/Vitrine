//! A viewer window: the grid's page (toolbar, grid, info bar) and the
//! full-screen view's, in one stack, with a toast over both. `Window` holds
//! the state; each part's widgets and handlers are in the submodules.

mod delete;
mod grid;
mod info;
mod keys;
mod menu;
mod navigation;
mod toast;
mod toolbar;

use self::{info::InfoBar, toast::Toast, toolbar::Toolbar};
use crate::{
    desktop::trash::TrashedItem,
    history::History,
    library::{Finished, Folder, image},
    thumbnails::tiles::Tiles,
    view::{View, preview::Preview},
};
use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
};

const APP_TITLE: &str = "Vitrine";

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
    toolbar: Toolbar,
    info: InfoBar,
    // The empty folder's message, over the grid.
    empty: gtk::Label,
    toast: Rc<Toast>,
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
    // The i buttons: the grid's and the view's.
    grid_properties: gtk::MenuButton,
    view_properties: gtk::MenuButton,
    // VITRINE_WALLPAPER_COMMAND, split into arguments; the image is appended.
    wallpaper_argv: Option<Vec<std::ffi::OsString>>,
    // The deletes to undo, most recent last; undos run one at a time, in
    // order (`undo_requests`).
    undo_stack: RefCell<Vec<TrashedItem>>,
    undo_requests: async_channel::Sender<()>,
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
        let tiles = Tiles::new(crate::thumbnails::cache_dir());
        let selection = gtk::SingleSelection::new(Some(folder.sorted.clone()));
        let grid = grid::build(&selection, &tiles);
        let preview = Preview::new();
        let toolbar = Toolbar::new(subfolders);
        let empty = gtk::Label::builder()
            .css_classes(["viewer-empty"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .visible(false)
            .build();
        let info = InfoBar::new();

        let library_page = gtk::Box::builder()
            .css_classes(["viewer-library"])
            .orientation(gtk::Orientation::Vertical)
            .build();
        library_page.append(&toolbar.root);
        library_page.append(
            &gtk::Separator::builder()
                .css_classes(["viewer-toolbar-separator"])
                .orientation(gtk::Orientation::Horizontal)
                .build(),
        );
        library_page.append(&grid::area(&grid, &empty));
        library_page.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        library_page.append(&info.root);

        let stack = gtk::Stack::builder().hexpand(true).vexpand(true).build();
        stack.add_named(&library_page, Some("grid"));
        let toast = Toast::new();
        let overlay = gtk::Overlay::builder().child(&stack).build();
        overlay.add_overlay(&toast.widget);
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title(APP_TITLE)
            .default_width(1600)
            .default_height(1000)
            .child(&overlay)
            .build();
        let view = View::new(&window, &stack, &preview);
        stack.add_named(&view.page, Some("preview"));
        let (undo_requests, undo_receiver) = async_channel::unbounded();
        let grid_properties = menu::properties_button(&selection, 16);
        grid_properties.add_css_class("viewer-toolbar-menu");
        let view_properties = menu::properties_button(&selection, 20);

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
            toolbar,
            info,
            empty,
            toast,
            pending: RefCell::default(),
            touched: Cell::new(false),
            last_selected: RefCell::default(),
            modified: RefCell::default(),
            resolutions: RefCell::default(),
            grid_properties,
            view_properties,
            wallpaper_argv: crate::desktop::wallpaper_command(),
            undo_stack: RefCell::default(),
            undo_requests,
        });

        this.connect_toolbar();
        this.connect_info();
        this.connect_preview();
        this.connect_keys();
        this.connect_trash(undo_receiver);
        this.build_menu();
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
        // After `keep_first_while_loading`'s, so it sees that selection.
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
                this.toolbar.history_panel.unparent();
                this.folder.dispose();
            }
        });

        this.render_history();
        this.sync_sort_buttons();
        if let Some(file) = &file {
            this.show_file(file.clone());
        }
        if !this.open_path(directory, file) {
            this.open_path(&glib::home_dir(), None);
        }
        this.window.present();
        this.grid.grab_focus();
        match std::env::var("VITRINE_PROBE").as_deref() {
            Ok("ui") => crate::probe::ui::run(&this.window),
            Ok(_) => crate::probe::bench::run(&this.window),
            Err(_) => {}
        }
        this
    }

    fn selected_path(&self) -> Option<PathBuf> {
        selected_path(&self.selection)
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

    // A folder typed in the entry.
    fn open_directory(self: &Rc<Self>, input: &str, select: Option<PathBuf>) -> bool {
        self.open_path(&expand_home(input), select)
    }

    // Paths stay paths (not text) until here, so names that aren't UTF-8
    // open too.
    fn open_path(self: &Rc<Self>, path: &Path, select: Option<PathBuf>) -> bool {
        let path = absolute(path);
        if !path.is_dir() {
            self.toolbar.entry.add_css_class("error");
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
}

fn selected_path(selection: &gtk::SingleSelection) -> Option<PathBuf> {
    selection
        .selected_item()
        .map(|object| image(&object).path.clone())
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

// Surrounding space trimmed, ~ expanded.
fn expand_home(input: &str) -> PathBuf {
    let input = input.trim();
    match input.strip_prefix('~') {
        Some("") => glib::home_dir(),
        Some(rest) if rest.starts_with('/') => glib::home_dir().join(rest.trim_start_matches('/')),
        _ => PathBuf::from(input),
    }
}

// Relative to the current folder, like g_canonicalize_filename: . and ..
// resolved, symlinks kept.
fn absolute(path: &Path) -> PathBuf {
    let current = std::env::current_dir().unwrap_or_else(|_| glib::home_dir());
    let mut resolved = PathBuf::new();
    for component in current.join(path).components() {
        match component {
            std::path::Component::ParentDir => {
                resolved.pop();
            }
            std::path::Component::CurDir => {}
            component => resolved.push(component),
        }
    }
    resolved
}
