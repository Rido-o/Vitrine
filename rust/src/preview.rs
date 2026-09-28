//! The full-screen view: the shown image and its neighbours decoded on two
//! worker threads at the size they're shown at, with the thumbnail as a
//! placeholder until the shown one arrives.

use crate::decode::{self, Pixels};
use gtk::{gdk, glib};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Condvar, Mutex},
};

// Decodes at once: the shown image never waits behind more than one preload.
const WORKERS: usize = 2;

struct Request {
    path: PathBuf,
    width: u32,
    height: u32,
}

#[derive(Default)]
struct Queue {
    // Next first; replaced on every move, so passed images are never decoded.
    requests: Vec<Request>,
}

struct Shared {
    queue: Mutex<Queue>,
    work: Condvar,
    results: async_channel::Sender<(PathBuf, Result<Pixels, String>)>,
}

// VITRINE_VIEWER=full decodes at full resolution (to compare uploads).
fn full_size() -> bool {
    std::env::var("VITRINE_VIEWER").is_ok_and(|v| v == "full")
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
        let (width, height) = if full_size() {
            (u32::MAX, u32::MAX)
        } else {
            (request.width, request.height)
        };
        let result = decode::to_fit(&request.path, width, height).map(|rgba| rgba.premultiplied());
        if shared
            .results
            .send_blocking((request.path, result))
            .is_err()
        {
            return;
        }
    }
}

pub struct Preview {
    pub picture: gtk::Picture,
    shared: Arc<Shared>,
    // Decoded textures of the shown image and its neighbours.
    textures: RefCell<HashMap<PathBuf, gdk::Texture>>,
    decoding: RefCell<HashSet<PathBuf>>,
    wanted: RefCell<Vec<PathBuf>>,
    shown: RefCell<Option<PathBuf>>,
}

impl Preview {
    pub fn new() -> Rc<Self> {
        let (sender, receiver) = async_channel::unbounded();
        let shared = Arc::new(Shared {
            queue: Mutex::default(),
            work: Condvar::new(),
            results: sender,
        });
        for _ in 0..WORKERS {
            let shared = shared.clone();
            std::thread::spawn(move || worker(&shared));
        }
        let picture = gtk::Picture::builder()
            .content_fit(gtk::ContentFit::Contain)
            .can_shrink(true)
            .hexpand(true)
            .vexpand(true)
            .name("preview")
            .build();
        let preview = Rc::new(Self {
            picture,
            shared,
            textures: RefCell::default(),
            decoding: RefCell::default(),
            wanted: RefCell::default(),
            shown: RefCell::default(),
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
        let texture = self.textures.borrow().get(&path).cloned();
        let sharp = texture.is_some();
        match texture.or(placeholder) {
            Some(texture) => self.picture.set_paintable(Some(&texture)),
            // Keep the previous image rather than flash an empty view.
            None if self.shown.borrow().is_some() => {}
            None => self.picture.set_paintable(None::<&gdk::Paintable>),
        }
        crate::probe::preview_shown(&path, sharp);
        *self.shown.borrow_mut() = Some(path);
    }

    /// Decodes `path` in the background (the image selected in the grid),
    /// so opening it shows it sharp at once.
    pub fn preload(&self, path: PathBuf, width: u32, height: u32) {
        self.want(vec![path], width, height);
    }

    /// Empties the view (it's hidden), keeping only `keep`'s image: the one
    /// selected in the grid, which is usually the one just shown.
    pub fn hide(&self, keep: Option<PathBuf>, width: u32, height: u32) {
        *self.shown.borrow_mut() = None;
        self.picture.set_paintable(None::<&gdk::Paintable>);
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
            decoding.remove(&dropped.path);
        }
        let textures = self.textures.borrow();
        for path in &wanted {
            if !textures.contains_key(path) && decoding.insert(path.clone()) {
                queue.requests.push(Request {
                    path: path.clone(),
                    width,
                    height,
                });
            }
        }
        drop(queue);
        self.shared.work.notify_all();
        *self.wanted.borrow_mut() = wanted;
    }

    fn decoded(&self, path: PathBuf, result: Result<Pixels, String>) {
        self.decoding.borrow_mut().remove(&path);
        if !self.wanted.borrow().contains(&path) {
            return;
        }
        let texture = match result {
            Ok(pixels) => pixels.texture(),
            Err(error) => {
                eprintln!("Could not show {}: {error}", path.display());
                return;
            }
        };
        self.textures
            .borrow_mut()
            .insert(path.clone(), texture.clone());
        if self.shown.borrow().as_ref() == Some(&path) {
            self.picture.set_paintable(Some(&texture));
            crate::probe::preview_shown(&path, true);
        }
    }
}
