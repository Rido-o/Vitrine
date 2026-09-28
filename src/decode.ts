import Gdk from "gi://Gdk?version=4.0"
import GdkPixbuf from "gi://GdkPixbuf"
import Gio from "gi://Gio"
import GLib from "gi://GLib"

// Decodes an image in GdkPixbuf's worker thread, so large images don't freeze
// the window (Gdk.Texture.new_from_bytes decodes on the main thread: up to
// ~150 ms for a 12 MP WebP). With `size`, scales down to fit within it while
// decoding (smaller images keep their size). Cancelling `cancellable` stops
// the read and the decode. The EXIF orientation is applied, so phone photos
// aren't sideways.
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

// Decodes an animation (GIF) in GdkPixbuf's worker thread; see decodeImage.
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
