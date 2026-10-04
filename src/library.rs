//! Finding the images in a folder, on a worker thread, and the folder's
//! sorted list of them.

use gtk::{gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    cmp::Ordering,
    collections::{HashMap, HashSet},
    fs,
    hash::{BuildHasher, BuildHasherDefault, DefaultHasher},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant, UNIX_EPOCH},
};

pub const EXTENSIONS: [&str; 7] = ["gif", "jpeg", "jpg", "png", "tif", "tiff", "webp"];
// A batch goes to the main thread when it's this big or this old, so the
// first images show quickly and a big folder isn't one item-changed per file.
const BATCH_SIZE: usize = 512;
const BATCH_AGE: Duration = Duration::from_millis(50);
// Folders watched for changes (inotify watches are limited), and how often
// changes trigger a rescan at most.
const WATCH_LIMIT: usize = 1000;
const RESCAN_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct Image {
    pub path: PathBuf,
    // Seconds since the epoch; with the path, the thumbnail cache's key.
    pub mtime: i64,
    pub size: u64,
}

// Seconds since the epoch.
fn mtime(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_secs() as i64)
}

pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

pub enum Scanned {
    Images(Vec<Image>),
    // Last: the folders read in full (to watch).
    Folders(Vec<PathBuf>),
}

/// Walks `root` (and its subfolders when `recursive`, not following symlinked
/// folders) and sends the images found in batches, then the folders read; the
/// channel closes when the walk is done, and the walk stops early if the
/// receiver is dropped.
pub fn scan(root: PathBuf, recursive: bool) -> async_channel::Receiver<Scanned> {
    let (sender, receiver) = async_channel::unbounded();
    let spawn = std::thread::Builder::new()
        .name("scan".into())
        .spawn(move || {
            let mut pending = vec![root];
            let mut folders = Vec::new();
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
                folders.push(dir.clone());
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
                    let mtime = mtime(&metadata);
                    if batch.is_empty() {
                        batch_start = Instant::now();
                    }
                    batch.push(Image {
                        path,
                        mtime,
                        size: metadata.len(),
                    });
                    if (batch.len() >= BATCH_SIZE || batch_start.elapsed() >= BATCH_AGE)
                        && sender
                            .send_blocking(Scanned::Images(std::mem::take(&mut batch)))
                            .is_err()
                    {
                        return;
                    }
                }
            }
            if !batch.is_empty() && sender.send_blocking(Scanned::Images(batch)).is_err() {
                return;
            }
            let _ = sender.send_blocking(Scanned::Folders(folders));
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

// The grid's order: ties (and Name) by path, as bytes; Random by a hash of the
// path, reseeded on each shuffle.
struct Order {
    key: Cell<SortKey>,
    descending: Cell<bool>,
    seed: Cell<u64>,
}

impl Order {
    fn compare(&self, a: &Image, b: &Image) -> Ordering {
        let shuffled = |path: &Path| {
            BuildHasherDefault::<DefaultHasher>::default().hash_one((self.seed.get(), path))
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

type FinishedCallback = Rc<dyn Fn(Finished)>;
type ModifiedCallback = Rc<dyn Fn(&Path)>;

/// A folder's images: `store` in the order found, `sorted` for the grid,
/// sorted incrementally while loading so a big batch doesn't block a frame.
pub struct Folder {
    store: gio::ListStore,
    pub sorted: gtk::SortListModel,
    sorter: gtk::CustomSorter,
    order: Rc<Order>,
    directory: RefCell<PathBuf>,
    recursive: Cell<bool>,
    loading: Cell<bool>,
    scanning: Cell<bool>,
    // Bumped by each load or rescan, so a superseded one stops.
    generation: Cell<u64>,
    finished: RefCell<Option<FinishedCallback>>,
    // Told each image a rescan found changed on disk.
    modified: RefCell<Option<ModifiedCallback>>,
    // A rescan is changing the list (removing, adding, replacing).
    updating: Cell<bool>,
    monitors: RefCell<HashMap<PathBuf, gio::FileMonitor>>,
    rescan_timeout: RefCell<Option<glib::SourceId>>,
    // Images removed (trashed) or added (restored) here while a scan runs;
    // its results predate them, so it leaves them alone.
    removed_during_scan: RefCell<HashSet<PathBuf>>,
    added_during_scan: RefCell<HashSet<PathBuf>>,
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
            modified: RefCell::default(),
            updating: Cell::new(false),
            monitors: RefCell::default(),
            rescan_timeout: RefCell::default(),
            removed_during_scan: RefCell::default(),
            added_during_scan: RefCell::default(),
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

    fn changed_during_scan(&self, path: &Path) -> bool {
        self.removed_during_scan.borrow().contains(path)
            || self.added_during_scan.borrow().contains(path)
    }

    /// Takes `path` out of the list (trashed); its position before, if it
    /// was there.
    pub fn remove(&self, path: &Path) -> Option<u32> {
        let position = self.position(path);
        if let Some(index) = self
            .store
            .find_with_equal_func(|object| image(object).path == path)
        {
            self.store.remove(index);
        }
        if self.scanning.get() {
            self.removed_during_scan
                .borrow_mut()
                .insert(path.to_owned());
            self.added_during_scan.borrow_mut().remove(path);
        }
        position
    }

    /// Puts a file that appeared outside a scan (restored from the trash)
    /// into the list; its position, or None if it doesn't belong here (or
    /// can't be read).
    pub fn add(&self, path: &Path) -> Option<u32> {
        if let Some(position) = self.position(path) {
            return Some(position);
        }
        let directory = self.directory();
        let belongs = is_image(path)
            && (path.parent() == Some(directory.as_path())
                || (self.recursive.get() && path.starts_with(&directory)));
        if !belongs {
            return None;
        }
        let metadata = fs::metadata(path)
            .inspect_err(|error| eprintln!("Could not read {}: {error}", path.display()))
            .ok()?;
        let image = Image {
            path: path.to_owned(),
            mtime: mtime(&metadata),
            size: metadata.len(),
        };
        // Sorted at once, so its position is known.
        self.sorted.set_incremental(false);
        self.store.append(&glib::BoxedAnyObject::new(image));
        self.sorted.set_incremental(true);
        if self.scanning.get() {
            self.added_during_scan.borrow_mut().insert(path.to_owned());
            self.removed_during_scan.borrow_mut().remove(path);
        }
        self.position(path)
    }

    fn begin(&self) -> u64 {
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.scanning.set(true);
        self.removed_during_scan.borrow_mut().clear();
        self.added_during_scan.borrow_mut().clear();
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
        self.unwatch();
        self.loading.set(true);
        *self.directory.borrow_mut() = directory.clone();
        self.store.remove_all();
        let receiver = scan(directory, self.recursive.get());
        let folder = self.clone();
        glib::spawn_future_local(async move {
            let mut times = crate::probe::ScanTimes::start();
            let mut folders = Vec::new();
            while let Ok(scanned) = receiver.recv().await {
                // Dropping the receiver stops the walk.
                if !folder.current(generation) {
                    return;
                }
                let batch = match scanned {
                    Scanned::Images(batch) => batch,
                    Scanned::Folders(read) => {
                        folders = read;
                        continue;
                    }
                };
                let started = glib::monotonic_time();
                let objects: Vec<glib::BoxedAnyObject> = batch
                    .into_iter()
                    .filter(|image| !folder.changed_during_scan(&image.path))
                    .map(glib::BoxedAnyObject::new)
                    .collect();
                folder.store.splice(folder.store.n_items(), 0, &objects);
                times.batch(started);
            }
            if !folder.current(generation) {
                return;
            }
            times.finish(folder.store.n_items());
            folder.watch(folders);
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
            let mut folders = Vec::new();
            while let Ok(scanned) = receiver.recv().await {
                if !folder.current(generation) {
                    return;
                }
                match scanned {
                    Scanned::Images(batch) => {
                        fresh.extend(batch.into_iter().map(|image| (image.path.clone(), image)))
                    }
                    Scanned::Folders(read) => folders = read,
                }
            }
            if !folder.current(generation) {
                return;
            }
            // In one go, so the grid never shows a half-sorted list.
            folder.updating.set(true);
            folder.sorted.set_incremental(false);
            let store = &folder.store;
            let mut modified = Vec::new();
            for i in (0..store.n_items()).rev() {
                let Some(object) = store.item(i) else {
                    continue;
                };
                let old = image(&object).clone();
                let unchanged = match fresh.get(&old.path) {
                    Some(new) if new.mtime == old.mtime && new.size == old.size => true,
                    Some(_) => {
                        modified.push(old.path.clone());
                        false
                    }
                    None => false,
                };
                let path = old.path;
                if unchanged || folder.added_during_scan.borrow().contains(&path) {
                    fresh.remove(&path);
                } else {
                    store.remove(i);
                }
            }
            let removed = folder.removed_during_scan.borrow();
            let objects: Vec<glib::BoxedAnyObject> = fresh
                .into_values()
                .filter(|image| !removed.contains(&image.path))
                .map(glib::BoxedAnyObject::new)
                .collect();
            drop(removed);
            store.splice(store.n_items(), 0, &objects);
            folder.sorted.set_incremental(true);
            folder.updating.set(false);
            let callback = folder.modified.borrow().clone();
            if let Some(callback) = callback {
                for path in &modified {
                    callback(path);
                }
            }
            folder.watch(folders);
            folder.finish(generation, Finished::Rescan);
        });
    }

    /// Called with each image a rescan found changed on disk, before the
    /// rescan's `finished`.
    pub fn connect_modified(&self, modified: impl Fn(&Path) + 'static) {
        *self.modified.borrow_mut() = Some(Rc::new(modified));
    }

    /// A rescan is changing the list (so a selection change is its doing,
    /// not the user's).
    pub fn is_updating(&self) -> bool {
        self.updating.get()
    }

    /// Stops scanning and watching, for a closed window.
    pub fn dispose(&self) {
        self.generation.set(self.generation.get() + 1);
        self.unwatch();
    }

    // Watches the folders read (the first WATCH_LIMIT) and rescans after a
    // change. Monitors only see changes made on this machine (not, say, on
    // an NFS server).
    fn watch(self: &Rc<Self>, folders: Vec<PathBuf>) {
        let wanted: HashSet<PathBuf> = folders.into_iter().take(WATCH_LIMIT).collect();
        let mut monitors = self.monitors.borrow_mut();
        monitors.retain(|path, monitor| {
            let keep = wanted.contains(path);
            if !keep {
                monitor.cancel();
            }
            keep
        });
        for path in wanted {
            if monitors.contains_key(&path) {
                continue;
            }
            let monitor = match gio::File::for_path(&path).monitor_directory(
                gio::FileMonitorFlags::WATCH_MOVES,
                None::<&gio::Cancellable>,
            ) {
                Ok(monitor) => monitor,
                Err(error) => {
                    eprintln!("Could not watch {}: {error}", path.display());
                    continue;
                }
            };
            let weak = Rc::downgrade(self);
            monitor.connect_changed(move |_, file, other, event| {
                if let Some(folder) = weak.upgrade() {
                    folder.changed(file, other, event);
                }
            });
            monitors.insert(path, monitor);
        }
    }

    fn unwatch(&self) {
        for (_, monitor) in self.monitors.borrow_mut().drain() {
            monitor.cancel();
        }
        if let Some(id) = self.rescan_timeout.borrow_mut().take() {
            id.remove();
        }
    }

    // Other files only matter when recursive: they may be folders.
    fn changed(
        self: &Rc<Self>,
        file: &gio::File,
        other: Option<&gio::File>,
        event: gio::FileMonitorEvent,
    ) {
        use gio::FileMonitorEvent::*;
        if !matches!(
            event,
            ChangesDoneHint | Created | Deleted | MovedIn | MovedOut | Renamed
        ) {
            return;
        }
        let image = |file: Option<&gio::File>| {
            file.and_then(|file| file.path())
                .is_some_and(|path| is_image(&path))
        };
        if self.recursive.get() || image(Some(file)) || image(other) {
            self.schedule_rescan();
        }
    }

    // At most one rescan per RESCAN_DELAY, so copying many files doesn't
    // rescan for each; waits for a running scan to finish first.
    fn schedule_rescan(self: &Rc<Self>) {
        if self.rescan_timeout.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(RESCAN_DELAY, move || {
            let Some(folder) = weak.upgrade() else { return };
            folder.rescan_timeout.borrow_mut().take();
            if folder.scanning.get() || folder.loading.get() {
                folder.schedule_rescan();
            } else {
                folder.rescan();
            }
        });
        *self.rescan_timeout.borrow_mut() = Some(id);
    }

    /// Random reshuffles each time; switching to Date or Size
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
