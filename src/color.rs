//! Embedded ICC colour profiles: read from JPEG and PNG files and from
//! GdkPixbuf's images, and converted to sRGB (what GTK takes every texture to
//! be) on the worker that decoded the image.

use crate::decode::Rgba;
use gtk::{gdk_pixbuf::Pixbuf, glib};
use moxcms::{ColorProfile, DataColorSpace, Layout, Transform8BitExecutor, TransformOptions};
use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::{Arc, Mutex, OnceLock},
};

type Transform = Arc<Transform8BitExecutor>;

// Profiles seen, by a hash of their bytes: a folder of photos from one camera
// or phone shares one, and building a transform takes a few ms.
const MAX_CACHED: usize = 32;

/// Converts `image` (straight RGBA) from the colour space of `icc` to sRGB.
/// Without a profile, with an sRGB one, or with one that isn't RGB (grey,
/// CMYK) or can't be read, the pixels stay as they are.
pub fn to_srgb(image: &mut Rgba, icc: Option<&[u8]>) {
    let Some(transform) = icc.and_then(transform) else {
        return;
    };
    let mut converted = vec![0; image.data.len()];
    match transform.transform(&image.data, &mut converted) {
        Ok(()) => image.data = converted,
        Err(error) => eprintln!("Colour conversion failed: {error}"),
    }
}

fn transform(icc: &[u8]) -> Option<Transform> {
    static CACHE: OnceLock<Mutex<HashMap<u64, Option<Transform>>>> = OnceLock::new();
    let mut hasher = DefaultHasher::new();
    icc.hash(&mut hasher);
    let key = hasher.finish();
    let mut cache = CACHE.get_or_init(Mutex::default).lock().ok()?;
    if let Some(transform) = cache.get(&key) {
        return transform.clone();
    }
    let transform = build(icc);
    if cache.len() >= MAX_CACHED {
        cache.clear();
    }
    cache.insert(key, transform.clone());
    transform
}

fn build(icc: &[u8]) -> Option<Transform> {
    let profile = ColorProfile::new_from_slice(icc).ok()?;
    let srgb = ColorProfile::new_srgb();
    if profile.color_space != DataColorSpace::Rgb || same_primaries(&profile, &srgb) {
        return None;
    }
    profile
        .create_transform_8bit(
            Layout::Rgba,
            &srgb,
            Layout::Rgba,
            TransformOptions::default(),
        )
        .map_err(|error| eprintln!("Unusable colour profile: {error}"))
        .ok()
}

// sRGB profiles differ slightly in their numbers (and some cameras tag sRGB
// with a plain 2.2 curve), too little to see; converting would only cost
// time.
fn same_primaries(a: &ColorProfile, b: &ColorProfile) -> bool {
    let close = |a: moxcms::Xyzd, b: moxcms::Xyzd| {
        (a.x - b.x).abs() < 0.002 && (a.y - b.y).abs() < 0.002 && (a.z - b.z).abs() < 0.002
    };
    close(a.red_colorant, b.red_colorant)
        && close(a.green_colorant, b.green_colorant)
        && close(a.blue_colorant, b.blue_colorant)
}

/// A JPEG's profile: its APP2 `ICC_PROFILE` segments, in order.
pub fn jpeg_profile(jpeg: &[u8]) -> Option<Vec<u8>> {
    const SIGNATURE: &[u8] = b"ICC_PROFILE\0";
    if !jpeg.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut chunks = Vec::new();
    let mut at = 2;
    while let [0xFF, marker, ..] = jpeg.get(at..).unwrap_or_default() {
        match marker {
            // Padding.
            0xFF => at += 1,
            // Start of scan, end of image: the headers are over.
            0xDA | 0xD9 => break,
            // Markers without a length.
            0x01 | 0xD0..=0xD7 => at += 2,
            _ => {
                let length = u16::from_be_bytes([*jpeg.get(at + 2)?, *jpeg.get(at + 3)?]) as usize;
                let segment = jpeg.get(at + 4..at + 2 + length.max(2))?;
                if *marker == 0xE2
                    && let Some([sequence, _count, data @ ..]) = segment.strip_prefix(SIGNATURE)
                {
                    chunks.push((*sequence, data));
                }
                at += 2 + length;
            }
        }
    }
    if chunks.is_empty() {
        return None;
    }
    chunks.sort_by_key(|(sequence, _)| *sequence);
    Some(
        chunks
            .into_iter()
            .flat_map(|(_, data)| data)
            .copied()
            .collect(),
    )
}

/// The profile GdkPixbuf's loader read (JPEG, PNG, TIFF, and WebP with a
/// recent loader), if any.
pub fn pixbuf_profile(pixbuf: &Pixbuf) -> Option<Vec<u8>> {
    let encoded = pixbuf.option("icc-profile")?;
    Some(glib::base64_decode(&encoded))
}
