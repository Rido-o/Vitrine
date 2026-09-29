//! Finding the images in a folder, on a worker thread, and the folder's
//! sorted list of them.

use gtk::{gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    cmp::Ordering,
    collections::HashMap,
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant, UNIX_EPOCH},
};

const EXTENSIONS: [&str; 7] = ["gif", "jpeg", "jpg", "png", "tif", "tiff", "webp"];
// A batch goes to the main thread when it's this big or this old, so the
// first images show quickly and a big folder isn't one item-changed per file.
const BATCH_SIZE: usize = 512;
const BATCH_AGE: Duration = Duration::from_millis(50);

#[derive(Debug, Clone)]
pub struct Image {
    pub path: PathBuf,
    // Seconds since the epoch; with the path, the thumbnail cache's key.
    pub mtime: i64,
    pub size: u64,
}

fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// Walks `root` (and its subfolders when `recursive`, not following symlinked
/// folders) and sends the images found in batches; the channel closes when
/// the walk is done, and the walk stops early if the receiver is dropped.
pub fn scan(root: PathBuf, recursive: bool) -> async_channel::Receiver<Vec<Image>> {
    let (sender, receiver) = async_channel::unbounded();
    let spawn = std::thread::Builder::new()
        .name("scan".into())
        .spawn(move || {
            let mut pending = vec![root];
            let mut batch = Vec::new();
            let mut batch_start = Instant::now();
            while let Some(dir) = pending.pop() {
                let entries = match fs::read_dir(&dir) {
                    Ok(entries) => entries,
                    Err(error) => {
                        eprintln!("Could not read {}: {error}", dir.display());
                        continue;
                    }
                };
                for entry in entries.flatten() {
                    let Ok(file_type) = entry.file_type() else {
                        continue;
                    };
                    let path = entry.path();
                    if file_type.is_dir() {
                        if recursive {
                            pending.push(path);
                        }
                        continue;
                    }
                    if !is_image(&path) {
                        continue;
                    }
                    // Symlinked files count, as what they point to.
                    let metadata = if file_type.is_symlink() {
                        fs::metadata(&path)
                    } else {
                        entry.metadata()
                    };
                    let Ok(metadata) = metadata else { continue };
                    if !metadata.is_file() {
                        continue;
                    }
                    let mtime = metadata
                        .modified()
                        .ok()
                        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                        .map_or(0, |duration| duration.as_secs() as i64);
                    if batch.is_empty() {
                        batch_start = Instant::now();
                    }
                    batch.push(Image {
                        path,
                        mtime,
                        size: metadata.len(),
                    });
                    if (batch.len() >= BATCH_SIZE || batch_start.elapsed() >= BATCH_AGE)
                        && sender.send_blocking(std::mem::take(&mut batch)).is_err()
                    {
                        return;
                    }
                }
            }
            if !batch.is_empty() {
                let _ = sender.send_blocking(batch);
            }
        });
    spawn.expect("the scan thread starts");
    receiver
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Name,
    Date,
    Size,
    Random,
}

// The grid's order. As Library.ts: ties (and Name) by path, as bytes like the
// TypeScript app's string comparison; Random by a hash of the path, reseeded
// on each shuffle.
struct Order {
    key: Cell<SortKey>,
    descending: Cell<bool>,
    seed: Cell<u64>,
}

impl Order {
    fn compare(&self, a: &Image, b: &Image) -> Ordering {
        let shuffled = |path: &Path| {
            let mut hasher = DefaultHasher::new();
            self.seed.get().hash(&mut hasher);
            path.hash(&mut hasher);
            hasher.finish()
        };
        let key = self.key.get();
        let order = match key {
            SortKey::Name => Ordering::Equal,
            SortKey::Date => a.mtime.cmp(&b.mtime),
            SortKey::Size => a.size.cmp(&b.size),
            SortKey::Random => shuffled(&a.path).cmp(&shuffled(&b.path)),
        }
        .then_with(|| {
            a.path
                .as_os_str()
                .as_bytes()
                .cmp(b.path.as_os_str().as_bytes())
        });
        if self.descending.get() && key != SortKey::Random {
            order.reverse()
        } else {
            order
        }
    }
}

fn new_seed() -> u64 {
    (glib::random_int() as u64) << 32 | glib::random_int() as u64
}

/// The store holds `Image`s boxed as GObjects.
pub fn image(object: &glib::Object) -> std::cell::Ref<'_, Image> {
    object
        .downcast_ref::<glib::BoxedAnyObject>()
        .expect("items are BoxedAnyObjects")
        .borrow::<Image>()
}

#[derive(Clone, Copy)]
pub enum Finished {
    Load,
    Rescan,
}

/// A folder's images: `store` in the order found, `sorted` for the grid,
/// sorted incrementally while loading so a big batch doesn't block a frame.
pub struct Folder {
    pub store: gio::ListStore,
    pub sorted: gtk::SortListModel,
    sorter: gtk::CustomSorter,
    order: Rc<Order>,
    directory: RefCell<PathBuf>,
    recursive: Cell<bool>,
    loading: Cell<bool>,
    scanning: Cell<bool>,
    // Bumped by each load or rescan, so a superseded one stops.
    generation: Cell<u64>,
    finished: RefCell<Option<Rc<dyn Fn(Finished)>>>,
}

impl Folder {
    pub fn new(recursive: bool) -> Rc<Self> {
        let store = gio::ListStore::new::<glib::BoxedAnyObject>();
        let order = Rc::new(Order {
            key: Cell::new(SortKey::Name),
            descending: Cell::new(false),
            seed: Cell::new(new_seed()),
        });
        let order_ = order.clone();
        let sorter =
            gtk::CustomSorter::new(move |a, b| order_.compare(&image(a), &image(b)).into());
        let sorted = gtk::SortListModel::new(Some(store.clone()), Some(sorter.clone()));
        sorted.set_incremental(true);
        Rc::new(Self {
            store,
            sorted,
            sorter,
            order,
            directory: RefCell::default(),
            recursive: Cell::new(recursive),
            loading: Cell::new(false),
            scanning: Cell::new(false),
            generation: Cell::new(0),
            finished: RefCell::default(),
        })
    }

    /// Called once a load or rescan is done and sorted.
    pub fn connect_finished(&self, finished: impl Fn(Finished) + 'static) {
        *self.finished.borrow_mut() = Some(Rc::new(finished));
    }

    pub fn directory(&self) -> PathBuf {
        self.directory.borrow().clone()
    }

    pub fn recursive(&self) -> bool {
        self.recursive.get()
    }

    pub fn set_recursive(&self, recursive: bool) {
        self.recursive.set(recursive);
    }

    /// Loading a folder: from `load` until its images are all in and sorted.
    pub fn is_loading(&self) -> bool {
        self.loading.get()
    }

    /// A load or a rescan is walking the folder.
    pub fn is_scanning(&self) -> bool {
        self.scanning.get()
    }

    pub fn sort_key(&self) -> SortKey {
        self.order.key.get()
    }

    pub fn descending(&self) -> bool {
        self.order.descending.get()
    }

    /// The images in the grid's order.
    pub fn images(&self) -> Vec<Image> {
        (0..self.sorted.n_items())
            .filter_map(|i| self.sorted.item(i))
            .map(|object| image(&object).clone())
            .collect()
    }

    pub fn position(&self, path: &Path) -> Option<u32> {
        (0..self.sorted.n_items()).find(|&i| {
            self.sorted
                .item(i)
                .is_some_and(|object| image(&object).path == path)
        })
    }

    fn begin(&self) -> u64 {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.scanning.set(true);
        generation
    }

    fn current(&self, generation: u64) -> bool {
        self.generation.get() == generation
    }

    fn finish(self: &Rc<Self>, generation: u64, finished: Finished) {
        self.scanning.set(false);
        let folder = self.clone();
        self.when_sorted(move || {
            if !folder.current(generation) {
                return;
            }
            folder.loading.set(false);
            let callback = folder.finished.borrow().clone();
            if let Some(callback) = callback {
                callback(finished);
            }
        });
    }

    fn when_sorted(&self, done: impl Fn() + 'static) {
        if self.sorted.pending() == 0 {
            return done();
        }
        let handler = Rc::new(RefCell::new(None));
        let handler_ = handler.clone();
        let id = self.sorted.connect_pending_notify(move |sorted| {
            if sorted.pending() > 0 {
                return;
            }
            if let Some(id) = handler_.borrow_mut().take() {
                sorted.disconnect(id);
            }
            done();
        });
        *handler.borrow_mut() = Some(id);
    }

    /// Replaces the list with `directory`'s images, as they're found.
    pub fn load(self: &Rc<Self>, directory: PathBuf) {
        let generation = self.begin();
        self.loading.set(true);
        *self.directory.borrow_mut() = directory.clone();
        self.store.remove_all();
        let receiver = scan(directory, self.recursive.get());
        let folder = self.clone();
        glib::spawn_future_local(async move {
            let mut times = crate::probe::ScanTimes::start();
            while let Ok(batch) = receiver.recv().await {
                // Dropping the receiver stops the walk.
                if !folder.current(generation) {
                    return;
                }
                let started = glib::monotonic_time();
                let objects: Vec<glib::BoxedAnyObject> =
                    batch.into_iter().map(glib::BoxedAnyObject::new).collect();
                folder.store.splice(folder.store.n_items(), 0, &objects);
                times.batch(started);
            }
            if !folder.current(generation) {
                return;
            }
            times.finish(folder.store.n_items());
            folder.finish(generation, Finished::Load);
        });
    }

    /// Picks up added, removed and changed files, leaving the rest of the
    /// list (and the grid's tiles) alone. Not while a load or rescan runs.
    pub fn rescan(self: &Rc<Self>) {
        if self.scanning.get() || self.loading.get() {
            return;
        }
        let generation = self.begin();
        let receiver = scan(self.directory(), self.recursive.get());
        let folder = self.clone();
        glib::spawn_future_local(async move {
            let mut fresh: HashMap<PathBuf, Image> = HashMap::new();
            while let Ok(batch) = receiver.recv().await {
                if !folder.current(generation) {
                    return;
                }
                fresh.extend(batch.into_iter().map(|image| (image.path.clone(), image)));
            }
            if !folder.current(generation) {
                return;
            }
            // In one go, so the grid never shows a half-sorted list.
            folder.sorted.set_incremental(false);
            let store = &folder.store;
            for i in (0..store.n_items()).rev() {
                let Some(object) = store.item(i) else {
                    continue;
                };
                let unchanged = {
                    let old = image(&object);
                    fresh
                        .get(&old.path)
                        .is_some_and(|new| new.mtime == old.mtime && new.size == old.size)
                };
                if unchanged {
                    fresh.remove(&image(&object).path.clone());
                } else {
                    store.remove(i);
                }
            }
            let objects: Vec<glib::BoxedAnyObject> =
                fresh.into_values().map(glib::BoxedAnyObject::new).collect();
            store.splice(store.n_items(), 0, &objects);
            folder.sorted.set_incremental(true);
            folder.finish(generation, Finished::Rescan);
        });
    }

    /// As Library.ts: Random reshuffles each time; switching to Date or Size
    /// starts with the newest or largest.
    pub fn set_sort(&self, key: SortKey) {
        if key == SortKey::Random {
            self.order.seed.set(new_seed());
        } else if key != self.order.key.get() {
            self.order
                .descending
                .set(matches!(key, SortKey::Date | SortKey::Size));
        }
        self.order.key.set(key);
        self.resort();
    }

    pub fn toggle_direction(&self) {
        self.order.descending.set(!self.order.descending.get());
        self.resort();
    }

    // At once rather than incrementally, so the selection can be put back
    // straight after.
    fn resort(&self) {
        self.sorted.set_incremental(false);
        self.sorter.changed(gtk::SorterChange::Different);
        self.sorted.set_incremental(true);
    }
}
