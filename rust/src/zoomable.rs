//! The full-screen view's image: fit, zoom around the cursor, pan, view-only
//! rotation and flips, sharp pixels, and clicks on the left/right edges to
//! move. As ZoomableImage.ts, plus resolution levels: a texture of about the
//! screen's size for the whole image, and full-resolution tiles for zooming
//! past it, of which only the visible ones are drawn (so uploaded), a few new
//! ones per frame.

use crate::decode::Tile;
use gtk::{gdk, glib, graphene, gsk, prelude::*, subclass::prelude::*};
use std::{cell::RefCell, collections::HashSet};

const ZOOM_STEP: f64 = 1.2;
const MAX_ZOOM: f64 = 8.0;
// Share of the width at each side that moves to the previous/next image when
// clicked (at fit).
const NAV_EDGE: f64 = 1.0 / 6.0;
// New tiles drawn per frame (each drawn for the first time is uploaded: a
// 512×512 tile is ~1 MB); the rest show the lower resolution until later.
const NEW_TILES_PER_FRAME: usize = 3;

/// Full-resolution tiles of the shown image.
pub struct Tiles {
    pub tiles: Vec<(Tile, gdk::Texture)>,
    pub width: u32,
    pub height: u32,
}

#[derive(Default)]
pub struct State {
    // The whole image at some resolution, and the image's own size.
    base: Option<gdk::Texture>,
    full_w: f64,
    full_h: f64,
    tiles: Option<Tiles>,
    // Tiles drawn (so uploaded) since they arrived.
    drawn: HashSet<usize>,
    wants_detail: bool,
    // Logical pixels per image pixel, and where the image's top-left is.
    scale: f64,
    offset_x: f64,
    offset_y: f64,
    fitted: bool,
    drag_start: (f64, f64),
    sharp: bool,
    // View-only: flipped horizontally (in the image's own axes), then
    // rotated clockwise.
    rotation: i32,
    flipped: bool,
    pointer: (f64, f64),
    cursor_hidden: bool,
}

type NavigateCallback = Box<dyn Fn(i32)>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ZoomableImage {
        pub state: RefCell<State>,
        // Called with -1/1 when an edge is clicked at fit.
        pub on_navigate: RefCell<Option<NavigateCallback>>,
        // Called when zooming needs more pixels than the texture has.
        pub on_detail: RefCell<Option<Box<dyn Fn()>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ZoomableImage {
        const NAME: &'static str = "VitrineZoomableImage";
        type Type = super::ZoomableImage;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ZoomableImage {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_overflow(gtk::Overflow::Hidden);
            obj.set_hexpand(true);
            obj.set_vexpand(true);
            self.state.borrow_mut().fitted = true;
            self.state.borrow_mut().scale = 1.0;

            let motion = gtk::EventControllerMotion::new();
            let weak = obj.downgrade();
            motion.connect_motion(move |_, x, y| {
                if let Some(obj) = weak.upgrade() {
                    obj.imp().state.borrow_mut().pointer = (x, y);
                    obj.update_cursor();
                }
            });
            obj.add_controller(motion);

            let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
            let weak = obj.downgrade();
            scroll.connect_scroll(move |_, _, dy| {
                if let Some(obj) = weak.upgrade() {
                    let (x, y) = obj.imp().state.borrow().pointer;
                    obj.zoom_at(ZOOM_STEP.powf(-dy), x, y);
                }
                glib::Propagation::Stop
            });
            obj.add_controller(scroll);

            let drag = gtk::GestureDrag::builder()
                .button(gdk::BUTTON_PRIMARY)
                .build();
            let weak = obj.downgrade();
            drag.connect_drag_begin(move |_, _, _| {
                let Some(obj) = weak.upgrade() else { return };
                let fitted = {
                    let mut state = obj.imp().state.borrow_mut();
                    state.drag_start = (state.offset_x, state.offset_y);
                    state.fitted
                };
                if !fitted {
                    obj.set_cursor_from_name(Some("grabbing"));
                }
            });
            let weak = obj.downgrade();
            drag.connect_drag_update(move |_, dx, dy| {
                let Some(obj) = weak.upgrade() else { return };
                let mut state = obj.imp().state.borrow_mut();
                if state.fitted {
                    return;
                }
                state.offset_x = state.drag_start.0 + dx;
                state.offset_y = state.drag_start.1 + dy;
                drop(state);
                obj.clamp_offsets();
                obj.queue_draw();
            });
            let weak = obj.downgrade();
            drag.connect_drag_end(move |_, _, _| {
                if let Some(obj) = weak.upgrade() {
                    obj.update_cursor();
                }
            });
            obj.add_controller(drag);

            // At fit, a click in the left/right edge moves straight away (fast
            // clicks skip several images), so double-click toggles zoom only
            // in the middle. Zoomed in, clicks do nothing and dragging pans.
            let click = gtk::GestureClick::builder()
                .button(gdk::BUTTON_PRIMARY)
                .build();
            let weak = obj.downgrade();
            click.connect_pressed(move |_, presses, x, y| {
                let Some(obj) = weak.upgrade() else { return };
                match obj.region(x) {
                    0 if presses == 2 => obj.toggle_actual_size(x, y),
                    0 => {}
                    side => {
                        if let Some(navigate) = obj.imp().on_navigate.borrow().as_ref() {
                            navigate(side);
                        }
                    }
                }
            });
            obj.add_controller(click);
        }
    }

    impl WidgetImpl for ZoomableImage {
        fn measure(&self, _: gtk::Orientation, _: i32) -> (i32, i32, i32, i32) {
            (0, 0, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            self.parent_size_allocate(width, height, baseline);
            let obj = self.obj();
            if self.state.borrow().fitted {
                obj.reset_zoom();
            } else {
                obj.clamp_offsets();
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            self.obj().draw(snapshot);
        }
    }
}

glib::wrapper! {
    pub struct ZoomableImage(ObjectSubclass<imp::ZoomableImage>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for ZoomableImage {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ZoomableImage {
    pub fn connect_navigate(&self, navigate: impl Fn(i32) + 'static) {
        *self.imp().on_navigate.borrow_mut() = Some(Box::new(navigate));
    }

    pub fn connect_detail(&self, detail: impl Fn() + 'static) {
        *self.imp().on_detail.borrow_mut() = Some(Box::new(detail));
    }

    /// Shows `texture` for an image of `full` (its own size, as shown). A new
    /// image starts at fit, unrotated; the same image at another resolution
    /// (the thumbnail, then the decoded one) keeps its place and on-screen
    /// size.
    pub fn set_image(&self, texture: &gdk::Texture, full: (f64, f64), new_image: bool) {
        let mut state = self.imp().state.borrow_mut();
        if new_image {
            state.rotation = 0;
            state.flipped = false;
            state.tiles = None;
            state.drawn.clear();
            state.wants_detail = false;
            state.fitted = true;
        } else if !state.fitted && state.full_w > 0.0 {
            // Same on-screen size: the scale is per image pixel.
            state.scale *= state.full_w / full.0;
        }
        state.base = Some(texture.clone());
        state.full_w = full.0;
        state.full_h = full.1;
        let fitted = state.fitted;
        drop(state);
        if fitted {
            self.reset_zoom();
        } else {
            self.clamp_offsets();
            self.check_detail();
            self.queue_draw();
        }
    }

    /// Replaces the texture only (an animation's next frame).
    pub fn set_frame(&self, texture: &gdk::Texture) {
        self.imp().state.borrow_mut().base = Some(texture.clone());
        self.queue_draw();
    }

    /// Full-resolution tiles for the image shown.
    pub fn set_tiles(&self, tiles: Tiles) {
        let mut state = self.imp().state.borrow_mut();
        state.drawn.clear();
        state.tiles = Some(tiles);
        drop(state);
        self.queue_draw();
    }

    pub fn clear(&self) {
        let mut state = self.imp().state.borrow_mut();
        state.base = None;
        state.tiles = None;
        state.drawn.clear();
        drop(state);
        self.queue_draw();
    }

    pub fn has_image(&self) -> bool {
        self.imp().state.borrow().base.is_some()
    }

    pub fn zoom_in(&self) {
        let (w, h) = (self.width() as f64, self.height() as f64);
        self.zoom_at(ZOOM_STEP, w / 2.0, h / 2.0);
    }

    pub fn zoom_out(&self) {
        let (w, h) = (self.width() as f64, self.height() as f64);
        self.zoom_at(1.0 / ZOOM_STEP, w / 2.0, h / 2.0);
    }

    pub fn reset_zoom(&self) {
        let fit = self.fit_scale();
        let mut state = self.imp().state.borrow_mut();
        state.fitted = true;
        state.scale = fit;
        drop(state);
        self.clamp_offsets();
        self.update_cursor();
        self.queue_draw();
    }

    pub fn toggle_sharp(&self) {
        let mut state = self.imp().state.borrow_mut();
        state.sharp = !state.sharp;
        drop(state);
        self.queue_draw();
    }

    // Rotation and flips are in screen terms and only change the view; they
    // reset for the next image. A flip on screen is a flip in the image's own
    // axes after undoing the rotation: H·R(θ) = R(−θ)·H, and V = R(180°)·H.
    pub fn rotate(&self, clockwise: bool) {
        let (rotation, flipped) = {
            let state = self.imp().state.borrow();
            (
                state.rotation + if clockwise { 90 } else { -90 },
                state.flipped,
            )
        };
        self.set_orientation(rotation, flipped);
    }

    pub fn flip(&self, horizontally: bool) {
        let (rotation, flipped) = {
            let state = self.imp().state.borrow();
            (
                if horizontally { 0 } else { 180 } - state.rotation,
                !state.flipped,
            )
        };
        self.set_orientation(rotation, flipped);
    }

    fn set_orientation(&self, rotation: i32, flipped: bool) {
        let mut state = self.imp().state.borrow_mut();
        if state.base.is_none() {
            return;
        }
        state.rotation = rotation.rem_euclid(360);
        state.flipped = flipped;
        drop(state);
        self.reset_zoom();
    }

    /// Hides the cursor over the image (while controls auto-hide).
    pub fn set_cursor_hidden(&self, hidden: bool) {
        self.imp().state.borrow_mut().cursor_hidden = hidden;
        self.update_cursor();
    }

    // For the probe: the zoom (logical pixels per image pixel), 1:1 in
    // device pixels, and whether every visible tile is drawn at full
    // resolution (true when none are needed).
    pub fn scale(&self) -> f64 {
        self.imp().state.borrow().scale
    }

    pub fn actual_scale(&self) -> f64 {
        1.0 / self.device_scale()
    }

    pub fn is_detailed(&self) -> bool {
        let state = self.imp().state.borrow();
        !self.needs_detail(&state)
            || state.tiles.is_some()
                && self
                    .visible_tiles(&state)
                    .iter()
                    .all(|i| state.drawn.contains(i))
    }

    pub fn pan_by(&self, dx: f64, dy: f64) {
        let mut state = self.imp().state.borrow_mut();
        if state.fitted {
            return;
        }
        state.offset_x += dx;
        state.offset_y += dy;
        drop(state);
        self.clamp_offsets();
        self.queue_draw();
    }

    fn device_scale(&self) -> f64 {
        self.native()
            .and_then(|native| native.surface())
            .map_or(self.scale_factor() as f64, |surface| surface.scale())
    }

    // The size shown on screen before scaling: rotated by 90° swaps it.
    fn shown_size(state: &State) -> (f64, f64) {
        if state.rotation % 180 != 0 {
            (state.full_h, state.full_w)
        } else {
            (state.full_w, state.full_h)
        }
    }

    fn fit_scale(&self) -> f64 {
        let state = self.imp().state.borrow();
        if state.base.is_none() || state.full_w <= 0.0 {
            return 1.0;
        }
        let (w, h) = Self::shown_size(&state);
        (self.width() as f64 / w).min(self.height() as f64 / h)
    }

    pub fn zoom_at(&self, factor: f64, x: f64, y: f64) {
        if !self.has_image() {
            return;
        }
        let min = self.fit_scale();
        let max = min.max(self.actual_scale() * MAX_ZOOM);
        let mut state = self.imp().state.borrow_mut();
        let scale = (state.scale * factor).clamp(min, max);
        if scale == state.scale {
            return;
        }
        state.offset_x = x - (x - state.offset_x) / state.scale * scale;
        state.offset_y = y - (y - state.offset_y) / state.scale * scale;
        state.scale = scale;
        state.fitted = scale <= min;
        drop(state);
        self.clamp_offsets();
        self.update_cursor();
        self.check_detail();
        self.queue_draw();
    }

    fn toggle_actual_size(&self, x: f64, y: f64) {
        if !self.imp().state.borrow().fitted {
            self.reset_zoom();
            return;
        }
        let fit = self.fit_scale();
        let actual = self.actual_scale();
        let target = if actual > fit { actual } else { fit * 2.0 };
        let scale = self.imp().state.borrow().scale;
        self.zoom_at(target / scale, x, y);
    }

    fn clamp_offsets(&self) {
        let (view_w, view_h) = (self.width() as f64, self.height() as f64);
        let mut state = self.imp().state.borrow_mut();
        if state.base.is_none() {
            return;
        }
        let (w, h) = Self::shown_size(&state);
        let (size_w, size_h) = (w * state.scale, h * state.scale);
        let fitted = state.fitted;
        let clamp = |offset: f64, view: f64, size: f64| {
            if fitted {
                (view - size) / 2.0
            } else {
                offset.clamp(view / 2.0 - size, view / 2.0)
            }
        };
        state.offset_x = clamp(state.offset_x, view_w, size_w);
        state.offset_y = clamp(state.offset_y, view_h, size_h);
    }

    // -1/1 for the left/right edge at fit, otherwise 0 (so when zoomed in,
    // double-click works anywhere and single clicks do nothing).
    fn region(&self, x: f64) -> i32 {
        let state = self.imp().state.borrow();
        if !state.fitted || state.base.is_none() {
            return 0;
        }
        let width = self.width() as f64;
        if x < width * NAV_EDGE {
            -1
        } else if x > width * (1.0 - NAV_EDGE) {
            1
        } else {
            0
        }
    }

    fn update_cursor(&self) {
        let (hidden, fitted, x) = {
            let state = self.imp().state.borrow();
            (state.cursor_hidden, state.fitted, state.pointer.0)
        };
        let name = if hidden {
            Some("none")
        } else if !fitted {
            Some("grab")
        } else {
            match self.region(x) {
                -1 => Some("w-resize"),
                1 => Some("e-resize"),
                _ => None,
            }
        };
        self.set_cursor_from_name(name);
    }

    // Whether the zoom shows more image pixels per device pixel than the
    // texture has.
    fn needs_detail(&self, state: &State) -> bool {
        let Some(base) = &state.base else {
            return false;
        };
        let device_per_image = state.scale * self.device_scale();
        let base_per_image = base.width() as f64 / state.full_w.max(1.0);
        state.full_w > base.width() as f64 && device_per_image > base_per_image * 1.02
    }

    fn check_detail(&self) {
        let wants = {
            let mut state = self.imp().state.borrow_mut();
            let wants = state.tiles.is_none() && !state.wants_detail && self.needs_detail(&state);
            if wants {
                state.wants_detail = true;
            }
            wants
        };
        if wants && let Some(detail) = self.imp().on_detail.borrow().as_ref() {
            detail();
        }
    }

    // The image's rectangle on screen (snapped to device pixels), and the
    // size it's drawn at in the image's own axes.
    fn layout(&self, state: &State) -> (graphene::Rect, f64, f64) {
        let device = self.device_scale();
        let snap = |v: f64| (v * device).round() / device;
        let (w, h) = Self::shown_size(state);
        let left = snap(state.offset_x);
        let top = snap(state.offset_y);
        let right = snap(state.offset_x + w * state.scale);
        let bottom = snap(state.offset_y + h * state.scale);
        let (bw, bh) = (right - left, bottom - top);
        let (dw, dh) = if state.rotation % 180 != 0 {
            (bh, bw)
        } else {
            (bw, bh)
        };
        (
            graphene::Rect::new(left as f32, top as f32, bw as f32, bh as f32),
            dw,
            dh,
        )
    }

    // A point on screen in the image's own pixels.
    fn to_image(
        state: &State,
        bounds: &graphene::Rect,
        dw: f64,
        dh: f64,
        x: f64,
        y: f64,
    ) -> (f64, f64) {
        let cx = (bounds.x() + bounds.width() / 2.0) as f64;
        let cy = (bounds.y() + bounds.height() / 2.0) as f64;
        let (mut px, mut py) = (x - cx, y - cy);
        // Undo the clockwise rotation, then the flip.
        (px, py) = match state.rotation {
            90 => (py, -px),
            180 => (-px, -py),
            270 => (-py, px),
            _ => (px, py),
        };
        if state.flipped {
            px = -px;
        }
        (
            (px + dw / 2.0) * state.full_w / dw,
            (py + dh / 2.0) * state.full_h / dh,
        )
    }

    // Indices of the tiles in view.
    fn visible_tiles(&self, state: &State) -> Vec<usize> {
        let Some(tiles) = &state.tiles else {
            return Vec::new();
        };
        let (bounds, dw, dh) = self.layout(state);
        let (vw, vh) = (self.width() as f64, self.height() as f64);
        let corners = [(0.0, 0.0), (vw, 0.0), (0.0, vh), (vw, vh)]
            .map(|(x, y)| Self::to_image(state, &bounds, dw, dh, x, y));
        let min_x = corners
            .iter()
            .map(|c| c.0)
            .fold(f64::MAX, f64::min)
            .max(0.0);
        let max_x = corners.iter().map(|c| c.0).fold(f64::MIN, f64::max);
        let min_y = corners
            .iter()
            .map(|c| c.1)
            .fold(f64::MAX, f64::min)
            .max(0.0);
        let max_y = corners.iter().map(|c| c.1).fold(f64::MIN, f64::max);
        tiles
            .tiles
            .iter()
            .enumerate()
            .filter(|(_, (tile, _))| {
                let (x, y) = (tile.x as f64, tile.y as f64);
                x < max_x
                    && x + tile.width as f64 > min_x
                    && y < max_y
                    && y + tile.height as f64 > min_y
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn draw(&self, snapshot: &gtk::Snapshot) {
        let visible = {
            let state = self.imp().state.borrow();
            if state.base.is_none() {
                return;
            }
            if self.needs_detail(&state) {
                self.visible_tiles(&state)
            } else {
                Vec::new()
            }
        };
        let mut state = self.imp().state.borrow_mut();
        let (bounds, dw, dh) = self.layout(&state);
        let filter = if state.sharp {
            gsk::ScalingFilter::Nearest
        } else {
            gsk::ScalingFilter::Linear
        };
        snapshot.save();
        snapshot.translate(&graphene::Point::new(
            bounds.x() + bounds.width() / 2.0,
            bounds.y() + bounds.height() / 2.0,
        ));
        snapshot.rotate(state.rotation as f32);
        if state.flipped {
            snapshot.scale(-1.0, 1.0);
        }
        let (x0, y0) = (-dw / 2.0, -dh / 2.0);
        let base = state.base.clone().expect("checked above");
        snapshot.append_scaled_texture(
            &base,
            filter,
            &graphene::Rect::new(x0 as f32, y0 as f32, dw as f32, dh as f32),
        );
        // Tiles over it, padded ones overlapping so there are no seams; the
        // first few not drawn before this frame, the rest later.
        let (sx, sy) = (dw / state.full_w, dh / state.full_h);
        let mut budget = NEW_TILES_PER_FRAME;
        let mut deferred = false;
        let State { tiles, drawn, .. } = &mut *state;
        if let Some(tiles) = tiles {
            for i in visible {
                if !drawn.contains(&i) {
                    if budget == 0 {
                        deferred = true;
                        continue;
                    }
                    budget -= 1;
                    drawn.insert(i);
                }
                let (tile, texture) = &tiles.tiles[i];
                let left = tile.x as f64 - tile.pad_left as f64;
                let top = tile.y as f64 - tile.pad_top as f64;
                snapshot.append_scaled_texture(
                    texture,
                    filter,
                    &graphene::Rect::new(
                        (x0 + left * sx) as f32,
                        (y0 + top * sy) as f32,
                        (texture.width() as f64 * sx) as f32,
                        (texture.height() as f64 * sy) as f32,
                    ),
                );
            }
        }
        snapshot.restore();
        drop(state);
        if deferred {
            // Next frame, the next few.
            self.add_tick_callback(|widget, _| {
                widget.queue_draw();
                glib::ControlFlow::Break
            });
        }
    }
}
