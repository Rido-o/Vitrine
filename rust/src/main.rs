mod autohide;
mod decode;
mod library;
mod preview;
mod probe;
mod thumbnails;
mod tiles;
mod view;
mod zoomable;

use gtk::{gdk, gio, glib, prelude::*};
use library::Image;
use preview::Preview;
use std::{
    cell::{Cell, RefCell},
    cmp::Ordering,
    path::{Path, PathBuf},
    rc::Rc,
};
use tiles::Tiles;
use view::View;

const APP_ID: &str = "io.github.Rido_o.Vitrine.Spike";
// As in the TypeScript app (Window.tsx, style.scss).
const TILE_WIDTH: i32 = 272;
const TILE_HEIGHT: i32 = 153;
// The selection is an outline, as in the TypeScript app, that fades in: the
// default theme's animated highlight looked choppy next to scrolling, and none
// at all felt abrupt. Only its colour changes, briefly, so a lower frame rate
// barely shows.
const CSS: &str = "
window { background-color: #1e1e1e; color: #ddd; }
gridview { background-color: transparent; }
gridview > child {
  padding: 6px;
  background: none;
  outline: 3px solid transparent;
  outline-offset: -7px;
  border-radius: 12px;
  transition: outline-color 120ms ease-out;
}
gridview > child:selected {
  outline-color: #8eaaaa;
}
.viewer-preview { background-color: black; }
/* The view's controls, as in style.scss (theme.scss's colours). */
.viewer-preview label,
.viewer-preview button {
  min-height: 40px;
  border: 1px solid rgba(193, 193, 193, 0.12);
  border-radius: 10px;
  background-color: rgba(16, 16, 16, 0.85);
  color: #b7b7b7;
}
.viewer-preview button {
  padding: 0 12px;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.3);
  transition: all 200ms cubic-bezier(0.25, 0.46, 0.45, 0.94);
}
.viewer-preview button:hover {
  background-color: rgba(35, 35, 35, 0.95);
  border-color: rgba(193, 193, 193, 0.22);
  color: #d6d6d6;
}
.viewer-preview button:active { background-color: rgba(51, 51, 51, 0.95); }
.viewer-preview .preview-image-info label { padding: 0 12px; }
.viewer-preview .preview-controls {
  transition: opacity 300ms cubic-bezier(0.25, 0.46, 0.45, 0.94);
}
.viewer-preview .preview-controls.hidden { opacity: 0; }
";

// The symbolic icons (../icons), installed by package.nix.
const ICONS_DIR: &str = match option_env!("VITRINE_ICONS_DIR") {
    Some(dir) => dir,
    None => concat!(env!("CARGO_MANIFEST_DIR"), "/../icons"),
};

fn main() -> glib::ExitCode {
    tune_malloc();
    prefer_gl_on_nvidia();
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE | gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    app.connect_startup(|_| {
        let display = gdk::Display::default().expect("a display");
        let provider = gtk::CssProvider::new();
        provider.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
        gtk::IconTheme::for_display(&display).add_search_path(ICONS_DIR);
    });
    app.connect_activate(|app| {
        let dir = std::env::current_dir().unwrap_or_else(|_| glib::home_dir());
        build_window(app, dir);
    });
    app.connect_open(|app, files, _| {
        if let Some(dir) = files.first().and_then(|file| file.path()) {
            build_window(app, dir);
        }
    });
    app.run()
}

// glibc raises its mmap threshold each time a large block is freed, so the
// workers' multi-MB decode buffers ended up in per-thread arenas that never
// shrink (~230 MB after generating a 3,000-image folder). A fixed 1 MB
// threshold returns them to the system on free (thumbnails, ~0.5 MB, stay in
// the arenas), and 4 arenas bound the rest: peak RSS 844 → 718 MB, no
// measurable slowdown.
fn tune_malloc() {
    // SAFETY: mallopt only sets allocator parameters; called before any
    // other thread exists.
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 1 << 20);
        libc::mallopt(libc::M_ARENA_MAX, 4);
    }
}

// With NVIDIA's driver, GTK's default Vulkan renderer spends several ms of
// main-thread time on each new texture (a row of thumbnails, or each tile
// panned into view: 76 ms frames panning at 100%); its GL renderer doesn't.
// Only when the driver is loaded, and never over an explicit GSK_RENDERER.
// As main.tsx.
fn prefer_gl_on_nvidia() {
    if std::env::var_os("GSK_RENDERER").is_none() && Path::new("/proc/driver/nvidia").is_dir() {
        // SAFETY: before any other thread exists.
        unsafe { std::env::set_var("GSK_RENDERER", "gl") };
    }
}

// The store holds `Image`s boxed as GObjects, in the order found; the grid
// shows them sorted by path, sorted incrementally so a big batch doesn't
// block a frame.
fn image(object: &glib::Object) -> std::cell::Ref<'_, Image> {
    object
        .downcast_ref::<glib::BoxedAnyObject>()
        .expect("items are BoxedAnyObjects")
        .borrow::<Image>()
}

fn build_grid(store: &gio::ListStore, tiles: &Rc<Tiles>) -> gtk::GridView {
    let sorter = gtk::CustomSorter::new(|a, b| match image(a).path.cmp(&image(b).path) {
        Ordering::Less => gtk::Ordering::Smaller,
        Ordering::Equal => gtk::Ordering::Equal,
        Ordering::Greater => gtk::Ordering::Larger,
    });
    let sorted = gtk::SortListModel::new(Some(store.clone()), Some(sorter));
    sorted.set_incremental(true);
    let selection = gtk::SingleSelection::new(Some(sorted));

    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let picture = gtk::Picture::builder()
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
        .model(&selection)
        .factory(&factory)
        .min_columns(1)
        .max_columns(12)
        .build()
}

fn build_window(app: &gtk::Application, dir: PathBuf) {
    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    let tiles = Tiles::new(glib::user_cache_dir().join("vitrine-spike/thumbnails"));
    let grid = build_grid(&store, &tiles);
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&grid)
        .vexpand(true)
        .build();
    let stack = gtk::Stack::new();
    stack.add_named(&scrolled, Some("grid"));
    let preview = Preview::new();
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Vitrine (spike)")
        .default_width(1600)
        .default_height(1000)
        .child(&stack)
        .build();

    let view = View::new(&window, &stack, &preview);
    stack.add_named(&view.page, Some("preview"));
    connect_preview(&window, &stack, &grid, &tiles, &preview, &view);

    let scanning = Rc::new(Cell::new(true));
    keep_first_while_loading(&grid, &scanning);

    let receiver = library::scan(dir, true);
    let scan = probe::ScanTimes::start();
    glib::spawn_future_local(async move {
        let mut scan = scan;
        while let Ok(batch) = receiver.recv().await {
            let started = glib::monotonic_time();
            let objects: Vec<glib::BoxedAnyObject> =
                batch.into_iter().map(glib::BoxedAnyObject::new).collect();
            store.splice(store.n_items(), 0, &objects);
            scan.batch(started);
        }
        scan.finish(store.n_items());
        scanning.set(false);
        let mut images: Vec<Image> = (0..store.n_items())
            .filter_map(|i| store.item(i))
            .map(|object| image(&object).clone())
            .collect();
        images.sort_by(|a, b| a.path.cmp(&b.path));
        tiles.set_background(&images);
    });

    window.present();
    if std::env::var_os("VITRINE_PROBE").is_some() {
        probe::run(&window);
    }
}

// Opening (Enter, double-click), ←/→ and Esc in the full-screen view.
fn connect_preview(
    window: &gtk::ApplicationWindow,
    stack: &gtk::Stack,
    grid: &gtk::GridView,
    tiles: &Rc<Tiles>,
    preview: &Rc<Preview>,
    view: &Rc<View>,
) {
    let selection = grid
        .model()
        .and_downcast::<gtk::SingleSelection>()
        .expect("the grid's model is a SingleSelection");
    let show_at: Rc<dyn Fn(i64)> = {
        let (window, stack, selection) = (window.clone(), stack.clone(), selection.clone());
        let (tiles, preview) = (tiles.clone(), preview.clone());
        Rc::new(move |position: i64| {
            let count = selection.n_items() as i64;
            if count == 0 {
                return;
            }
            let at = |offset: i64| {
                selection
                    .item((position + offset).rem_euclid(count) as u32)
                    .map(|object| image(&object).clone())
            };
            let Some(shown) = at(0) else { return };
            selection.set_selected(position.rem_euclid(count) as u32);
            // Nearest first, the way ←/→ would reach them.
            let neighbours = [1, -1, 2, -2]
                .into_iter()
                .filter_map(at)
                .map(|image| image.path)
                .collect();
            let (width, height) = view_size(&window);
            preview.show(
                shown.path.clone(),
                neighbours,
                tiles.cached(&shown),
                width,
                height,
            );
            stack.set_visible_child_name("preview");
        })
    };

    let open = show_at.clone();
    grid.connect_activate(move |_, position| open(position as i64));

    // The image selected in the grid is decoded in the background, so
    // opening it shows it sharp straight away. (In the view, `show` preloads
    // the neighbours instead.)
    let (window_, stack_, preview_) = (window.clone(), stack.clone(), preview.clone());
    selection.connect_selected_item_notify(move |selection| {
        if stack_.visible_child_name().as_deref() != Some("grid") {
            return;
        }
        if let Some(object) = selection.selected_item() {
            let (width, height) = view_size(&window_);
            preview_.preload(image(&object).path.clone(), width, height);
        }
    });

    let close: Rc<dyn Fn()> = {
        let (window, stack, grid, preview, view) = (
            window.clone(),
            stack.clone(),
            grid.clone(),
            preview.clone(),
            view.clone(),
        );
        let selection = selection.clone();
        Rc::new(move || {
            view.leave();
            stack.set_visible_child_name("grid");
            grid.scroll_to(selection.selected(), gtk::ListScrollFlags::FOCUS, None);
            hide_after_paint(&window, &stack, &selection, &preview);
        })
    };
    let close_ = close.clone();
    view.connect_close(move || close_());

    // At fit, a click in the left/right edge moves to the previous/next.
    let (move_, selection_) = (show_at.clone(), selection.clone());
    preview.image.connect_navigate(move |side| {
        move_(selection_.selected() as i64 + side as i64);
    });

    let keys = gtk::EventControllerKey::builder()
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    let (stack_, view_) = (stack.clone(), view.clone());
    keys.connect_key_pressed(move |_, key, _, state| {
        // Plain keys only (Shift is allowed, for + and ?), so e.g. Ctrl+Q
        // stays the window's.
        let held = gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::ALT_MASK
            | gdk::ModifierType::SUPER_MASK;
        if state.intersects(held) {
            return glib::Propagation::Proceed;
        }
        let selected = selection.selected() as i64;
        if stack_.visible_child_name().as_deref() != Some("preview") {
            if matches!(key, gdk::Key::e | gdk::Key::E) && selection.selected_item().is_some() {
                show_at(selected);
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }
        match key {
            gdk::Key::Right => show_at(selected + 1),
            gdk::Key::Left => show_at(selected - 1),
            gdk::Key::Escape | gdk::Key::q => close(),
            _ if view_.key(key) => {}
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    window.add_controller(keys);
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

// While a folder loads, keeps the first image selected and the grid at the
// top, until the user clicks, types or scrolls in it. Batches arrive in any
// order and are sorted as they come; the automatic selection would otherwise
// stay on whichever image arrived first, wherever sorting put it, and the grid
// kept it in view (opening halfway down a big folder). Loading is over once
// the scan has finished and the sorting has caught up.
fn keep_first_while_loading(grid: &gtk::GridView, scanning: &Rc<Cell<bool>>) {
    let selection = grid
        .model()
        .and_downcast::<gtk::SingleSelection>()
        .expect("the grid's model is a SingleSelection");
    let sorted = selection
        .model()
        .and_downcast::<gtk::SortListModel>()
        .expect("the selection's model is a SortListModel");
    // Any press, key or scroll in the grid (seen, not consumed).
    let touched = Rc::new(Cell::new(false));
    let events = gtk::EventControllerLegacy::new();
    events.set_propagation_phase(gtk::PropagationPhase::Capture);
    let touched_ = touched.clone();
    events.connect_event(move |_, event| {
        use gdk::EventType::*;
        if matches!(
            event.event_type(),
            ButtonPress | KeyPress | Scroll | TouchBegin
        ) {
            touched_.set(true);
        }
        glib::Propagation::Proceed
    });
    grid.add_controller(events);
    let back_to_first = {
        let (grid, selection) = (grid.clone(), selection.clone());
        move || {
            if touched.get() || selection.n_items() == 0 {
                return;
            }
            if selection.selected() != 0 {
                selection.set_selected(0);
            }
            grid.scroll_to(0, gtk::ListScrollFlags::NONE, None);
        }
    };
    let back = Rc::new(back_to_first);
    let (back_, scanning_, sorted_) = (back.clone(), scanning.clone(), sorted.clone());
    selection.connect_items_changed(move |_, _, _, _| {
        if scanning_.get() || sorted_.pending() > 0 {
            back_();
        }
    });
    // Sorting can catch up after the scan's last batch: once more then.
    let scanning = scanning.clone();
    sorted.connect_pending_notify(move |sorted| {
        if sorted.pending() == 0 && !scanning.get() {
            back();
        }
    });
}
