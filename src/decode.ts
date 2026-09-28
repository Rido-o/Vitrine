import Gdk from "gi://Gdk?version=4.0"
import GdkPixbuf from "gi://GdkPixbuf"
import Gio from "gi://Gio"
import GLib from "gi://GLib"
import Gly from "gi://Gly?version=2"
import GlyGtk4 from "gi://GlyGtk4?version=2"

// Decodes a full-size image with glycin, in a separate (sandboxed) process, so
// the window keeps drawing: GdkPixbuf's async API only reads in the background
// and decodes on the main thread (~240 ms of stalls for a 34 MP JPEG). The
// EXIF orientation is applied. Cancelling `cancellable` stops the decode.
export function decodeTexture(
  path: string,
  cancellable: Gio.Cancellable | null = null,
): Promise<Gdk.Texture> {
  const loader = Gly.Loader.new(Gio.File.new_for_path(path))
  // Formats GTK uploads as they are; given plain RGB (most JPEGs), it
  // converted to RGBX on the main thread (~200 ms for 34 MP) when first drawn.
  loader.set_accepted_memory_formats(
    Gly.MemoryFormatSelection.B8G8R8A8_PREMULTIPLIED |
      Gly.MemoryFormatSelection.R8G8B8A8_PREMULTIPLIED |
      Gly.MemoryFormatSelection.R16G16B16A16_PREMULTIPLIED |
      Gly.MemoryFormatSelection.R32G32B32A32_FLOAT_PREMULTIPLIED,
  )
  return new Promise((resolve, reject) => {
    loader.load_async(cancellable, (_loader, result) => {
      let image: Gly.Image
      try {
        image = loader.load_finish(result)
      } catch (error) {
        reject(error)
        return
      }
      image.next_frame_async(cancellable, (_image, res) => {
        try {
          resolve(GlyGtk4.frame_get_texture(image.next_frame_finish(res)))
        } catch (error) {
          reject(error)
        }
      })
    })
  })
}

// Decodes an image with GdkPixbuf, reading asynchronously but decoding on the
// main thread in chunks (Gdk.Texture.new_from_bytes decodes in one go: up to
// ~150 ms for a 12 MP WebP). Used for thumbnails, where scaling while decoding
// keeps the chunks short, and when glycin fails. With `size`, scales down to
// fit within it while decoding (smaller images keep their size). Cancelling
// `cancellable` stops the read and the decode. The EXIF orientation is
// applied, so phone photos aren't sideways.
export async function decodeImage(
  path: string,
  size?: { width: number; height: number },
  cancellable: Gio.Cancellable | null = null,
): Promise<GdkPixbuf.Pixbuf> {
  if (size) {
    const [width, height] = await imageSize(path, cancellable)
    if (width <= size.width && height <= size.height) size = undefined
  }
  return decode(path, size, cancellable)
}

// The stored size (before EXIF rotation), from the file's header.
function imageSize(path: string, cancellable: Gio.Cancellable | null) {
  return new Promise<[number, number]>((resolve, reject) =>
    GdkPixbuf.Pixbuf.get_file_info_async(
      path,
      cancellable,
      (_source, result) => {
        try {
          const [format, width, height] =
            GdkPixbuf.Pixbuf.get_file_info_finish(result)
          if (!format) throw new Error("Unrecognized image format")
          resolve([width, height])
        } catch (error) {
          reject(error)
        }
      },
    ),
  )
}

function decode(
  path: string,
  size?: { width: number; height: number },
  cancellable: Gio.Cancellable | null = null,
): Promise<GdkPixbuf.Pixbuf> {
  return new Promise((resolve, reject) => {
    Gio.File.new_for_path(path).read_async(
      GLib.PRIORITY_LOW,
      cancellable,
      (source, result) => {
        let stream: Gio.InputStream
        try {
          stream = (source as Gio.File).read_finish(result)
        } catch (error) {
          reject(error)
          return
        }
        const onDone = (_source: unknown, res: Gio.AsyncResult) => {
          try {
            const pixbuf = GdkPixbuf.Pixbuf.new_from_stream_finish(res)
            stream.close(null)
            if (!pixbuf) throw new Error("Could not decode image")
            resolve(pixbuf.apply_embedded_orientation() ?? pixbuf)
          } catch (error) {
            stream.close(null)
            reject(error)
          }
        }
        if (size) {
          GdkPixbuf.Pixbuf.new_from_stream_at_scale_async(
            stream,
            size.width,
            size.height,
            true,
            cancellable,
            onDone,
          )
        } else {
          GdkPixbuf.Pixbuf.new_from_stream_async(stream, cancellable, onDone)
        }
      },
    )
  })
}

// Decodes an animation (GIF) with GdkPixbuf, on the main thread in chunks;
// see decodeImage.
export function decodeAnimation(
  path: string,
  cancellable: Gio.Cancellable | null = null,
): Promise<GdkPixbuf.PixbufAnimation> {
  return new Promise((resolve, reject) => {
    Gio.File.new_for_path(path).read_async(
      GLib.PRIORITY_LOW,
      cancellable,
      (source, result) => {
        let stream: Gio.InputStream
        try {
          stream = (source as Gio.File).read_finish(result)
        } catch (error) {
          reject(error)
          return
        }
        GdkPixbuf.PixbufAnimation.new_from_stream_async(
          stream,
          cancellable,
          (_source, res) => {
            try {
              const animation =
                GdkPixbuf.PixbufAnimation.new_from_stream_finish(res)
              stream.close(null)
              if (!animation) throw new Error("Could not decode image")
              resolve(animation)
            } catch (error) {
              stream.close(null)
              reject(error)
            }
          },
        )
      },
    )
  })
}

// A texture holding a copy of `pixbuf`'s pixels, for pixbufs that change
// afterwards (an animation iterator draws every frame into the same one).
export function copyToTexture(pixbuf: GdkPixbuf.Pixbuf) {
  return Gdk.MemoryTexture.new(
    pixbuf.get_width(),
    pixbuf.get_height(),
    pixbuf.get_has_alpha()
      ? Gdk.MemoryFormat.R8G8B8A8
      : Gdk.MemoryFormat.R8G8B8,
    pixbuf.read_pixel_bytes(),
    pixbuf.get_rowstride(),
  )
}
