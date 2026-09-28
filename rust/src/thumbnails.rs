//! Thumbnails: a pool of worker threads that load them from the disk cache or
//! generate them, and hand back pixels ready for a texture. Only `Pixels`
//! cross threads; textures are made on the main thread (`Pixels::texture`).

use gtk::{gdk, gdk_pixbuf::Pixbuf, glib};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::Instant,
};

// Thumbnails fit within this (smaller images keep their size).
pub const WIDTH: i32 = 440;
pub const HEIGHT: i32 = 320;
const JPEG_QUALITY: &str = "85";
const WORKER_NICE: libc::c_int = 10;

/// Premultiplied RGBA (what GTK uploads as is; plain RGB it would convert on
/// the main thread).
pub struct Pixels {
    width: i32,
    height: i32,
    data: Vec<u8>,
}

impl Pixels {
    pub fn texture(self) -> gdk::Texture {
        let stride = self.width as usize * 4;
        gdk::MemoryTexture::new(
            self.width,
            self.height,
            gdk::MemoryFormat::R8g8b8a8Premultiplied,
            &glib::Bytes::from_owned(self.data),
            stride,
        )
        .into()
    }
}

pub enum Outcome {
    Loaded(Pixels),
    Failed,
    /// No tile wanted it any more by the time its turn came.
    Skipped,
}

pub enum Done {
    Thumbnail {
        key: String,
        outcome: Outcome,
    },
    /// Every background job has finished.
    BackgroundFinished {
        generated: usize,
    },
}

#[derive(Clone)]
pub struct Job {
    pub key: String,
    pub path: PathBuf,
}

/// The cache key: a thumbnail belongs to a file's path and modification time.
pub fn key(path: &Path, mtime: i64) -> String {
    glib::compute_checksum_for_string(
        glib::ChecksumType::Md5,
        format!("{}\n{mtime}", path.display()),
    )
    .map(|sum| sum.to_string())
    .unwrap_or_default()
}

#[derive(Default)]
struct State {
    // Tiles' requests, newest last (taken first).
    requested: Vec<Job>,
    background: Vec<Job>,
    background_next: usize,
    background_active: usize,
    background_generated: usize,
    background_reported: bool,
    // How many bound tiles want each key; a request nobody wants is skipped.
    wanted: HashMap<String, u32>,
}

struct Shared {
    state: Mutex<State>,
    work: Condvar,
    cache_dir: PathBuf,
    background_limit: usize,
    results: async_channel::Sender<Done>,
}

pub struct Pool {
    shared: Arc<Shared>,
}

impl Pool {
    pub fn new(cache_dir: PathBuf) -> (Self, async_channel::Receiver<Done>) {
        let _ = std::fs::create_dir_all(&cache_dir);
        let workers = std::thread::available_parallelism()
            .map_or(2, |n| n.get())
            .saturating_sub(1)
            .max(1);
        let (sender, receiver) = async_channel::unbounded();
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            work: Condvar::new(),
            cache_dir,
            background_limit: (workers / 2).max(1),
            results: sender,
        });
        for _ in 0..workers {
            let shared = shared.clone();
            std::thread::spawn(move || {
                lower_priority();
                worker(&shared)
            });
        }
        (Self { shared }, receiver)
    }

    pub fn want(&self, key: &str) {
        let mut state = self.shared.state.lock().unwrap();
        *state.wanted.entry(key.to_owned()).or_default() += 1;
    }

    pub fn unwant(&self, key: &str) {
        let mut state = self.shared.state.lock().unwrap();
        if let Some(count) = state.wanted.get_mut(key) {
            *count -= 1;
            if *count == 0 {
                state.wanted.remove(key);
            }
        }
    }

    pub fn request(&self, job: Job) {
        self.shared.state.lock().unwrap().requested.push(job);
        self.shared.work.notify_one();
    }

    /// Generates the missing thumbnails of `jobs` while no tile is waiting.
    pub fn set_background(&self, jobs: Vec<Job>) {
        let mut state = self.shared.state.lock().unwrap();
        state.background = jobs;
        state.background_next = 0;
        state.background_generated = 0;
        state.background_reported = false;
        drop(state);
        self.shared.work.notify_all();
    }
}

// Workers run below the main thread: with every core busy generating, the
// main thread otherwise waited its turn and missed frames.
fn lower_priority() {
    // On Linux, PRIO_PROCESS with 0 applies to the calling thread only.
    // SAFETY: setpriority takes no pointers; failure just leaves the priority.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, WORKER_NICE);
    }
}

enum Task {
    Requested(Job),
    Background(Job),
}

fn next_task(shared: &Shared) -> Task {
    let mut state = shared.state.lock().unwrap();
    loop {
        if let Some(job) = state.requested.pop() {
            return Task::Requested(job);
        }
        if state.background_active < shared.background_limit
            && state.background_next < state.background.len()
        {
            let job = state.background[state.background_next].clone();
            state.background_next += 1;
            state.background_active += 1;
            return Task::Background(job);
        }
        state = shared.work.wait(state).unwrap();
    }
}

fn worker(shared: &Shared) {
    loop {
        match next_task(shared) {
            Task::Requested(job) => {
                let wanted = shared.state.lock().unwrap().wanted.contains_key(&job.key);
                let outcome = if !wanted {
                    Outcome::Skipped
                } else {
                    match thumbnail(&shared.cache_dir, &job) {
                        Ok((pixels, _)) => Outcome::Loaded(pixels),
                        Err(error) => {
                            eprintln!("No thumbnail for {}: {error}", job.path.display());
                            Outcome::Failed
                        }
                    }
                };
                let _ = shared.results.send_blocking(Done::Thumbnail {
                    key: job.key,
                    outcome,
                });
            }
            Task::Background(job) => {
                let generated = cached_file(&shared.cache_dir, &job.key).is_none()
                    && matches!(thumbnail(&shared.cache_dir, &job), Ok((_, true)));
                let mut state = shared.state.lock().unwrap();
                state.background_active -= 1;
                state.background_generated += usize::from(generated);
                if state.background_active == 0
                    && state.background_next >= state.background.len()
                    && !state.background_reported
                {
                    state.background_reported = true;
                    let _ = shared.results.send_blocking(Done::BackgroundFinished {
                        generated: state.background_generated,
                    });
                }
                drop(state);
                // Another background job may now be allowed to start.
                shared.work.notify_one();
            }
        }
    }
}

fn cached_file(cache_dir: &Path, key: &str) -> Option<PathBuf> {
    ["jpg", "png"]
        .iter()
        .map(|ext| cache_dir.join(format!("{key}.{ext}")))
        .find(|path| path.exists())
}

/// The thumbnail's pixels, from the cache or generated (then true).
fn thumbnail(cache_dir: &Path, job: &Job) -> Result<(Pixels, bool), String> {
    if let Some(file) = cached_file(cache_dir, &job.key) {
        match Pixbuf::from_file(&file) {
            Ok(pixbuf) => return Ok((to_pixels(&pixbuf).0, false)),
            Err(error) => eprintln!("Bad cached thumbnail {}: {error}", file.display()),
        }
    }
    generate(cache_dir, job).map(|pixels| (pixels, true))
}

fn generate(cache_dir: &Path, job: &Job) -> Result<Pixels, String> {
    let started = Instant::now();
    let (_, width, height) = Pixbuf::file_info(&job.path).ok_or("unknown format")?;
    let pixbuf = if width <= WIDTH && height <= HEIGHT {
        Pixbuf::from_file(&job.path)
    } else {
        Pixbuf::from_file_at_scale(&job.path, WIDTH, HEIGHT, true)
    }
    .map_err(|error| error.to_string())?;
    let pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
    let (pixels, transparent) = to_pixels(&pixbuf);

    // Written under a temporary name: a request and a background job can
    // generate the same thumbnail at once.
    let (kind, ext, options): (&str, &str, &[(&str, &str)]) = if transparent {
        ("png", "png", &[])
    } else {
        ("jpeg", "jpg", &[("quality", JPEG_QUALITY)])
    };
    let target = cache_dir.join(format!("{}.{ext}", job.key));
    let part = cache_dir.join(format!(
        "{}.{:?}.part",
        job.key,
        std::thread::current().id()
    ));
    let saved = pixbuf
        .savev(&part, kind, options)
        .map_err(|error| error.to_string())
        .and_then(|()| std::fs::rename(&part, &target).map_err(|error| error.to_string()));
    if let Err(error) = saved {
        let _ = std::fs::remove_file(&part);
        eprintln!("Could not cache {}: {error}", job.path.display());
    }
    if std::env::var_os("VITRINE_PROBE_VERBOSE").is_some() {
        eprintln!(
            "generated {} in {:?}",
            job.path.display(),
            started.elapsed()
        );
    }
    Ok(pixels)
}

/// Premultiplied RGBA, and whether any pixel is transparent.
fn to_pixels(pixbuf: &Pixbuf) -> (Pixels, bool) {
    let width = pixbuf.width();
    let height = pixbuf.height();
    let channels = pixbuf.n_channels() as usize;
    let stride = pixbuf.rowstride() as usize;
    let bytes = pixbuf.read_pixel_bytes();
    let mut data = Vec::with_capacity(width as usize * height as usize * 4);
    let mut transparent = false;
    for y in 0..height as usize {
        let row = &bytes[y * stride..];
        for x in 0..width as usize {
            let pixel = &row[x * channels..x * channels + channels];
            let alpha = if channels == 4 { pixel[3] } else { 255 };
            if alpha == 255 {
                data.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            } else {
                transparent = true;
                let premultiply = |c: u8| ((c as u16 * alpha as u16 + 127) / 255) as u8;
                data.extend_from_slice(&[
                    premultiply(pixel[0]),
                    premultiply(pixel[1]),
                    premultiply(pixel[2]),
                    alpha,
                ]);
            }
        }
    }
    (
        Pixels {
            width,
            height,
            data,
        },
        transparent,
    )
}
