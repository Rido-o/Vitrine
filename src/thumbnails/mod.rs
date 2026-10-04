//! Thumbnails: a pool of worker threads (`pool.rs`) that load them from the
//! disk cache (`cache.rs`) or generate them, and hand back pixels ready for a
//! texture; `tiles.rs` is the grid's side, on the main thread. Only `Pixels`
//! cross threads; textures are made on the main thread (`Pixels::texture`).
//! A thumbnail comes with its image's own size, which the info bar shows.

mod cache;
mod pool;
pub mod tiles;

use cache::key;
pub use cache::{cache_dir, housekeeping};
use pool::{Done, Job, Outcome, Pool};

// Thumbnails fit within this (smaller images keep their size).
const WIDTH: u32 = 440;
const HEIGHT: u32 = 320;
const WORKER_NICE: libc::c_int = 10;

// Workers run below the main thread: with every core busy generating, the
// main thread otherwise waited its turn and missed frames.
fn lower_priority() {
    // On Linux, PRIO_PROCESS with 0 applies to the calling thread only.
    // SAFETY: setpriority takes no pointers; failure just leaves the priority.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, WORKER_NICE);
    }
}
