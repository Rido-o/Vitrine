//! The full-screen view: the shown image and its neighbours decoded on a few
//! worker threads at the size they're shown at, with the thumbnail as a
//! placeholder until the shown one arrives; full-resolution tiles of the shown
//! image when zooming needs them; GIFs played from frames decoded ahead on a
//! thread of their own.

use super::zoomable::{Tiles, ZoomableImage};
use crate::decode::{self, Pixels, Tile};
use gtk::{gdk, gdk_pixbuf::PixbufAnimation, glib, prelude::*};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Condvar, Mutex, mpsc},
    time::{Duration, SystemTime},
};

// Decodes at once, at most: holding ←/→ at 30 images/s, 4 workers showed 42
// of 60 sharp before the next against 27 with 2; a fifth added nothing.
const MAX_WORKERS: usize = 4;
// Full resolution comes in tiles this size (see ZoomableImage).
const TILE_SIZE: u32 = 512;
// Animation frames decoded ahead of the one shown.
const FRAMES_AHEAD: usize = 4;
// A floor for frame delays, as browsers have (GdkPixbuf already turns 0 and
// 10 ms, "as fast as possible", into 100 ms).
const MIN_FRAME_DELAY: Duration = Duration::from_millis(20);

enum Kind {
    // Fitted within this many device pixels.
    Fit(u32, u32),
    // Full resolution, in tiles.
    Full,
}

struct Request {
    path: PathBuf,
    kind: Kind,
}

enum Decoded {
    Fit {
        pixels: Pixels,
        full: (u32, u32),
    },
    Tiles {
        tiles: Vec<(Tile, Pixels)>,
        width: u32,
        height: u32,
    },
}

#[derive(Default)]
struct Queue {
    // Next first; replaced on every move, so passed images are never decoded.
    requests: Vec<Request>,
}

struct Shared {
    queue: Mutex<Queue>,
    work: Condvar,
    results: async_channel::Sender<(PathBuf, Result<Decoded, String>)>,
}

fn worker(shared: &Shared) {
    loop {
        let request = {
            let mut queue = shared.queue.lock().unwrap();
            loop {
                if !queue.requests.is_empty() {
                    break queue.requests.remove(0);
                }
                queue = shared.work.wait(queue).unwrap();
            }
        };
        let result = match request.kind {
            Kind::Fit(width, height) => {
                decode::to_fit(&request.path, width, height).map(|fitted| Decoded::Fit {
                    pixels: fitted.rgba.premultiplied(),
                    full: fitted.full,
                })
            }
            Kind::Full => decode::to_fit(&request.path, u32::MAX, u32::MAX).map(|fitted| {
                let (tiles, width, height) = decode::split(fitted.rgba, TILE_SIZE, 1);
                Decoded::Tiles {
                    tiles,
                    width,
                    height,
                }
            }),
        };
        if shared
            .results
            .send_blocking((request.path, result))
            .is_err()
        {
            return;
        }
    }
}

pub fn is_animation(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("gif"))
}

struct Frame {
    pixels: Pixels,
    delay: Option<Duration>,
}

// Decodes `path`'s frames in order, fitted within `max`, until the receiver
// is dropped (or a finite animation's last frame). GdkPixbuf's iterator is
// driven by a clock of our own, a frame's delay at a time.
fn decode_frames(path: &Path, max: (u32, u32), frames: &mpsc::SyncSender<Frame>) {
    let Ok(animation) = PixbufAnimation::from_file(path) else {
        return;
    };
    if animation.is_static_image() {
        return;
    }
    let mut time = SystemTime::UNIX_EPOCH;
    let iter = animation.iter(Some(time));
    loop {
        let mut rgba = decode::Rgba::from_pixbuf(&iter.pixbuf());
        let (width, height) = decode::fitted(rgba.width, rgba.height, max.0, max.1);
        if (width, height) != (rgba.width, rgba.height) {
            match decode::resize(rgba, width, height) {
                Ok(resized) => rgba = resized,
                Err(_) => return,
            }
        }
        let delay = iter.delay_time().map(|delay| delay.max(MIN_FRAME_DELAY));
        let frame = Frame {
            pixels: rgba.premultiplied(),
            delay,
        };
        if frames.send(frame).is_err() {
            return;
        }
        let Some(delay) = delay else { return };
        time += delay;
        iter.advance(time);
    }
}

// A decoded texture, and its image's own size.
type FitTexture = (gdk::Texture, (u32, u32));
type InfoCallback = Box<dyn Fn(&Path, Option<(u32, u32)>)>;

pub struct Preview {
    pub image: ZoomableImage,
    shared: Arc<Shared>,
    // Decoded textures of the shown image and its neighbours, with each
    // image's own size.
    textures: RefCell<HashMap<PathBuf, FitTexture>>,
    decoding: RefCell<HashSet<PathBuf>>,
    wanted: RefCell<Vec<PathBuf>>,
    // The image asked for, and the one whose pixels are in the view (the
    // previous one stays until the new one's placeholder or texture).
    shown: RefCell<Option<PathBuf>>,
    displayed: RefCell<Option<PathBuf>>,
    playing: RefCell<Option<gtk::TickCallbackId>>,
    // What animations are fitted to.
    size: RefCell<(u32, u32)>,
    // Told the image shown and, once known, its size.
    on_info: RefCell<Option<InfoCallback>>,
}

impl Preview {
    pub fn new() -> Rc<Self> {
        let (sender, receiver) = async_channel::unbounded();
        let shared = Arc::new(Shared {
            queue: Mutex::default(),
            work: Condvar::new(),
            results: sender,
        });
        // One core is left for the main thread.
        let workers = std::thread::available_parallelism()
            .map_or(2, |n| n.get())
            .saturating_sub(1)
            .clamp(1, MAX_WORKERS);
        for _ in 0..workers {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("preview".into())
                .spawn(move || worker(&shared))
                .expect("a preview worker starts");
        }
        let image = ZoomableImage::default();
        image.set_widget_name("preview");
        let preview = Rc::new(Self {
            image,
            shared,
            textures: RefCell::default(),
            decoding: RefCell::default(),
            wanted: RefCell::default(),
            shown: RefCell::default(),
            displayed: RefCell::default(),
            playing: RefCell::default(),
            size: RefCell::new((1, 1)),
            on_info: RefCell::default(),
        });
        let weak = Rc::downgrade(&preview);
        preview.image.connect_resize(move |width, height| {
            if let Some(preview) = weak.upgrade() {
                preview.resized(width, height);
            }
        });
        let weak = Rc::downgrade(&preview);
        preview.image.connect_detail(move || {
            if let Some(preview) = weak.upgrade() {
                preview.request_detail();
            }
        });
        let weak = Rc::downgrade(&preview);
        glib::spawn_future_local(async move {
            while let Ok((path, result)) = receiver.recv().await {
                let Some(preview) = weak.upgrade() else { break };
                preview.decoded(path, result);
            }
        });
        preview
    }

    pub fn connect_info(&self, info: impl Fn(&Path, Option<(u32, u32)>) + 'static) {
        *self.on_info.borrow_mut() = Some(Box::new(info));
    }

    fn info(&self, path: &Path, full: Option<(u32, u32)>) {
        if let Some(info) = self.on_info.borrow().as_ref() {
            info(path, full);
        }
    }

    /// Shows `path` (the thumbnail `placeholder` until it's decoded) and
    /// preloads `neighbours`, nearest first, at `width`×`height` pixels.
    pub fn show(
        &self,
        path: PathBuf,
        neighbours: Vec<PathBuf>,
        placeholder: Option<gdk::Texture>,
        width: u32,
        height: u32,
    ) {
        let mut wanted = vec![path.clone()];
        wanted.extend(neighbours.into_iter().filter(|p| *p != path));
        self.want(wanted, width, height);
        *self.size.borrow_mut() = (width, height);
        self.stop_animation();
        let cached = self.textures.borrow().get(&path).cloned();
        // Decoded for another size: a placeholder, and decoded again.
        let fresh = cached
            .as_ref()
            .is_some_and(|cached| !self.stale(cached, width, height));
        if cached.is_some() && !fresh {
            self.redecode(&path, width, height);
        }
        let sharp = fresh;
        self.info(&path, cached.as_ref().map(|(_, full)| *full));
        let shown = cached
            .map(|(texture, (w, h))| (texture, (w as f64, h as f64)))
            .or_else(|| {
                placeholder.map(|texture| {
                    let size = (texture.width() as f64, texture.height() as f64);
                    (texture, size)
                })
            });
        // Without either, the previous image stays rather than flash an
        // empty view.
        if let Some((texture, full)) = shown {
            self.image.set_image(&texture, full, true);
            *self.displayed.borrow_mut() = Some(path.clone());
        }
        crate::probe::preview_shown(&path, sharp);
        if sharp && is_animation(&path) {
            self.start_animation(&path);
        }
        *self.shown.borrow_mut() = Some(path);
    }

    /// The image asked for, if the view shows one.
    pub fn shown(&self) -> Option<PathBuf> {
        self.shown.borrow().clone()
    }

    /// Drops what's decoded of `path` (changed on disk), so showing it again
    /// decodes it again.
    pub fn forget(&self, path: &Path) {
        self.textures.borrow_mut().remove(path);
        self.decoding.borrow_mut().remove(path);
    }

    /// Decodes `path` in the background (the image selected in the grid),
    /// so opening it shows it sharp at once.
    pub fn preload(&self, path: PathBuf, width: u32, height: u32) {
        self.want(vec![path], width, height);
    }

    /// Empties the view (it's hidden), keeping only `keep`'s image: the one
    /// selected in the grid, which is usually the one just shown.
    pub fn hide(&self, keep: Option<PathBuf>, width: u32, height: u32) {
        self.stop_animation();
        *self.shown.borrow_mut() = None;
        *self.displayed.borrow_mut() = None;
        self.image.clear();
        self.want(keep.into_iter().collect(), width, height);
    }

    // Keeps (and decodes, in order) exactly `wanted`; everything else is
    // dropped, and queued decodes of it never start.
    fn want(&self, wanted: Vec<PathBuf>, width: u32, height: u32) {
        self.textures
            .borrow_mut()
            .retain(|kept, _| wanted.contains(kept));
        // A path counts as decoding from being queued until its result; the
        // queue is replaced, and requests no worker had taken yet stop
        // counting, so they can be asked for again later.
        let mut queue = self.shared.queue.lock().unwrap();
        let mut decoding = self.decoding.borrow_mut();
        for dropped in queue.requests.drain(..) {
            if let Kind::Fit(..) = dropped.kind {
                decoding.remove(&dropped.path);
            }
        }
        let textures = self.textures.borrow();
        for path in &wanted {
            if !textures.contains_key(path) && decoding.insert(path.clone()) {
                queue.requests.push(Request {
                    path: path.clone(),
                    kind: Kind::Fit(width, height),
                });
            }
        }
        drop(queue);
        self.shared.work.notify_all();
        *self.wanted.borrow_mut() = wanted;
    }

    // Whether a texture isn't the size its image fits `width`×`height` at
    // (so it would be drawn scaled, which softens it).
    fn stale(&self, (texture, full): &FitTexture, width: u32, height: u32) -> bool {
        let (w, h) = decode::fitted(full.0, full.1, width, height);
        (texture.width() as u32, texture.height() as u32) != (w, h)
    }

    // The view's size changed (a resized window, fullscreen): the shown
    // image again at the new size; the rest when they're shown.
    fn resized(&self, width: u32, height: u32) {
        if *self.size.borrow() == (width, height) {
            return;
        }
        *self.size.borrow_mut() = (width, height);
        let Some(path) = self.shown.borrow().clone() else {
            return;
        };
        let cached = self.textures.borrow().get(&path).cloned();
        if cached.is_some_and(|cached| self.stale(&cached, width, height)) {
            self.redecode(&path, width, height);
        }
    }

    // `path` at `width`×`height`, ahead of preloads; its current texture
    // stays until then.
    fn redecode(&self, path: &Path, width: u32, height: u32) {
        self.decoding.borrow_mut().insert(path.to_owned());
        self.shared.queue.lock().unwrap().requests.insert(
            0,
            Request {
                path: path.to_owned(),
                kind: Kind::Fit(width, height),
            },
        );
        self.shared.work.notify_one();
    }

    // Full resolution of the image shown, ahead of any preload.
    fn request_detail(&self) {
        let Some(path) = self.shown.borrow().clone() else {
            return;
        };
        if is_animation(&path) {
            return;
        }
        self.shared.queue.lock().unwrap().requests.insert(
            0,
            Request {
                path,
                kind: Kind::Full,
            },
        );
        self.shared.work.notify_one();
    }

    fn decoded(&self, path: PathBuf, result: Result<Decoded, String>) {
        let decoded = match result {
            Ok(decoded) => decoded,
            Err(error) => {
                self.decoding.borrow_mut().remove(&path);
                eprintln!("Could not show {}: {error}", path.display());
                return;
            }
        };
        let shown = self.shown.borrow().as_ref() == Some(&path);
        match decoded {
            Decoded::Fit { pixels, full } => {
                self.decoding.borrow_mut().remove(&path);
                if !self.wanted.borrow().contains(&path) {
                    return;
                }
                let texture = pixels.texture();
                self.textures
                    .borrow_mut()
                    .insert(path.clone(), (texture.clone(), full));
                if shown {
                    let new_image = self.displayed.borrow().as_ref() != Some(&path);
                    self.image
                        .set_image(&texture, (full.0 as f64, full.1 as f64), new_image);
                    *self.displayed.borrow_mut() = Some(path.clone());
                    crate::probe::preview_shown(&path, true);
                    self.info(&path, Some(full));
                    if is_animation(&path) {
                        self.start_animation(&path);
                    }
                }
            }
            Decoded::Tiles {
                tiles,
                width,
                height,
            } => {
                if shown && self.displayed.borrow().as_ref() == Some(&path) {
                    let tiles = tiles
                        .into_iter()
                        .map(|(tile, pixels)| (tile, pixels.texture()))
                        .collect();
                    self.image.set_tiles(Tiles {
                        tiles,
                        width,
                        height,
                    });
                }
            }
        }
    }

    // Plays `path`'s frames from a thread decoding a few ahead; the view
    // swaps them in on the frame clock at each frame's delay.
    fn start_animation(&self, path: &Path) {
        self.stop_animation();
        let (sender, frames) = mpsc::sync_channel::<Frame>(FRAMES_AHEAD);
        let (path, size) = (path.to_owned(), *self.size.borrow());
        let started = std::thread::Builder::new()
            .name("animation".into())
            .spawn(move || decode_frames(&path, size, &sender));
        if started.is_err() {
            return;
        }
        let due = std::cell::Cell::new(0);
        let tick = self.image.add_tick_callback(move |image, clock| {
            let now = clock.frame_time();
            if now < due.get() {
                return glib::ControlFlow::Continue;
            }
            match frames.try_recv() {
                Ok(frame) => {
                    image.set_frame(&frame.pixels.texture());
                    crate::probe::animation_frame();
                    match frame.delay {
                        Some(delay) => {
                            due.set(now + delay.as_micros() as i64);
                            glib::ControlFlow::Continue
                        }
                        // A finite animation's last frame stays.
                        None => glib::ControlFlow::Break,
                    }
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
        *self.playing.borrow_mut() = Some(tick);
    }

    fn stop_animation(&self) {
        if let Some(tick) = self.playing.borrow_mut().take() {
            tick.remove();
        }
    }
}
