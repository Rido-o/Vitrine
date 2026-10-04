//! The on-disk thumbnail cache: files named after the image's path and
//! modification time, each holding the image's own size too (so showing it
//! doesn't need the image), touched when used and pruned when unused for
//! long; earlier versions' caches are deleted.

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
// What a thumbnail's comment (JPEG) or text chunk (PNG) with the image's
// size is marked with: "vitrine:size 6000x4000".
const SIZE_TAG: &str = "vitrine:size";

/// An image's own size, as shown.
pub type Size = (u32, u32);

/// "-5": thumbnails in sRGB (JPEG, or PNG with transparency, keyed by
/// `key`) that hold the image's size; "-4" lacked the size, "-3" had embedded
/// colour profiles ignored, the TypeScript app's were in "-2".
pub fn cache_dir() -> PathBuf {
    glib::user_cache_dir().join("vitrine/thumbnails-5")
}

// Caches of earlier versions, deleted rather than migrated: thumbnails
// regenerate.
fn old_caches() -> [PathBuf; 6] {
    let cache = glib::user_cache_dir();
    [
        cache.join("vitrine/thumbnails"),
        cache.join("vitrine/thumbnails-2"),
        cache.join("vitrine/thumbnails-3"),
        cache.join("vitrine/thumbnails-4"),
        cache.join("vitrine-spike"),
        cache.join("shard-view"),
    ]
}

// The folder history's files, from when the entry listed recent folders.
fn old_history() -> [PathBuf; 2] {
    let state = glib::user_state_dir();
    [
        state.join("vitrine/history"),
        state.join("vitrine-spike/history"),
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

/// The thumbnail and the size of its image.
pub(super) fn load_cached(file: &Path) -> Result<(Rgba, Option<Size>), String> {
    let data = std::fs::read(file).map_err(|error| error.to_string())?;
    if file.extension().is_some_and(|ext| ext == "png") {
        decode::decode_png(&data).map(|rgba| (rgba, png_size(&data)))
    } else {
        decode::decode_jpeg(&data).map(|rgba| (rgba, jpeg_size(&data)))
    }
}

// Written under a temporary name: a request and a background job can generate
// the same thumbnail at once. Transparent thumbnails are PNG, the rest JPEG.
pub(super) fn save(cache_dir: &Path, job: &Job, rgba: &Rgba, full: Size) {
    let transparent = rgba.transparent();
    let ext = if transparent { "png" } else { "jpg" };
    let target = cache_dir.join(format!("{}.{ext}", job.key));
    let part = cache_dir.join(format!(
        "{}.{:?}.part",
        job.key,
        std::thread::current().id()
    ));
    let encoded = if transparent {
        encode_png(rgba, full)
    } else {
        encode_jpeg(rgba).map(|bytes| with_comment(bytes, full))
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

// The image's size in a comment segment, after the JFIF header (which has to
// come first).
fn with_comment(mut jpeg: Vec<u8>, (width, height): Size) -> Vec<u8> {
    let text = format!("{SIZE_TAG} {width}x{height}");
    let mut segment = vec![0xff, 0xfe];
    segment.extend_from_slice(&(text.len() as u16 + 2).to_be_bytes());
    segment.extend_from_slice(text.as_bytes());
    let at = match jpeg.get(2..6) {
        Some(&[0xff, 0xe0, high, low]) => 4 + usize::from(u16::from_be_bytes([high, low])),
        _ => 2,
    };
    let at = at.min(jpeg.len());
    jpeg.splice(at..at, segment);
    jpeg
}

// The size `with_comment` wrote: the segments before the image data.
fn jpeg_size(jpeg: &[u8]) -> Option<Size> {
    let mut at = 2;
    loop {
        let &[0xff, code, high, low] = jpeg.get(at..at + 4)? else {
            return None;
        };
        let end = at + 2 + usize::from(u16::from_be_bytes([high, low]));
        match code {
            0xfe => {
                let text = std::str::from_utf8(jpeg.get(at + 4..end)?).ok()?;
                if let Some(size) = text.strip_prefix(SIZE_TAG) {
                    return parse_size(size.trim_start());
                }
            }
            0xda => return None,
            _ => {}
        }
        at = end;
    }
}

// The size `encode_png` wrote: the chunks before the image data.
fn png_size(png: &[u8]) -> Option<Size> {
    let mut at = 8;
    loop {
        let header = png.get(at..at + 8)?;
        let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let data = at + 8;
        match &header[4..] {
            b"tEXt" => {
                // The keyword, a zero, the text.
                let chunk = png.get(data..data.checked_add(length)?)?;
                let (keyword, text) = chunk.split_at(chunk.iter().position(|&byte| byte == 0)?);
                if keyword == SIZE_TAG.as_bytes() {
                    return parse_size(std::str::from_utf8(&text[1..]).ok()?);
                }
            }
            b"IDAT" => return None,
            _ => {}
        }
        // The chunk and its CRC.
        at = data.checked_add(length)?.checked_add(4)?;
    }
}

fn parse_size(text: &str) -> Option<Size> {
    let (width, height) = text.split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?))
}

// Fast compression: the same size as the default here, ~40× quicker.
fn encode_png(rgba: &Rgba, (width, height): Size) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, rgba.width, rgba.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    encoder
        .add_text_chunk(SIZE_TAG.into(), format!("{width}x{height}"))
        .map_err(|error| error.to_string())?;
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(&rgba.data)
        .and_then(|()| writer.finish())
        .map_err(|error| error.to_string())?;
    Ok(out)
}

/// Deletes cache files unused for PRUNE_AFTER, and earlier versions' caches
/// and folder history. On a thread of its own, at low priority.
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
            for file in old_history() {
                match std::fs::remove_file(&file) {
                    Ok(()) => {
                        println!("Removed the folder history, {}", file.display());
                        // Its folder too, if that leaves it empty.
                        let _ = file.parent().map(std::fs::remove_dir);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => eprintln!("Could not remove {}: {error}", file.display()),
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
