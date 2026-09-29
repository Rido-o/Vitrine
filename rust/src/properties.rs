//! The i button's popover: the image's file, image and camera properties,
//! read on a worker each time it opens. As Properties.ts and
//! PropertiesPopover.ts; EXIF through kamadak-exif (the TypeScript app uses
//! gexiv2), with values written the way exiv2 prints them.

use exif::{Exif, In, Tag, Value};
use gtk::{gdk_pixbuf::Pixbuf, gio, glib, pango, prelude::*};
use std::{
    cell::Cell,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    rc::Rc,
};

type Rows = Vec<(&'static str, String)>;
pub type Section = (&'static str, Rows);

const FILE_ATTRIBUTES: &str = "standard::size,standard::content-type,time::modified,time::created";

/// A popover showing `path()`'s properties, read each time it opens.
pub fn popover(path: impl Fn() -> Option<PathBuf> + 'static) -> gtk::Popover {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();
    let popover = gtk::Popover::builder()
        .child(&content)
        .css_classes(["viewer-properties"])
        .build();
    // Each opening reads again; an older read that finishes late is dropped.
    let reads = Rc::new(Cell::new(0u64));
    popover.connect_show(move |_| {
        let read = reads.get() + 1;
        reads.set(read);
        let Some(path) = path() else {
            return render(&content, None);
        };
        let (content, reads) = (content.clone(), reads.clone());
        glib::spawn_future_local(async move {
            let sections = gio::spawn_blocking(move || image_properties(&path))
                .await
                .unwrap_or_default();
            if reads.get() == read {
                render(&content, Some(sections));
            }
        });
    });
    popover
}

fn render(content: &gtk::Box, sections: Option<Vec<Section>>) {
    while let Some(child) = content.first_child() {
        content.remove(&child);
    }
    let Some(sections) = sections else {
        content.append(&gtk::Label::new(Some("No image selected")));
        return;
    };
    for (title, rows) in sections {
        content.append(
            &gtk::Label::builder()
                .label(title)
                .xalign(0.0)
                .css_classes(["viewer-properties-heading"])
                .build(),
        );
        let grid = gtk::Grid::builder()
            .column_spacing(16)
            .row_spacing(4)
            .build();
        for (row, (key, value)) in rows.into_iter().enumerate() {
            let row = row as i32;
            grid.attach(
                &gtk::Label::builder()
                    .label(key)
                    .xalign(1.0)
                    .yalign(0.0)
                    .css_classes(["viewer-properties-key"])
                    .build(),
                0,
                row,
                1,
                1,
            );
            // Selectable for copying, but not focusable: the popover would
            // focus the first value and select all of it on opening.
            grid.attach(
                &gtk::Label::builder()
                    .label(value)
                    .xalign(0.0)
                    .selectable(true)
                    .focusable(false)
                    .wrap(true)
                    .wrap_mode(pango::WrapMode::WordChar)
                    .max_width_chars(48)
                    .build(),
                1,
                row,
                1,
                1,
            );
        }
        content.append(&grid);
    }
}

/// The sections, without empty rows; Camera only with EXIF data.
pub fn image_properties(path: &Path) -> Vec<Section> {
    let exif = read_exif(path);
    [
        ("File", file_rows(path)),
        ("Image", image_rows(path, exif.as_ref())),
        ("Camera", exif.as_ref().map(camera_rows).unwrap_or_default()),
    ]
    .into_iter()
    .filter(|(_, rows)| !rows.is_empty())
    .collect()
}

fn rows(entries: Vec<(&'static str, Option<String>)>) -> Rows {
    entries
        .into_iter()
        .filter_map(|(key, value)| Some((key, value.filter(|value| !value.is_empty())?)))
        .collect()
}

fn file_rows(path: &Path) -> Rows {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    let folder = path.parent().map(|folder| folder.display().to_string());
    let info = gio::File::for_path(path).query_info(
        FILE_ATTRIBUTES,
        gio::FileQueryInfoFlags::NONE,
        None::<&gio::Cancellable>,
    );
    let Ok(info) = info else {
        return rows(vec![("Name", name), ("Folder", folder)]);
    };
    let size = info.size();
    let date = |date: Option<glib::DateTime>| {
        date.and_then(|date| date.to_local().ok())
            .and_then(|date| date.format("%Y-%m-%d %H:%M:%S").ok())
            .map(|date| date.to_string())
    };
    rows(vec![
        ("Name", name),
        ("Folder", folder),
        (
            "Size",
            Some(format!(
                "{} ({} bytes)",
                glib::format_size(size as u64),
                grouped(size as u64)
            )),
        ),
        (
            "Type",
            info.content_type()
                .map(|content_type| gio::content_type_get_description(&content_type).to_string()),
        ),
        ("Modified", date(info.modification_date_time())),
        ("Created", date(info.creation_date_time())),
    ])
}

// 4213456 → "4,213,456", as the TypeScript app's toLocaleString.
fn grouped(number: u64) -> String {
    let digits = number.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

// GdkPixbuf's descriptions are inconsistent ("The WebP image format").
fn format_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "gif" => "GIF",
        "jpeg" => "JPEG",
        "png" => "PNG",
        "tiff" => "TIFF",
        "webp" => "WebP",
        _ => return None,
    })
}

// EXIF orientations 2–8; 5–8 swap width and height.
fn orientation_name(orientation: u32) -> Option<&'static str> {
    Some(match orientation {
        2 => "Flipped horizontally",
        3 => "Rotated 180°",
        4 => "Flipped vertically",
        5 => "Rotated 90° and flipped",
        6 => "Rotated 90° clockwise",
        7 => "Rotated 90° counter-clockwise and flipped",
        8 => "Rotated 90° counter-clockwise",
        _ => return None,
    })
}

fn read_exif(path: &Path) -> Option<Exif> {
    let file = File::open(path).ok()?;
    exif::Reader::new()
        .read_from_container(&mut BufReader::new(file))
        .ok()
}

/// Width × height as shown: the header's, swapped for EXIF orientations 5–8
/// (the info bar; as Dimensions here).
pub fn shown_size(path: &Path) -> Option<(i32, i32)> {
    let (_, width, height) = Pixbuf::file_info(path)?;
    let orientation = read_exif(path)
        .and_then(|exif| {
            exif.get_field(Tag::Orientation, In::PRIMARY)?
                .value
                .get_uint(0)
        })
        .unwrap_or(1);
    Some(if orientation >= 5 {
        (height, width)
    } else {
        (width, height)
    })
}

fn image_rows(path: &Path, exif: Option<&Exif>) -> Rows {
    let info = Pixbuf::file_info(path);
    let orientation = exif
        .and_then(|exif| exif.get_field(Tag::Orientation, In::PRIMARY))
        .and_then(|field| field.value.get_uint(0))
        .unwrap_or(1);
    let size = info
        .as_ref()
        .map(|(_, width, height)| (*width, *height))
        .filter(|&(width, height)| width > 0 && height > 0);
    let format = info.as_ref().and_then(|(format, _, _)| {
        format
            .name()
            .and_then(|name| format_name(&name))
            .map(str::to_owned)
            .or_else(|| {
                format
                    .description()
                    .map(|description| description.to_string())
            })
    });
    rows(vec![
        (
            "Dimensions",
            size.map(|(width, height)| {
                let (width, height) = if orientation >= 5 {
                    (height, width)
                } else {
                    (width, height)
                };
                format!("{width} × {height}")
            }),
        ),
        (
            "Megapixels",
            size.map(|(width, height)| megapixels(width as f64 * height as f64)),
        ),
        ("Format", format),
        (
            "Orientation",
            orientation_name(orientation).map(str::to_owned),
        ),
    ])
}

fn megapixels(pixels: f64) -> String {
    let mp = pixels / 1e6;
    if mp < 1.0 {
        format!("{mp:.2} MP")
    } else {
        format!("{mp:.1} MP")
    }
}

fn field<'a>(exif: &'a Exif, tag: Tag) -> Option<&'a Value> {
    exif.get_field(tag, In::PRIMARY).map(|field| &field.value)
}

fn text(exif: &Exif, tag: Tag) -> Option<String> {
    match field(exif, tag)? {
        Value::Ascii(parts) => {
            let text = parts
                .iter()
                .map(|part| {
                    String::from_utf8_lossy(part)
                        .trim_matches(['\0', ' '])
                        .to_owned()
                })
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(", ");
            Some(text).filter(|text| !text.is_empty())
        }
        other => Some(other.display_as(tag).to_string()),
    }
}

fn number(exif: &Exif, tag: Tag) -> Option<f64> {
    match field(exif, tag)? {
        Value::Rational(v) => v.first().filter(|r| r.denom != 0).map(|r| r.to_f64()),
        Value::SRational(v) => v.first().filter(|r| r.denom != 0).map(|r| r.to_f64()),
        value => value.get_uint(0).map(f64::from),
    }
}

// Decimals without trailing zeros: 2.8, 11.
fn trimmed(value: f64, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

fn camera_rows(exif: &Exif) -> Rows {
    let make = text(exif, Tag::Make);
    let model = text(exif, Tag::Model);
    let camera = match (&make, &model) {
        (Some(make), Some(model)) if !model.to_lowercase().starts_with(&make.to_lowercase()) => {
            Some(format!("{make} {model}"))
        }
        _ => model.or(make),
    };
    let exposure = number(exif, Tag::ExposureTime)
        .filter(|&t| t > 0.0)
        .map(|time| {
            if time >= 1.0 {
                format!("{} s", trimmed(time, 1))
            } else {
                format!("1/{} s", (1.0 / time).round())
            }
        });
    let aperture = number(exif, Tag::FNumber)
        .filter(|&f| f > 0.0)
        .map(|f| format!("F{}", trimmed(f, 1)));
    let iso = field(exif, Tag::PhotographicSensitivity)
        .and_then(|value| value.get_uint(0))
        .map(|iso| iso.to_string());
    let focal = number(exif, Tag::FocalLength)
        .filter(|&f| f > 0.0)
        .map(|f| format!("{f:.1} mm"));
    let focal35 = number(exif, Tag::FocalLengthIn35mmFilm)
        .filter(|&f| f > 0.0)
        .map(|f| format!("{f:.1} mm"));
    let focal = match (focal, focal35) {
        (Some(focal), Some(focal35)) => Some(format!("{focal} ({focal35} full-frame)")),
        (focal, _) => focal,
    };
    let flash = field(exif, Tag::Flash).and_then(|value| {
        let code = value.get_uint(0)?;
        Some(
            flash_name(code)
                .map_or_else(|| value.display_as(Tag::Flash).to_string(), str::to_owned),
        )
    });
    // EXIF dates are "YYYY:MM:DD HH:MM:SS".
    let taken = text(exif, Tag::DateTimeOriginal).map(|taken| {
        let (date, time) = taken.split_at(taken.len().min(10));
        format!("{}{time}", date.replace(':', "-"))
    });
    rows(vec![
        ("Camera", camera),
        ("Lens", text(exif, Tag::LensModel)),
        ("Exposure", exposure),
        ("Aperture", aperture),
        ("ISO", iso),
        ("Focal length", focal),
        ("Flash", flash),
        ("Taken", taken),
        ("Location", gps_location(exif)),
        ("Software", text(exif, Tag::Software)),
        ("Artist", text(exif, Tag::Artist)),
        ("Copyright", text(exif, Tag::Copyright)),
    ])
}

// As exiv2 words them.
fn flash_name(code: u32) -> Option<&'static str> {
    Some(match code {
        0x00 => "No flash",
        0x01 => "Fired",
        0x05 => "Fired, return light not detected",
        0x07 => "Fired, return light detected",
        0x08 => "Yes, did not fire",
        0x09 => "Yes, compulsory",
        0x0d => "Yes, compulsory, return light not detected",
        0x0f => "Yes, compulsory, return light detected",
        0x10 => "No, compulsory",
        0x14 => "No, did not fire, return light not detected",
        0x18 => "No, auto",
        0x19 => "Yes, auto",
        0x1d => "Yes, auto, return light not detected",
        0x1f => "Yes, auto, return light detected",
        0x20 => "No flash function",
        0x30 => "No, no flash function",
        0x41 => "Yes, red-eye reduction",
        0x45 => "Yes, red-eye reduction, return light not detected",
        0x47 => "Yes, red-eye reduction, return light detected",
        0x49 => "Yes, compulsory, red-eye reduction",
        0x4d => "Yes, compulsory, red-eye reduction, return light not detected",
        0x4f => "Yes, compulsory, red-eye reduction, return light detected",
        0x50 => "No, red-eye reduction",
        0x58 => "No, auto, red-eye reduction",
        0x59 => "Yes, auto, red-eye reduction",
        0x5d => "Yes, auto, red-eye reduction, return light not detected",
        0x5f => "Yes, auto, red-eye reduction, return light detected",
        _ => return None,
    })
}

// Degrees, minutes and seconds with N/S (E/W) to signed decimal degrees.
fn coordinate(exif: &Exif, tag: Tag, reference: Tag, negative: &str) -> Option<f64> {
    let Value::Rational(parts) = field(exif, tag)? else {
        return None;
    };
    let mut degrees = 0.0;
    for (part, scale) in parts.iter().zip([1.0, 60.0, 3600.0]) {
        if part.denom == 0 {
            return None;
        }
        degrees += part.to_f64() / scale;
    }
    let sign = if text(exif, reference).is_some_and(|r| r.eq_ignore_ascii_case(negative)) {
        -1.0
    } else {
        1.0
    };
    Some(sign * degrees)
}

fn gps_location(exif: &Exif) -> Option<String> {
    let latitude = coordinate(exif, Tag::GPSLatitude, Tag::GPSLatitudeRef, "S")?;
    let longitude = coordinate(exif, Tag::GPSLongitude, Tag::GPSLongitudeRef, "W")?;
    let position = format!("{latitude:.5}, {longitude:.5}");
    Some(match number(exif, Tag::GPSAltitude) {
        Some(altitude) => {
            let below =
                field(exif, Tag::GPSAltitudeRef).and_then(|value| value.get_uint(0)) == Some(1);
            let altitude = if below { -altitude } else { altitude };
            format!("{position} ({} m)", altitude.round())
        }
        None => position,
    })
}
