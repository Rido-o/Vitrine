//! Finding the images in a folder, on a worker thread.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, UNIX_EPOCH},
};

const EXTENSIONS: [&str; 7] = ["gif", "jpeg", "jpg", "png", "tif", "tiff", "webp"];
// A batch goes to the main thread when it's this big or this old, so the
// first images show quickly and a big folder isn't one item-changed per file.
const BATCH_SIZE: usize = 512;
const BATCH_AGE: Duration = Duration::from_millis(50);

// `mtime` and `size` are read from Phase 2 on (thumbnail cache, sorting).
#[allow(dead_code)]
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
    std::thread::spawn(move || {
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
    receiver
}
