import GdkPixbuf from "gi://GdkPixbuf"
import GExiv2 from "gi://GExiv2?version=0.16"
import Gio from "gi://Gio"
import GLib from "gi://GLib"
import { basename } from "./util"

export type PropertySection = { title: string; rows: Array<[string, string]> }

GExiv2.initialize()

const FILE_ATTRIBUTES =
  "standard::size,standard::content-type,time::modified,time::created"

// GdkPixbuf's descriptions are inconsistent ("The WebP image format").
const FORMATS: Record<string, string> = {
  gif: "GIF",
  jpeg: "JPEG",
  png: "PNG",
  tiff: "TIFF",
  webp: "WebP",
}

// EXIF orientations 2–8; 5–8 swap width and height.
const ORIENTATIONS: Record<number, string> = {
  2: "Flipped horizontally",
  3: "Rotated 180°",
  4: "Flipped vertically",
  5: "Rotated 90° and flipped",
  6: "Rotated 90° clockwise",
  7: "Rotated 90° counter-clockwise and flipped",
  8: "Rotated 90° counter-clockwise",
}

function megapixels(pixels: number) {
  const mp = pixels / 1e6
  return `${mp.toFixed(mp < 1 ? 2 : 1)} MP`
}

function formatDate(date: GLib.DateTime | null) {
  return date?.to_local()?.format("%Y-%m-%d %H:%M:%S") ?? null
}

// Sections for the properties panel; rows without a value are left out, and
// the Camera section only appears when the file has EXIF data.
export function imageProperties(path: string): PropertySection[] {
  const metadata = readMetadata(path)
  return [
    { title: "File", rows: fileRows(path) },
    { title: "Image", rows: imageRows(path, metadata) },
    { title: "Camera", rows: metadata ? cameraRows(metadata) : [] },
  ].filter((section) => section.rows.length > 0)
}

function rows(entries: Array<[string, string | null | undefined]>) {
  return entries.filter((entry): entry is [string, string] => !!entry[1])
}

function readMetadata(path: string) {
  try {
    const metadata = new GExiv2.Metadata()
    metadata.open_path(path)
    return metadata
  } catch {
    return null
  }
}

function fileRows(path: string) {
  let info: Gio.FileInfo
  try {
    info = Gio.File.new_for_path(path).query_info(
      FILE_ATTRIBUTES,
      Gio.FileQueryInfoFlags.NONE,
      null,
    )
  } catch {
    return rows([
      ["Name", basename(path)],
      ["Folder", GLib.path_get_dirname(path)],
    ])
  }
  const size = info.get_size()
  const contentType = info.get_content_type()
  return rows([
    ["Name", basename(path)],
    ["Folder", GLib.path_get_dirname(path)],
    ["Size", `${GLib.format_size(size)} (${size.toLocaleString()} bytes)`],
    ["Type", contentType && Gio.content_type_get_description(contentType)],
    ["Modified", formatDate(info.get_modification_date_time())],
    ["Created", formatDate(info.get_creation_date_time())],
  ])
}

function imageRows(path: string, metadata: GExiv2.Metadata | null) {
  const [format, width, height] = GdkPixbuf.Pixbuf.get_file_info(path)
  let orientation = 1
  try {
    orientation = metadata?.try_get_orientation() ?? 1
  } catch {}
  const [shownWidth, shownHeight] =
    orientation >= 5 ? [height, width] : [width, height]
  const hasSize = format !== null && width > 0 && height > 0
  const formatName =
    format && (FORMATS[format.get_name() ?? ""] ?? format.get_description())
  return rows([
    ["Dimensions", hasSize ? `${shownWidth} × ${shownHeight}` : null],
    ["Megapixels", hasSize ? megapixels(width * height) : null],
    ["Format", formatName],
    ["Orientation", ORIENTATIONS[orientation]],
  ])
}

function tag(metadata: GExiv2.Metadata, name: string) {
  try {
    return metadata.try_get_tag_interpreted_string(name)?.trim() || null
  } catch {
    return null
  }
}

function cameraRows(metadata: GExiv2.Metadata) {
  const make = tag(metadata, "Exif.Image.Make")
  const model = tag(metadata, "Exif.Image.Model")
  const camera =
    make && model && !model.toLowerCase().startsWith(make.toLowerCase())
      ? `${make} ${model}`
      : (model ?? make)
  const focal = tag(metadata, "Exif.Photo.FocalLength")
  const focal35 = tag(metadata, "Exif.Photo.FocalLengthIn35mmFilm")
  // EXIF dates are "YYYY:MM:DD HH:MM:SS".
  const taken = tag(metadata, "Exif.Photo.DateTimeOriginal")?.replace(
    /^(\d{4}):(\d{2}):(\d{2})/,
    "$1-$2-$3",
  )
  return rows([
    ["Camera", camera],
    ["Lens", tag(metadata, "Exif.Photo.LensModel")],
    ["Exposure", tag(metadata, "Exif.Photo.ExposureTime")],
    ["Aperture", tag(metadata, "Exif.Photo.FNumber")],
    ["ISO", tag(metadata, "Exif.Photo.ISOSpeedRatings")],
    [
      "Focal length",
      focal && focal35 ? `${focal} (${focal35} full-frame)` : focal,
    ],
    ["Flash", tag(metadata, "Exif.Photo.Flash")],
    ["Taken", taken],
    ["Location", gpsLocation(metadata)],
    ["Software", tag(metadata, "Exif.Image.Software")],
    ["Artist", tag(metadata, "Exif.Image.Artist")],
    ["Copyright", tag(metadata, "Exif.Image.Copyright")],
  ])
}

function gpsLocation(metadata: GExiv2.Metadata) {
  try {
    // Reports success with zeros when there's no GPS data at all.
    if (!metadata.try_has_tag("Exif.GPSInfo.GPSLatitude")) return null
    const [ok, longitude, latitude, altitude] = metadata.try_get_gps_info()
    if (!ok) return null
    const position = `${latitude.toFixed(5)}, ${longitude.toFixed(5)}`
    return metadata.try_has_tag("Exif.GPSInfo.GPSAltitude")
      ? `${position} (${Math.round(altitude)} m)`
      : position
  } catch {
    return null
  }
}
