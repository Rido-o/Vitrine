//! The grid's side of thumbnails, on the main thread: which tiles show which
//! thumbnail, a memory cache of textures, and requests to the worker pool.

use crate::library::Image;
use crate::thumbnails::{self, Done, Job, Outcome, Pool};
use gtk::{gdk, glib, prelude::*};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
};

// Textures kept in memory (~560 KB each at 440×320), least recently used
// dropped first.
const MAX_TEXTURES: usize = 300;

#[derive(Default)]
struct TextureCache {
    textures: HashMap<String, (gdk::Texture, u64)>,
    clock: u64,
}

impl TextureCache {
    fn get(&mut self, key: &str) -> Option<gdk::Texture> {
        self.clock += 1;
        let clock = self.clock;
        self.textures.get_mut(key).map(|(texture, used)| {
            *used = clock;
            texture.clone()
        })
    }

    fn insert(&mut self, key: String, texture: gdk::Texture) {
        self.clock += 1;
        self.textures.insert(key, (texture, self.clock));
        if self.textures.len() > MAX_TEXTURES {
            let oldest = self
                .textures
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                self.textures.remove(&oldest);
            }
        }
    }
}

pub struct Tiles {
    pool: Pool,
    cache: RefCell<TextureCache>,
    // Keys requested from the pool and not answered yet.
    loading: RefCell<HashSet<String>>,
    // Bound tiles: each picture's key and image, and each key's pictures.
    keys: RefCell<HashMap<gtk::Picture, (String, PathBuf)>>,
    pictures: RefCell<HashMap<String, Vec<gtk::Picture>>>,
    // Shown for images that couldn't be read, so they aren't retried.
    failed: gdk::Texture,
}

impl Tiles {
    pub fn new(cache_dir: PathBuf) -> Rc<Self> {
        let (pool, results) = Pool::new(cache_dir);
        let failed = gdk::MemoryTexture::new(
            1,
            1,
            gdk::MemoryFormat::R8g8b8a8Premultiplied,
            &glib::Bytes::from_static(&[0, 0, 0, 0]),
            4,
        )
        .upcast();
        let tiles = Rc::new(Self {
            pool,
            cache: RefCell::default(),
            loading: RefCell::default(),
            keys: RefCell::default(),
            pictures: RefCell::default(),
            failed,
        });
        let weak = Rc::downgrade(&tiles);
        glib::spawn_future_local(async move {
            while let Ok(done) = results.recv().await {
                let Some(tiles) = weak.upgrade() else { break };
                tiles.done(done);
            }
        });
        tiles
    }

    pub fn bind(&self, picture: &gtk::Picture, image: &Image) {
        let key = thumbnails::key(&image.path, image.mtime);
        self.keys
            .borrow_mut()
            .insert(picture.clone(), (key.clone(), image.path.clone()));
        self.pictures
            .borrow_mut()
            .entry(key.clone())
            .or_default()
            .push(picture.clone());
        self.pool.want(&key);
        let texture = self.cache.borrow_mut().get(&key);
        picture.set_paintable(texture.as_ref());
        if texture.is_none() && self.loading.borrow_mut().insert(key.clone()) {
            self.pool.request(Job {
                key,
                path: image.path.clone(),
            });
        }
    }

    pub fn unbind(&self, picture: &gtk::Picture) {
        let Some((key, _)) = self.keys.borrow_mut().remove(picture) else {
            return;
        };
        let mut pictures = self.pictures.borrow_mut();
        if let Some(list) = pictures.get_mut(&key) {
            list.retain(|p| p != picture);
            if list.is_empty() {
                pictures.remove(&key);
            }
        }
        self.pool.unwant(&key);
    }

    /// Generates the missing thumbnails of all `images` in the background.
    pub fn set_background(&self, images: &[Image]) {
        self.pool.set_background(
            images
                .iter()
                .map(|image| Job {
                    key: thumbnails::key(&image.path, image.mtime),
                    path: image.path.clone(),
                })
                .collect(),
        );
    }

    fn done(&self, done: Done) {
        let (key, outcome) = match done {
            Done::Thumbnail { key, outcome } => (key, outcome),
            Done::BackgroundFinished { generated } => {
                crate::probe::background_finished(generated);
                return;
            }
        };
        self.loading.borrow_mut().remove(&key);
        let texture = match outcome {
            Outcome::Loaded(pixels) => pixels.texture(),
            Outcome::Failed => self.failed.clone(),
            Outcome::Skipped => {
                // Bound again after the worker decided nobody wanted it.
                let path = self.bound_path(&key);
                if let Some(path) = path {
                    self.loading.borrow_mut().insert(key.clone());
                    self.pool.request(Job { key, path });
                }
                return;
            }
        };
        self.cache.borrow_mut().insert(key.clone(), texture.clone());
        if let Some(pictures) = self.pictures.borrow().get(&key) {
            for picture in pictures {
                picture.set_paintable(Some(&texture));
            }
        }
    }

    // The image a tile bound to `key` shows, if any is.
    fn bound_path(&self, key: &str) -> Option<PathBuf> {
        let pictures = self.pictures.borrow();
        let picture = pictures.get(key)?.first()?;
        self.keys
            .borrow()
            .get(picture)
            .map(|(_, path)| path.clone())
    }
}
