import GdkPixbuf from "gi://GdkPixbuf"
import Gio from "gi://Gio"
import GLib from "gi://GLib"

// Decodes an image in GdkPixbuf's worker thread, so large images don't freeze
// the window (Gdk.Texture.new_from_bytes decodes on the main thread: up to
// ~150 ms for a 12 MP WebP). With `size`, scales to fit within it while
// decoding. Cancelling `cancellable` stops the read and the decode. The EXIF
// orientation is applied, so phone photos aren't sideways.
export function decodeImage(
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
