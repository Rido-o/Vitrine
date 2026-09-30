//! The worker threads: the tiles' requests first (newest first, skipped once
//! no tile wants them), then, while none wait, the folder's missing
//! thumbnails in the background on half of them.

use super::{HEIGHT, WIDTH, cache, lower_priority};
use crate::decode::{self, Pixels};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::Instant,
};

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
            std::thread::Builder::new()
                .name("thumbnail".into())
                .spawn(move || {
                    lower_priority();
                    worker(&shared)
                })
                .expect("a thumbnail worker starts");
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
                let generated = cache::cached_file(&shared.cache_dir, &job.key).is_none()
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

/// The thumbnail's pixels, from the cache or generated (then true).
fn thumbnail(cache_dir: &Path, job: &Job) -> Result<(Pixels, bool), String> {
    if let Some(file) = cache::cached_file(cache_dir, &job.key) {
        match cache::load_cached(&file) {
            Ok(rgba) => return Ok((rgba.premultiplied(), false)),
            Err(error) => eprintln!("Bad cached thumbnail {}: {error}", file.display()),
        }
    }
    generate(cache_dir, job).map(|pixels| (pixels, true))
}

fn generate(cache_dir: &Path, job: &Job) -> Result<Pixels, String> {
    let started = Instant::now();
    let rgba = decode::to_fit(&job.path, WIDTH, HEIGHT)?.rgba;
    cache::save(cache_dir, job, &rgba);
    if std::env::var_os("VITRINE_PROBE_VERBOSE").is_some() {
        eprintln!(
            "generated {} in {:?}",
            job.path.display(),
            started.elapsed()
        );
    }
    Ok(rgba.premultiplied())
}
