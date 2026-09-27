import Gdk from "gi://Gdk?version=4.0"
import Gtk from "gi://Gtk?version=4.0"
import GObject from "gi://GObject"
import Gio from "gi://Gio"
import Graphene from "gi://Graphene"
import Gsk from "gi://Gsk"

const ZOOM_STEP = 1.2
const MAX_ZOOM = 8

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
      this.overflow = Gtk.Overflow.HIDDEN

      let pointerX = 0
      let pointerY = 0
      const motion = new Gtk.EventControllerMotion()
      motion.connect("motion", (_c, x, y) => {
        pointerX = x
        pointerY = y
      })
      this.add_controller(motion)

      const scroll = new Gtk.EventControllerScroll({
        flags: Gtk.EventControllerScrollFlags.VERTICAL,
      })
      scroll.connect("scroll", (_c, _dx, dy) => {
        this.zoomAt(ZOOM_STEP ** -dy, pointerX, pointerY)
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

      const click = new Gtk.GestureClick({ button: Gdk.BUTTON_PRIMARY })
      click.connect("pressed", (_g, nPress, x, y) => {
        if (nPress === 2) this.toggleActualSize(x, y)
      })
      this.add_controller(click)
    }

    setFile(path: string | null) {
      const id = ++this.loadId
      if (!path) {
        this.texture = null
        this.queue_draw()
        return
      }
      Gio.File.new_for_path(path).load_bytes_async(null, (file, result) => {
        if (id !== this.loadId) return
        try {
          const [bytes] = file!.load_bytes_finish(result)
          this.texture = Gdk.Texture.new_from_bytes(bytes)
        } catch (error) {
          console.error(`Could not load ${path}:`, error)
          this.texture = null
        }
        this.resetZoom()
      })
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

    private updateCursor() {
      this.set_cursor_from_name(this.fitted ? null : "grab")
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
