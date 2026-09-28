mod decode;
mod library;
mod preview;
mod probe;
mod thumbnails;
mod tiles;

use gtk::{gdk, gio, glib, prelude::*};
use library::Image;
use preview::Preview;
use std::{cell::RefCell, cmp::Ordering, path::PathBuf, rc::Rc};
use tiles::Tiles;

const APP_ID: &str = "io.github.Rido_o.Vitrine.Spike";
// As in the TypeScript app (Window.tsx, style.scss).
const TILE_WIDTH: i32 = 272;
const TILE_HEIGHT: i32 = 153;
const CSS: &str = "
window { background-color: #1e1e1e; color: #ddd; }
gridview { background-color: transparent; }
gridview > child { padding: 6px; }
.preview { background-color: black; }
";

fn main() -> glib::ExitCode {
    tune_malloc();
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::NON_UNIQUE | gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    app.connect_startup(|_| {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().expect("a display"),
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
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
    let preview_page = gtk::Box::builder().css_classes(["preview"]).build();
    preview_page.append(&preview.picture);
    stack.add_named(&preview_page, Some("preview"));
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Vitrine (spike)")
        .default_width(1600)
        .default_height(1000)
        .child(&stack)
        .build();

    connect_preview(&window, &stack, &grid, &tiles, &preview);

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
            // In device pixels (integer scale; fractional scaling is later).
            let scale = window.scale_factor().max(1) as u32;
            let width = window.width().max(1) as u32 * scale;
            let height = window.height().max(1) as u32 * scale;
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

    let keys = gtk::EventControllerKey::builder()
        .propagation_phase(gtk::PropagationPhase::Capture)
        .build();
    let (window_, stack_, grid_, preview_) =
        (window.clone(), stack.clone(), grid.clone(), preview.clone());
    keys.connect_key_pressed(move |_, key, _, _| {
        if stack_.visible_child_name().as_deref() != Some("preview") {
            return glib::Propagation::Proceed;
        }
        let selected = selection.selected() as i64;
        match key {
            gdk::Key::Right => show_at(selected + 1),
            gdk::Key::Left => show_at(selected - 1),
            gdk::Key::Escape => {
                stack_.set_visible_child_name("grid");
                grid_.scroll_to(selected as u32, gtk::ListScrollFlags::FOCUS, None);
                clear_after_paint(&window_, &stack_, &preview_);
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    window.add_controller(keys);
}

// Dropping the view's textures once the grid's first frame is drawn, so
// freeing them doesn't delay it.
fn clear_after_paint(window: &gtk::ApplicationWindow, stack: &gtk::Stack, preview: &Rc<Preview>) {
    let Some(clock) = window.frame_clock() else {
        preview.clear();
        return;
    };
    let handler = Rc::new(RefCell::new(None));
    let (stack, preview, handler_) = (stack.clone(), preview.clone(), handler.clone());
    let id = clock.connect_after_paint(move |clock| {
        if let Some(id) = handler_.borrow_mut().take() {
            clock.disconnect(id);
        }
        if stack.visible_child_name().as_deref() == Some("grid") {
            preview.clear();
        }
    });
    *handler.borrow_mut() = Some(id);
}
