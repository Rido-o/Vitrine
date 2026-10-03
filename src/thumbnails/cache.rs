//! The on-disk thumbnail cache: files named after the image's path and
//! modification time, touched when used and pruned when unused for long;
//! earlier versions' caches are deleted.

use super::{lower_priority, pool::Job};
use crate::decode::{self, Rgba};
use gtk::glib;
use std::{
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

const JPEG_QUALITY: i32 = 85;
// Cache files are named after the image's path and mtime, so edited, moved or
// deleted images leave orphans. Files are touched when used (at most daily)
// and pruned once unused for PRUNE_AFTER.
const PRUNE_AFTER: Duration = Duration::from_secs(90 * 24 * 60 * 60);
const TOUCH_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
// A partly written thumbnail this old was left by a process that died
// (another may be writing a newer one right now).
const PART_AFTER: Duration = Duration::from_secs(60 * 60);

/// "-4": thumbnails in sRGB (JPEG, or PNG with transparency, keyed by
/// `key`); "-3" had embedded colour profiles ignored, the TypeScript app's
/// were in "-2".
pub fn cache_dir() -> PathBuf {
    glib::user_cache_dir().join("vitrine/thumbnails-4")
}

// Caches of earlier versions, deleted rather than migrated: thumbnails
// regenerate.
fn old_caches() -> [PathBuf; 5] {
    let cache = glib::user_cache_dir();
    [
        cache.join("vitrine/thumbnails"),
        cache.join("vitrine/thumbnails-2"),
        cache.join("vitrine/thumbnails-3"),
        cache.join("vitrine-spike"),
        cache.join("shard-view"),
    ]
}

/// The cache key: a thumbnail belongs to a file's path and modification time.
/// The path's bytes, so names that aren't UTF-8 don't collide (the same key
/// as the text for those that are).
pub fn key(path: &Path, mtime: i64) -> String {
    let mut data = path.as_os_str().as_bytes().to_vec();
    data.extend_from_slice(format!("\n{mtime}").as_bytes());
    glib::compute_checksum_for_data(glib::ChecksumType::Md5, &data)
        .map(|sum| sum.to_string())
        .unwrap_or_default()
}

// The cached thumbnail's file, if any, touched when it's a day old.
pub(super) fn cached_file(cache_dir: &Path, key: &str) -> Option<PathBuf> {
    let file = ["jpg", "png"]
        .iter()
        .map(|ext| cache_dir.join(format!("{key}.{ext}")))
        .find(|path| path.exists())?;
    let stale = std::fs::metadata(&file)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| modified.elapsed().is_ok_and(|age| age > TOUCH_AFTER));
    if stale && let Ok(opened) = std::fs::File::options().write(true).open(&file) {
        let _ = opened.set_modified(SystemTime::now());
    }
    Some(file)
}

pub(super) fn load_cached(file: &Path) -> Result<Rgba, String> {
    let data = std::fs::read(file).map_err(|error| error.to_string())?;
    if file.extension().is_some_and(|ext| ext == "png") {
        decode::decode_png(&data)
    } else {
        decode::decode_jpeg(&data)
    }
}

// Written under a temporary name: a request and a background job can generate
// the same thumbnail at once. Transparent thumbnails are PNG, the rest JPEG.
pub(super) fn save(cache_dir: &Path, job: &Job, rgba: &Rgba) {
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
        .and_then(|()| writer.finish())
        .map_err(|error| error.to_string())?;
    Ok(out)
}

/// Deletes cache files unused for PRUNE_AFTER, and earlier versions' caches. On a thread of its own, at low priority.
pub fn housekeeping() {
    let spawn = std::thread::Builder::new()
        .name("housekeeping".into())
        .spawn(|| {
            lower_priority();
            let pruned = prune(&cache_dir());
            if pruned > 0 {
                println!("Pruned {pruned} unused thumbnails");
            }
            for dir in old_caches() {
                match std::fs::remove_dir_all(&dir) {
                    Ok(()) => println!("Removed outdated thumbnails in {}", dir.display()),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => eprintln!("Could not remove {}: {error}", dir.display()),
                }
            }
        });
    if let Err(error) = spawn {
        eprintln!("Could not start housekeeping: {error}");
    }
}

fn prune(cache_dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(cache_dir) else {
        return 0;
    };
    // A thumbnail, or one whose writing never finished (`save`, killed
    // mid-write): unused for this long.
    let unused_after = |name: &str| {
        let (stem, ext) = name.split_once('.').unwrap_or((name, ""));
        if stem.len() != 32 || !stem.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        match ext {
            "jpg" | "png" => Some(PRUNE_AFTER),
            _ if ext.ends_with(".part") => Some(PART_AFTER),
            _ => None,
        }
    };
    let mut pruned = 0;
    for entry in entries.flatten() {
        let Some(after) = entry.file_name().to_str().and_then(unused_after) else {
            continue;
        };
        let unused = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| modified.elapsed().is_ok_and(|age| age > after));
        if unused && std::fs::remove_file(entry.path()).is_ok() {
            pruned += 1;
        }
    }
    pruned
}
