//! Thumbnails: a pool of worker threads that load them from the disk cache or
//! generate them, and hand back pixels ready for a texture. Only `Pixels`
//! cross threads; textures are made on the main thread (`Pixels::texture`).

use crate::decode::{self, Pixels, Rgba};
use gtk::glib;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::Instant,
};

// Thumbnails fit within this (smaller images keep their size).
const WIDTH: u32 = 440;
const HEIGHT: u32 = 320;
const JPEG_QUALITY: i32 = 85;
const WORKER_NICE: libc::c_int = 10;

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
        match load_cached(&file) {
            Ok(rgba) => return Ok((rgba.premultiplied(), false)),
            Err(error) => eprintln!("Bad cached thumbnail {}: {error}", file.display()),
        }
    }
    generate(cache_dir, job).map(|pixels| (pixels, true))
}

fn load_cached(file: &Path) -> Result<Rgba, String> {
    let data = std::fs::read(file).map_err(|error| error.to_string())?;
    if file.extension().is_some_and(|ext| ext == "png") {
        decode::decode_png(&data)
    } else {
        decode::decode_jpeg(&data)
    }
}

fn generate(cache_dir: &Path, job: &Job) -> Result<Pixels, String> {
    let started = Instant::now();
    let rgba = decode::to_fit(&job.path, WIDTH, HEIGHT)?;
    save(cache_dir, job, &rgba);
    if std::env::var_os("VITRINE_PROBE_VERBOSE").is_some() {
        eprintln!(
            "generated {} in {:?}",
            job.path.display(),
            started.elapsed()
        );
    }
    Ok(rgba.premultiplied())
}

// Written under a temporary name: a request and a background job can generate
// the same thumbnail at once. Transparent thumbnails are PNG, the rest JPEG.
fn save(cache_dir: &Path, job: &Job, rgba: &Rgba) {
    let transparent = rgba.transparent();
    let ext = if transparent { "png" } else { "jpg" };
    let target = cache_dir.join(format!("{}.{ext}", job.key));
    let part = cache_dir.join(format!(
        "{}.{:?}.part",
        job.key,
        std::thread::current().id()
    ));
    let encoded = if transparent {
        encode_png(rgba)
    } else {
        encode_jpeg(rgba)
    };
    let saved = encoded
        .and_then(|bytes| std::fs::write(&part, bytes).map_err(|e| e.to_string()))
        .and_then(|()| std::fs::rename(&part, &target).map_err(|e| e.to_string()));
    if let Err(error) = saved {
        let _ = std::fs::remove_file(&part);
        eprintln!("Could not cache {}: {error}", job.path.display());
    }
}

fn encode_jpeg(rgba: &Rgba) -> Result<Vec<u8>, String> {
    let image = turbojpeg::Image {
        pixels: &rgba.data[..],
        width: rgba.width as usize,
        pitch: rgba.width as usize * 4,
        height: rgba.height as usize,
        format: turbojpeg::PixelFormat::RGBX,
    };
    turbojpeg::compress(image, JPEG_QUALITY, turbojpeg::Subsamp::Sub2x2)
        .map(|buf| buf.to_vec())
        .map_err(|error| error.to_string())
}

// Fast compression: the same size as the default here, ~40× quicker.
fn encode_png(rgba: &Rgba) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, rgba.width, rgba.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(&rgba.data)
        .map_err(|error| error.to_string())?;
    drop(writer);
    Ok(out)
}
