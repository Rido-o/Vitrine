//! Decoding images to fit within a size, on any thread: JPEG through
//! libjpeg-turbo at a reduced size (1/2, 1/4, 1/8…), PNG through `png`, then a
//! SIMD resize; everything else (and anything those fail on) through
//! GdkPixbuf. EXIF orientation is applied, read the same way for every format
//! (`orientation`), and embedded colour profiles are converted to sRGB
//! (`color.rs`).

mod color;

use gtk::{gdk, gdk_pixbuf::Pixbuf, glib};
use std::{
    path::Path,
    sync::{OnceLock, mpsc},
};

/// Straight (not premultiplied) RGBA, rows packed.
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Premultiplied RGBA, what GTK uploads as is (plain RGB it would convert on
/// the main thread). Crosses threads; made into a texture on the main one.
pub struct Pixels {
    pub width: i32,
    pub height: i32,
    data: Buffer,
}

// Buffers this big are freed on a background thread: returning a
// full-screen image's 33 MB to the system (munmap) took 1–4 ms, and happened
// on the main thread when a texture was dropped (in the grid's selection
// handler, delaying the highlight by a frame at 144 Hz).
const FREE_ELSEWHERE: usize = 1 << 20;

/// Pixel memory that frees itself off the main thread when large.
struct Buffer(Vec<u8>);

impl AsRef<[u8]> for Buffer {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        let data = std::mem::take(&mut self.0);
        if data.len() >= FREE_ELSEWHERE {
            // If the thread is gone, the buffer is freed here as usual.
            let _ = janitor().send(data);
        }
    }
}

fn janitor() -> &'static mpsc::Sender<Vec<u8>> {
    static SENDER: OnceLock<mpsc::Sender<Vec<u8>>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<Vec<u8>>();
        std::thread::Builder::new()
            .name("janitor".into())
            .spawn(move || receiver.into_iter().for_each(drop))
            .expect("the janitor thread starts");
        sender
    })
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

impl Rgba {
    pub fn from_pixbuf(pixbuf: &Pixbuf) -> Self {
        let width = pixbuf.width() as usize;
        let height = pixbuf.height() as usize;
        let channels = pixbuf.n_channels() as usize;
        let stride = pixbuf.rowstride() as usize;
        let bytes = pixbuf.read_pixel_bytes();
        let mut data = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            let row = &bytes[y * stride..];
            for x in 0..width {
                let pixel = &row[x * channels..x * channels + channels];
                let alpha = if channels == 4 { pixel[3] } else { 255 };
                data.extend_from_slice(&[pixel[0], pixel[1], pixel[2], alpha]);
            }
        }
        Self {
            width: width as u32,
            height: height as u32,
            data,
        }
    }

    pub fn transparent(&self) -> bool {
        self.data
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] < 255)
    }

    pub fn premultiplied(mut self) -> Pixels {
        for pixel in self.data.as_chunks_mut::<4>().0 {
            let alpha = pixel[3] as u16;
            if alpha < 255 {
                for c in &mut pixel[..3] {
                    *c = ((*c as u16 * alpha + 127) / 255) as u8;
                }
            }
        }
        Pixels {
            width: self.width as i32,
            height: self.height as i32,
            data: Buffer(self.data),
        }
    }
}

/// An image decoded to fit a size, and the image's own (full) size, both as
/// shown (after EXIF rotation).
pub struct Fitted {
    pub rgba: Rgba,
    pub full: (u32, u32),
}

/// `path` decoded to fit within `max_w`×`max_h` (as shown, after EXIF
/// rotation), never scaled up.
pub fn to_fit(path: &Path, max_w: u32, max_h: u32) -> Result<Fitted, String> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let fast = match ext.as_str() {
        "jpg" | "jpeg" => jpeg_to_fit(path, max_w, max_h),
        "png" => png_to_fit(path, max_w, max_h),
        _ => return pixbuf_to_fit(path, max_w, max_h),
    };
    fast.or_else(|error| {
        eprintln!("Falling back to GdkPixbuf for {}: {error}", path.display());
        pixbuf_to_fit(path, max_w, max_h)
    })
}

pub fn fitted(width: u32, height: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let scale = (max_w as f64 / width as f64)
        .min(max_h as f64 / height as f64)
        .min(1.0);
    (
        ((width as f64 * scale).round() as u32).max(1),
        ((height as f64 * scale).round() as u32).max(1),
    )
}

/// The EXIF orientation (1–8) of `path`, 1 without one: what decoding
/// applies, and what the info bar and properties go by.
pub fn orientation(path: &Path) -> u32 {
    std::fs::File::open(path).map_or(1, |file| orientation_in(&mut std::io::BufReader::new(file)))
}

/// As `orientation`, from EXIF already read.
pub fn orientation_of(exif: &exif::Exif) -> u32 {
    exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .and_then(|field| field.value.get_uint(0))
        .filter(|orientation| (1..=8).contains(orientation))
        .unwrap_or(1)
}

fn orientation_in(container: &mut (impl std::io::BufRead + std::io::Seek)) -> u32 {
    exif::Reader::new()
        .read_from_container(container)
        .map_or(1, |exif| orientation_of(&exif))
}

/// Orientations 5–8 swap width and height.
pub fn swaps(orientation: u32) -> bool {
    (5..=8).contains(&orientation)
}

type Size = (u32, u32);

// The stored size that fits within `max_w`×`max_h` once turned upright, and
// the image's own size as shown.
fn fit_stored(width: u32, height: u32, max_w: u32, max_h: u32, orientation: u32) -> (Size, Size) {
    if swaps(orientation) {
        let (w, h) = fitted(height, width, max_w, max_h);
        ((h, w), (height, width))
    } else {
        (fitted(width, height, max_w, max_h), (width, height))
    }
}

// Converted to sRGB and turned upright, the last steps of every decode.
fn finish(mut rgba: Rgba, icc: Option<&[u8]>, orientation: u32, full: Size) -> Fitted {
    color::to_srgb(&mut rgba, icc);
    Fitted {
        rgba: orient(rgba, orientation),
        full,
    }
}

fn pixbuf_to_fit(path: &Path, max_w: u32, max_h: u32) -> Result<Fitted, String> {
    let (_, width, height) = Pixbuf::file_info(path).ok_or("unknown format")?;
    let (width, height) = (width as u32, height as u32);
    let orientation = orientation(path);
    let (target, full) = fit_stored(width, height, max_w, max_h, orientation);
    let pixbuf = if target == (width, height) {
        Pixbuf::from_file(path)
    } else {
        Pixbuf::from_file_at_scale(path, target.0 as i32, target.1 as i32, true)
    }
    .map_err(|error| error.to_string())?;
    let icc = color::pixbuf_profile(&pixbuf);
    Ok(finish(
        Rgba::from_pixbuf(&pixbuf),
        icc.as_deref(),
        orientation,
        full,
    ))
}

fn jpeg_to_fit(path: &Path, max_w: u32, max_h: u32) -> Result<Fitted, String> {
    let data = std::fs::read(path).map_err(|error| error.to_string())?;
    let orientation = orientation_in(&mut std::io::Cursor::new(&data));
    let mut decompressor = turbojpeg::Decompressor::new().map_err(|error| error.to_string())?;
    let header = decompressor
        .read_header(&data)
        .map_err(|error| error.to_string())?;
    let (stored_w, stored_h) = (header.width as u32, header.height as u32);
    let ((target_w, target_h), full) = fit_stored(stored_w, stored_h, max_w, max_h, orientation);
    let factor = turbojpeg::Decompressor::supported_scaling_factors()
        .into_iter()
        .filter(|f| f.num() <= f.denom())
        .filter(|f| {
            f.scale(stored_w as usize) >= target_w as usize
                && f.scale(stored_h as usize) >= target_h as usize
        })
        .min_by(|a, b| (a.num() * b.denom()).cmp(&(b.num() * a.denom())))
        .unwrap_or(turbojpeg::ScalingFactor::ONE);
    decompressor
        .set_scaling_factor(factor)
        .map_err(|error| error.to_string())?;
    let (decoded_w, decoded_h) = (
        factor.scale(stored_w as usize),
        factor.scale(stored_h as usize),
    );
    let mut pixels = vec![0; decoded_w * decoded_h * 4];
    decompressor
        .decompress(
            &data,
            turbojpeg::Image {
                pixels: &mut pixels[..],
                width: decoded_w,
                pitch: decoded_w * 4,
                height: decoded_h,
                format: turbojpeg::PixelFormat::RGBA,
            },
        )
        .map_err(|error| error.to_string())?;
    let decoded = Rgba {
        width: decoded_w as u32,
        height: decoded_h as u32,
        data: pixels,
    };
    // Converted once resized: far fewer pixels.
    let rgba = resize(decoded, target_w, target_h)?;
    let icc = color::jpeg_profile(&data);
    Ok(finish(rgba, icc.as_deref(), orientation, full))
}

// PNG can't decode smaller: in full, then resized (alpha-aware).
fn png_to_fit(path: &Path, max_w: u32, max_h: u32) -> Result<Fitted, String> {
    let data = std::fs::read(path).map_err(|error| error.to_string())?;
    let orientation = orientation_in(&mut std::io::Cursor::new(&data));
    let (image, profile) = decode_png_with_profile(&data)?;
    let (target, full) = fit_stored(image.width, image.height, max_w, max_h, orientation);
    let rgba = resize(image, target.0, target.1)?;
    Ok(finish(rgba, profile.as_deref(), orientation, full))
}

pub fn resize(image: Rgba, width: u32, height: u32) -> Result<Rgba, String> {
    use fast_image_resize as fr;
    if image.width == width && image.height == height {
        return Ok(image);
    }
    let source =
        fr::images::Image::from_vec_u8(image.width, image.height, image.data, fr::PixelType::U8x4)
            .map_err(|error| error.to_string())?;
    let mut target = fr::images::Image::new(width, height, fr::PixelType::U8x4);
    thread_local! {
        static RESIZER: std::cell::RefCell<fr::Resizer> = std::cell::RefCell::new(fr::Resizer::new());
    }
    let options =
        fr::ResizeOptions::new().resize_alg(fr::ResizeAlg::Convolution(fr::FilterType::Lanczos3));
    RESIZER
        .with_borrow_mut(|resizer| resizer.resize(&source, &mut target, &options))
        .map_err(|error| error.to_string())?;
    Ok(Rgba {
        width,
        height,
        data: target.into_vec(),
    })
}

/// Applies an EXIF orientation (1–8) to the pixels.
fn orient(image: Rgba, orientation: u32) -> Rgba {
    if orientation == 1 {
        return image;
    }
    let (w, h) = (image.width as usize, image.height as usize);
    let (out_w, out_h) = if swaps(orientation) { (h, w) } else { (w, h) };
    let mut data = vec![0; image.data.len()];
    for y in 0..h {
        for x in 0..w {
            // Where the stored pixel (x, y) ends up.
            let (nx, ny) = match orientation {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (h - 1 - y, x),
                7 => (h - 1 - y, w - 1 - x),
                _ => (y, w - 1 - x), // 8
            };
            let from = (y * w + x) * 4;
            let to = (ny * out_w + nx) * 4;
            data[to..to + 4].copy_from_slice(&image.data[from..from + 4]);
        }
    }
    Rgba {
        width: out_w as u32,
        height: out_h as u32,
        data,
    }
}

/// A PNG decoded in full (the thumbnail cache's own files, sRGB).
pub fn decode_png(data: &[u8]) -> Result<Rgba, String> {
    decode_png_with_profile(data).map(|(image, _)| image)
}

// With its embedded colour profile (iCCP), if any.
fn decode_png_with_profile(data: &[u8]) -> Result<(Rgba, Option<Vec<u8>>), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
    // ALPHA leaves only RGBA and grey with alpha.
    decoder.set_transformations(png::Transformations::ALPHA | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let profile = reader.info().icc_profile.as_ref().map(|icc| icc.to_vec());
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("PNG too large")?];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| error.to_string())?;
    buffer.truncate(info.buffer_size());
    let data = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::GrayscaleAlpha => buffer
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        other => return Err(format!("unexpected PNG colour type {other:?}")),
    };
    Ok((
        Rgba {
            width: info.width,
            height: info.height,
            data,
        },
        profile,
    ))
}

/// A JPEG decoded in full (the thumbnail cache's own files).
pub fn decode_jpeg(data: &[u8]) -> Result<Rgba, String> {
    let image = turbojpeg::decompress(data, turbojpeg::PixelFormat::RGBA)
        .map_err(|error| error.to_string())?;
    Ok(Rgba {
        width: image.width as u32,
        height: image.height as u32,
        data: image.pixels,
    })
}

/// A piece of a full-resolution image: `x`, `y`, `width`, `height` in the
/// image. Its pixels (`split`) have a `pad`-pixel border copied from the
/// neighbours (none at the image's edges), so adjacent tiles overlap and
/// smoothing shows no seams; `pad_left` and `pad_top` are where the tile
/// starts in them.
pub struct Tile {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub pad_left: u32,
    pub pad_top: u32,
}

/// Splits `image` into tiles of at most `size` pixels square, row by row,
/// each with its pixels.
pub fn split(image: Rgba, size: u32, pad: u32) -> (Vec<(Tile, Pixels)>, u32, u32) {
    let (width, height) = (image.width, image.height);
    let pixels = image.premultiplied();
    let source = pixels.data.as_ref();
    let stride = width as usize * 4;
    let mut tiles = Vec::new();
    for y in (0..height).step_by(size as usize) {
        for x in (0..width).step_by(size as usize) {
            let (w, h) = (size.min(width - x), size.min(height - y));
            let left = x.saturating_sub(pad);
            let top = y.saturating_sub(pad);
            let right = (x + w + pad).min(width);
            let bottom = (y + h + pad).min(height);
            let (tw, th) = (right - left, bottom - top);
            let mut data = Vec::with_capacity(tw as usize * th as usize * 4);
            for row in top..bottom {
                let start = row as usize * stride + left as usize * 4;
                data.extend_from_slice(&source[start..start + tw as usize * 4]);
            }
            let tile = Tile {
                x,
                y,
                width: w,
                height: h,
                pad_left: x - left,
                pad_top: y - top,
            };
            let pixels = Pixels {
                width: tw as i32,
                height: th as i32,
                data: Buffer(data),
            };
            tiles.push((tile, pixels));
        }
    }
    (tiles, width, height)
}
