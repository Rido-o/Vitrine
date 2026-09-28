import Gdk from "gi://Gdk?version=4.0"
import GLib from "gi://GLib"
import Gtk from "gi://Gtk?version=4.0"
import GdkPixbuf from "gi://GdkPixbuf"
import GObject from "gi://GObject"
import Graphene from "gi://Graphene"
import Gsk from "gi://Gsk"
import { copyToTexture } from "./decode"
import ImageCache, { Cancelled, type DecodedImage } from "./ImageCache"
import { requestGc } from "./util"

const ZOOM_STEP = 1.2
const MAX_ZOOM = 8
// Share of the width at each side that navigates when clicked (at fit).
const NAV_EDGE = 1 / 6
// Each animation frame is a new texture; collect after this many bytes of them.
const GC_AFTER_FRAME_BYTES = 64 * 1024 * 1024

const ZoomableImage = GObject.registerClass(
  class ZoomableImage extends Gtk.Widget {
    declare private texture: Gdk.Texture | null
    declare private scale: number
    declare private offsetX: number
    declare private offsetY: number
    declare private fitted: boolean
    declare private loadId: number
    declare private dragStartX: number
    declare private dragStartY: number
    declare private sharp: boolean
    declare private path: string | null
    declare private images: ImageCache
    declare private showingPlaceholder: boolean
    declare private pointerX: number
    declare private pointerY: number
    declare private cursorHidden: boolean
    declare private frames: GdkPixbuf.PixbufAnimationIter | null
    declare private tickId: number
    declare private frameBytes: number
    // Called with -1/1 when the left/right edge is clicked at fit-to-screen.
    declare onNavigate: (offset: number) => void

    constructor(params?: Partial<Gtk.Widget.ConstructorProps>) {
      super(params)
      this.texture = null
      this.scale = 1
      this.offsetX = 0
      this.offsetY = 0
      this.fitted = true
      this.loadId = 0
      this.dragStartX = 0
      this.dragStartY = 0
      this.sharp = false
      this.path = null
      this.images = new ImageCache()
      this.showingPlaceholder = false
      this.pointerX = 0
      this.pointerY = 0
      this.cursorHidden = false
      this.frames = null
      this.tickId = 0
      this.frameBytes = 0
      this.onNavigate = () => {}
      this.overflow = Gtk.Overflow.HIDDEN

      const motion = new Gtk.EventControllerMotion()
      motion.connect("motion", (_c, x, y) => {
        this.pointerX = x
        this.pointerY = y
        this.updateCursor()
      })
      this.add_controller(motion)

      const scroll = new Gtk.EventControllerScroll({
        flags: Gtk.EventControllerScrollFlags.VERTICAL,
      })
      scroll.connect("scroll", (_c, _dx, dy) => {
        this.zoomAt(ZOOM_STEP ** -dy, this.pointerX, this.pointerY)
        return true
      })
      this.add_controller(scroll)

      const drag = new Gtk.GestureDrag({ button: Gdk.BUTTON_PRIMARY })
      drag.connect("drag-begin", () => {
        this.dragStartX = this.offsetX
        this.dragStartY = this.offsetY
        if (!this.fitted) this.set_cursor_from_name("grabbing")
      })
      drag.connect("drag-update", (_g, dx, dy) => {
        if (this.fitted) return
        this.offsetX = this.dragStartX + dx
        this.offsetY = this.dragStartY + dy
        this.clampOffsets()
        this.queue_draw()
      })
      drag.connect("drag-end", () => this.updateCursor())
      this.add_controller(drag)

      // At fit-to-screen, a click in the left/right edge navigates straight
      // away (fast clicks skip several images), so double-click toggles zoom
      // only in the middle. Zoomed in, clicks there do nothing and
      // dragging pans.
      const click = new Gtk.GestureClick({ button: Gdk.BUTTON_PRIMARY })
      click.connect("pressed", (_g, nPress, x, y) => {
        const region = this.region(x)
        if (region === 0 && nPress === 2) this.toggleActualSize(x, y)
        else if (region === -1 || region === 1) this.onNavigate(region)
      })
      this.add_controller(click)
    }

    // Shows `path` (null clears the view and the cache); `neighbours` are
    // preloaded in order and everything else is dropped. `placeholder` (the
    // thumbnail) is shown until the full image is decoded.
    setFile(
      path: string | null,
      neighbours: string[] = [],
      placeholder: Gdk.Texture | null = null,
    ) {
      this.images.keep(path ? [path, ...neighbours] : [])
      if (path !== null && path === this.path) return
      this.path = path
      const id = ++this.loadId
      this.stopAnimation()
      this.showingPlaceholder = false
      if (!path) {
        this.texture = null
        this.queue_draw()
        return
      }
      if (placeholder) {
        this.texture = placeholder
        this.showingPlaceholder = true
        this.resetZoom()
      }
      this.images
        .get(path)
        .then((image) => {
          if (id === this.loadId) this.showImage(image)
        })
        .catch((error) => {
          if (error instanceof Cancelled) return
          console.error(`Could not load ${path}:`, error)
          if (id === this.loadId) {
            this.texture = null
            this.resetZoom()
          }
        })
    }

    // Replacing the placeholder keeps its on-screen size if zoomed.
    private showImage({ texture, animation }: DecodedImage) {
      if (animation) this.animate(animation)
      const previous = this.texture
      const keepZoom = this.showingPlaceholder && !this.fitted && previous
      this.texture = texture
      this.showingPlaceholder = false
      if (!keepZoom) return this.resetZoom()
      this.scale *= previous.get_width() / texture.get_width()
      this.clampOffsets()
      this.queue_draw()
    }

    // Plays from the first frame on the frame clock, so it only runs while the
    // view is on screen; stops for good at a finite GIF's last frame.
    private animate(animation: GdkPixbuf.PixbufAnimation) {
      this.frames = animation.get_iter(null)
      this.tickId = this.add_tick_callback(() => {
        const frames = this.frames
        if (!frames) return GLib.SOURCE_REMOVE
        if (frames.advance(null)) {
          const pixbuf = frames.get_pixbuf()
          this.texture = copyToTexture(pixbuf)
          this.queue_draw()
          this.frameBytes += pixbuf.get_byte_length()
          if (this.frameBytes >= GC_AFTER_FRAME_BYTES) {
            this.frameBytes = 0
            requestGc()
          }
        }
        if (frames.get_delay_time() >= 0) return GLib.SOURCE_CONTINUE
        this.tickId = 0
        this.frames = null
        return GLib.SOURCE_REMOVE
      })
    }

    private stopAnimation() {
      if (this.tickId) this.remove_tick_callback(this.tickId)
      this.tickId = 0
      this.frames = null
    }

    toggleSharp() {
      this.sharp = !this.sharp
      this.queue_draw()
    }

    zoomIn() {
      this.zoomAt(ZOOM_STEP, this.get_width() / 2, this.get_height() / 2)
    }

    zoomOut() {
      this.zoomAt(1 / ZOOM_STEP, this.get_width() / 2, this.get_height() / 2)
    }

    resetZoom() {
      this.fitted = true
      this.scale = this.fitScale()
      this.clampOffsets()
      this.updateCursor()
      this.queue_draw()
    }

    private fitScale() {
      if (!this.texture) return 1
      return Math.min(
        this.get_width() / this.texture.get_width(),
        this.get_height() / this.texture.get_height(),
      )
    }

    private actualScale() {
      const surfaceScale = this.get_native()?.get_surface()?.get_scale()
      return 1 / (surfaceScale ?? this.get_scale_factor())
    }

    private zoomAt(factor: number, x: number, y: number) {
      if (!this.texture) return
      const minScale = this.fitScale()
      const maxScale = Math.max(minScale, this.actualScale() * MAX_ZOOM)
      const scale = Math.min(Math.max(this.scale * factor, minScale), maxScale)
      if (scale === this.scale) return

      this.offsetX = x - ((x - this.offsetX) / this.scale) * scale
      this.offsetY = y - ((y - this.offsetY) / this.scale) * scale
      this.scale = scale
      this.fitted = scale <= minScale
      this.clampOffsets()
      this.updateCursor()
      this.queue_draw()
    }

    private toggleActualSize(x: number, y: number) {
      if (!this.fitted) {
        this.resetZoom()
        return
      }
      const fit = this.fitScale()
      const actual = this.actualScale()
      this.zoomAt((actual > fit ? actual : fit * 2) / this.scale, x, y)
    }

    private clampOffsets() {
      if (!this.texture) return
      const clamp = (offset: number, view: number, size: number) =>
        this.fitted
          ? (view - size) / 2
          : Math.min(Math.max(offset, view / 2 - size), view / 2)
      this.offsetX = clamp(
        this.offsetX,
        this.get_width(),
        this.texture.get_width() * this.scale,
      )
      this.offsetY = clamp(
        this.offsetY,
        this.get_height(),
        this.texture.get_height() * this.scale,
      )
    }

    // -1/1 for the left/right edge at fit-to-screen, otherwise 0 (so when
    // zoomed in, double-click works anywhere and single clicks do nothing).
    private region(x: number) {
      if (!this.fitted || !this.texture) return 0
      const width = this.get_width()
      if (x < width * NAV_EDGE) return -1
      if (x > width * (1 - NAV_EDGE)) return 1
      return 0
    }

    // Hides the cursor over the image (while controls auto-hide in fullscreen).
    setCursorHidden(hidden: boolean) {
      this.cursorHidden = hidden
      this.updateCursor()
    }

    private updateCursor() {
      if (this.cursorHidden) return this.set_cursor_from_name("none")
      const region = this.region(this.pointerX)
      this.set_cursor_from_name(
        !this.fitted
          ? "grab"
          : region === -1
            ? "w-resize"
            : region === 1
              ? "e-resize"
              : null,
      )
    }

    vfunc_measure(): [number, number, number, number] {
      return [0, 0, -1, -1]
    }

    vfunc_size_allocate(_width: number, _height: number, _baseline: number) {
      if (this.fitted) this.resetZoom()
      else this.clampOffsets()
    }

    vfunc_snapshot(snapshot: Gtk.Snapshot) {
      if (!this.texture) return
      const pixels = 1 / this.actualScale()
      const snap = (value: number) => Math.round(value * pixels) / pixels
      const left = snap(this.offsetX)
      const top = snap(this.offsetY)
      const bounds = new Graphene.Rect().init(
        left,
        top,
        snap(this.offsetX + this.texture.get_width() * this.scale) - left,
        snap(this.offsetY + this.texture.get_height() * this.scale) - top,
      )
      if (this.sharp) {
        snapshot.append_scaled_texture(
          this.texture,
          Gsk.ScalingFilter.NEAREST,
          bounds,
        )
      } else {
        snapshot.append_texture(this.texture, bounds)
      }
    }
  },
)

type ZoomableImage = InstanceType<typeof ZoomableImage>
export default ZoomableImage
