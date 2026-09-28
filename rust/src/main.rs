mod library;
mod probe;

use gtk::{gdk, gio, glib, prelude::*};
use library::Image;
use std::{cmp::Ordering, path::PathBuf};

const APP_ID: &str = "io.github.Rido_o.Vitrine.Spike";
// As in the TypeScript app (Window.tsx, style.scss).
const TILE_WIDTH: i32 = 272;
const TILE_HEIGHT: i32 = 153;
const CSS: &str = "
window { background-color: #1e1e1e; color: #ddd; }
gridview { background-color: transparent; }
gridview > child { padding: 6px; }
";

fn main() -> glib::ExitCode {
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

// The store holds `Image`s boxed as GObjects, in the order found; the grid
// shows them sorted by path, sorted incrementally so a big batch doesn't
// block a frame.
fn image(object: &glib::Object) -> std::cell::Ref<'_, Image> {
    object
        .downcast_ref::<glib::BoxedAnyObject>()
        .expect("items are BoxedAnyObjects")
        .borrow::<Image>()
}

fn build_grid(store: &gio::ListStore) -> gtk::GridView {
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
        let label = gtk::Label::builder()
            .width_request(TILE_WIDTH)
            .height_request(TILE_HEIGHT)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .build();
        item.downcast_ref::<gtk::ListItem>()
            .expect("a ListItem")
            .set_child(Some(&label));
    });
    factory.connect_bind(|_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a ListItem");
        let label = item.child().and_downcast::<gtk::Label>().expect("a Label");
        let object = item.item().expect("a bound item");
        let name = image(&object)
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        label.set_label(&name);
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
    let grid = build_grid(&store);
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&grid)
        .vexpand(true)
        .build();
    let stack = gtk::Stack::new();
    stack.add_named(&scrolled, Some("grid"));
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Vitrine (spike)")
        .default_width(1600)
        .default_height(1000)
        .child(&stack)
        .build();

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
    });

    window.present();
    if std::env::var_os("VITRINE_PROBE").is_some() {
        probe::run(&window);
    }
}
