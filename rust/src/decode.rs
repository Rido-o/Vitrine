//! Decoding images to fit within a size, on any thread: JPEG through
//! libjpeg-turbo at a reduced size (1/2, 1/4, 1/8…), PNG through `png`, then a
//! SIMD resize; everything else (and anything those fail on) through
//! GdkPixbuf. EXIF orientation is applied.

use gtk::{gdk, gdk_pixbuf::Pixbuf, glib};
use std::path::Path;

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
            data: self.data,
        }
    }
}

/// `path` decoded to fit within `max_w`×`max_h` (as shown, after EXIF
/// rotation), never scaled up.
pub fn to_fit(path: &Path, max_w: u32, max_h: u32) -> Result<Rgba, String> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let fast = match ext.as_str() {
        "jpg" | "jpeg" => Some(jpeg_to_fit(path, max_w, max_h)),
        "png" => Some(png_to_fit(path, max_w, max_h)),
        _ => None,
    };
    match fast {
        Some(Ok(rgba)) => Ok(rgba),
        Some(Err(error)) => {
            eprintln!("Falling back to GdkPixbuf for {}: {error}", path.display());
            pixbuf_to_fit(path, max_w, max_h)
        }
        None => pixbuf_to_fit(path, max_w, max_h),
    }
}

fn fitted(width: u32, height: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let scale = (max_w as f64 / width as f64)
        .min(max_h as f64 / height as f64)
        .min(1.0);
    (
        ((width as f64 * scale).round() as u32).max(1),
        ((height as f64 * scale).round() as u32).max(1),
    )
}

// GdkPixbuf fits the stored size, before EXIF rotation (GIF, TIFF and WebP
// rarely have one).
fn pixbuf_to_fit(path: &Path, max_w: u32, max_h: u32) -> Result<Rgba, String> {
    let (_, width, height) = Pixbuf::file_info(path).ok_or("unknown format")?;
    let pixbuf = if width as u32 <= max_w && height as u32 <= max_h {
        Pixbuf::from_file(path)
    } else {
        Pixbuf::from_file_at_scale(path, max_w as i32, max_h as i32, true)
    }
    .map_err(|error| error.to_string())?;
    let pixbuf = pixbuf.apply_embedded_orientation().unwrap_or(pixbuf);
    Ok(Rgba::from_pixbuf(&pixbuf))
}

fn jpeg_to_fit(path: &Path, max_w: u32, max_h: u32) -> Result<Rgba, String> {
    let data = std::fs::read(path).map_err(|error| error.to_string())?;
    let orientation = exif_orientation(&data);
    let mut decompressor = turbojpeg::Decompressor::new().map_err(|error| error.to_string())?;
    let header = decompressor
        .read_header(&data)
        .map_err(|error| error.to_string())?;
    let (stored_w, stored_h) = (header.width as u32, header.height as u32);
    // Orientations 5–8 swap width and height.
    let swapped = (5..=8).contains(&orientation);
    let (target_w, target_h) = if swapped {
        let (w, h) = fitted(stored_h, stored_w, max_w, max_h);
        (h, w)
    } else {
        fitted(stored_w, stored_h, max_w, max_h)
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
    Ok(orient(resize(decoded, target_w, target_h)?, orientation))
}

// PNG can't decode smaller: in full, then resized (alpha-aware).
fn png_to_fit(path: &Path, max_w: u32, max_h: u32) -> Result<Rgba, String> {
    let data = std::fs::read(path).map_err(|error| error.to_string())?;
    let image = decode_png(&data)?;
    let (width, height) = fitted(image.width, image.height, max_w, max_h);
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

pub fn decode_png(data: &[u8]) -> Result<Rgba, String> {
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
