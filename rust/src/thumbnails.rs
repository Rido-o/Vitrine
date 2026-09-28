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

// VITRINE_DECODER=pixbuf: Phase 2a's GdkPixbuf-only path, for comparison.
fn pixbuf_only() -> bool {
    static PIXBUF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PIXBUF.get_or_init(|| std::env::var("VITRINE_DECODER").is_ok_and(|v| v == "pixbuf"))
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
    if pixbuf_only() {
        let pixbuf = Pixbuf::from_file(file).map_err(|error| error.to_string())?;
        return Ok(Rgba::from_pixbuf(&pixbuf));
    }
    let data = std::fs::read(file).map_err(|error| error.to_string())?;
    if file.extension().is_some_and(|ext| ext == "png") {
        decode_png(&data)
    } else {
        let image = turbojpeg::decompress(&data, turbojpeg::PixelFormat::RGBA)
            .map_err(|error| error.to_string())?;
        Ok(Rgba {
            width: image.width as u32,
            height: image.height as u32,
            data: image.pixels,
        })
    }
}

fn generate(cache_dir: &Path, job: &Job) -> Result<Pixels, String> {
    let started = Instant::now();
    let ext = job
        .path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    // JPEG, WebP and PNG decode at (or close to) the thumbnail's size; the
    // rest, and anything these fail on, goes through GdkPixbuf.
    let fast: Option<Decode> = match ext.as_str() {
        _ if pixbuf_only() => None,
        "jpg" | "jpeg" => Some(jpeg_thumbnail),
        "webp" => Some(webp_thumbnail),
        "png" => Some(png_thumbnail),
        _ => None,
    };
    let rgba = match fast.map(|decode| decode(&job.path)) {
        Some(Ok(rgba)) => rgba,
        Some(Err(error)) => {
            eprintln!("Fast decode failed on {}: {error}", job.path.display());
            pixbuf_thumbnail(&job.path)?
        }
        None => pixbuf_thumbnail(&job.path)?,
    };
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

type Decode = fn(&Path) -> Result<Rgba, String>;

/// Straight (not premultiplied) RGBA, rows packed.
struct Rgba {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

impl Rgba {
    fn from_pixbuf(pixbuf: &Pixbuf) -> Self {
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

    fn transparent(&self) -> bool {
        self.data
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] < 255)
    }

    fn premultiplied(mut self) -> Pixels {
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
            data: self.data,
        }
    }
}

// The size an image of `width`×`height` (as shown, after EXIF rotation) gets
// as a thumbnail: fitted within WIDTH×HEIGHT, never scaled up.
fn fitted(width: u32, height: u32) -> (u32, u32) {
    let scale = (WIDTH as f64 / width as f64)
        .min(HEIGHT as f64 / height as f64)
        .min(1.0);
    (
        ((width as f64 * scale).round() as u32).max(1),
        ((height as f64 * scale).round() as u32).max(1),
    )
}

fn pixbuf_thumbnail(path: &Path) -> Result<Rgba, String> {
    let (_, width, height) = Pixbuf::file_info(path).ok_or("unknown format")?;
    let pixbuf = if width <= WIDTH && height <= HEIGHT {
        Pixbuf::from_file(path)
    } else {
        Pixbuf::from_file_at_scale(path, WIDTH, HEIGHT, true)
    }
    .map_err(|error| error.to_string())?;
    let pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
    Ok(Rgba::from_pixbuf(&pixbuf))
}

/// JPEG: decoded by libjpeg-turbo at the smallest reduced size (1/2, 1/4,
/// 1/8…) that still covers the thumbnail, then resized (SIMD) and rotated.
fn jpeg_thumbnail(path: &Path) -> Result<Rgba, String> {
    let data = std::fs::read(path).map_err(|error| error.to_string())?;
    let orientation = exif_orientation(&data);
    let mut decompressor = turbojpeg::Decompressor::new().map_err(|error| error.to_string())?;
    let header = decompressor
        .read_header(&data)
        .map_err(|error| error.to_string())?;
    let (stored_w, stored_h) = (header.width as u32, header.height as u32);
    // Orientations 5–8 swap width and height.
    let swapped = (5..=8).contains(&orientation);
    let (shown_w, shown_h) = if swapped {
        (stored_h, stored_w)
    } else {
        (stored_w, stored_h)
    };
    let (fit_w, fit_h) = fitted(shown_w, shown_h);
    let (target_w, target_h) = if swapped {
        (fit_h, fit_w)
    } else {
        (fit_w, fit_h)
    };

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
    let resized = resize(decoded, target_w, target_h)?;
    Ok(orient(resized, orientation))
}

/// WebP: libwebp scales while decoding (no full-size bitmap). Animated WebP
/// is left to GdkPixbuf (an error here).
fn webp_thumbnail(path: &Path) -> Result<Rgba, String> {
    use libwebp_sys as webp;
    let data = std::fs::read(path).map_err(|error| error.to_string())?;
    // SAFETY: `config` is initialised by libwebp before use; the output buffer
    // is ours (`is_external_memory`), sized for width × height × 4 with this
    // stride, and outlives the WebPDecode call.
    unsafe {
        let mut config: webp::WebPDecoderConfig = std::mem::zeroed();
        if !webp::WebPInitDecoderConfig(&mut config) {
            return Err("libwebp version mismatch".into());
        }
        if webp::WebPGetFeatures(data.as_ptr(), data.len(), &mut config.input)
            != webp::VP8StatusCode::VP8_STATUS_OK
        {
            return Err("not a WebP file".into());
        }
        if config.input.has_animation != 0 {
            return Err("animated".into());
        }
        let (width, height) = fitted(config.input.width as u32, config.input.height as u32);
        if width != config.input.width as u32 || height != config.input.height as u32 {
            config.options.use_scaling = 1;
            config.options.scaled_width = width as i32;
            config.options.scaled_height = height as i32;
        }
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        config.output.colorspace = webp::WEBP_CSP_MODE::MODE_RGBA;
        config.output.is_external_memory = 1;
        config.output.u.RGBA = webp::WebPRGBABuffer {
            rgba: pixels.as_mut_ptr(),
            stride: width as i32 * 4,
            size: pixels.len(),
        };
        let status = webp::WebPDecode(data.as_ptr(), data.len(), &mut config);
        webp::WebPFreeDecBuffer(&mut config.output);
        if status != webp::VP8StatusCode::VP8_STATUS_OK {
            return Err(format!("libwebp: {status:?}"));
        }
        Ok(Rgba {
            width,
            height,
            data: pixels,
        })
    }
}

/// PNG: decoded in full (PNG can't decode smaller), then resized (SIMD,
/// alpha-aware).
fn png_thumbnail(path: &Path) -> Result<Rgba, String> {
    let data = std::fs::read(path).map_err(|error| error.to_string())?;
    let image = decode_png(&data)?;
    let (width, height) = fitted(image.width, image.height);
    resize(image, width, height)
}

fn resize(image: Rgba, width: u32, height: u32) -> Result<Rgba, String> {
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
        fr::ResizeOptions::new().resize_alg(fr::ResizeAlg::Convolution(fr::FilterType::CatmullRom));
    RESIZER
        .with_borrow_mut(|resizer| resizer.resize(&source, &mut target, &options))
        .map_err(|error| error.to_string())?;
    Ok(Rgba {
        width,
        height,
        data: target.into_vec(),
    })
}

fn exif_orientation(jpeg: &[u8]) -> u32 {
    exif::Reader::new()
        .read_from_container(&mut std::io::Cursor::new(jpeg))
        .ok()
        .and_then(|exif| {
            exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                .and_then(|field| field.value.get_uint(0))
        })
        .filter(|orientation| (1..=8).contains(orientation))
        .unwrap_or(1)
}

/// Applies an EXIF orientation (1–8) to the pixels.
fn orient(image: Rgba, orientation: u32) -> Rgba {
    if orientation == 1 {
        return image;
    }
    let (w, h) = (image.width as usize, image.height as usize);
    let swapped = (5..=8).contains(&orientation);
    let (out_w, out_h) = if swapped { (h, w) } else { (w, h) };
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

fn decode_png(data: &[u8]) -> Result<Rgba, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(data));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let mut buffer = vec![0; reader.output_buffer_size().ok_or("PNG too large")?];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| error.to_string())?;
    buffer.truncate(info.buffer_size());
    let data = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buffer
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        _ => buffer.iter().flat_map(|&g| [g, g, g, 255]).collect(),
    };
    Ok(Rgba {
        width: info.width,
        height: info.height,
        data,
    })
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
    let encoded = if pixbuf_only() {
        save_pixbuf(rgba, transparent, &part)
    } else if transparent {
        encode_png(rgba).and_then(|bytes| std::fs::write(&part, bytes).map_err(|e| e.to_string()))
    } else {
        encode_jpeg(rgba).and_then(|bytes| std::fs::write(&part, bytes).map_err(|e| e.to_string()))
    };
    let saved = encoded.and_then(|()| std::fs::rename(&part, &target).map_err(|e| e.to_string()));
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
    turbojpeg::compress(
        image,
        JPEG_QUALITY.parse().unwrap_or(85),
        turbojpeg::Subsamp::Sub2x2,
    )
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

fn save_pixbuf(rgba: &Rgba, transparent: bool, part: &Path) -> Result<(), String> {
    let pixbuf = Pixbuf::from_bytes(
        &glib::Bytes::from(&rgba.data[..]),
        gtk::gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        rgba.width as i32,
        rgba.height as i32,
        rgba.width as i32 * 4,
    );
    let (kind, options): (&str, &[(&str, &str)]) = if transparent {
        ("png", &[])
    } else {
        ("jpeg", &[("quality", JPEG_QUALITY)])
    };
    // JPEG has no alpha channel; GdkPixbuf wants an opaque pixbuf for it.
    let pixbuf = if transparent {
        pixbuf
    } else {
        pixbuf
            .composite_color_simple(
                rgba.width as i32,
                rgba.height as i32,
                gtk::gdk_pixbuf::InterpType::Nearest,
                255,
                1,
                0,
                0,
            )
            .unwrap_or(pixbuf)
    };
    pixbuf
        .savev(part, kind, options)
        .map_err(|error| error.to_string())
}
