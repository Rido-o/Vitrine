//! Opening images in the full-screen view, moving through them and going back
//! to the grid; the view's decodes follow the selection.

use super::Window;
use crate::{
    library::image,
    view::{preview::Preview, zoomable},
};
use gtk::prelude::*;
use std::{cell::RefCell, rc::Rc};

impl Window {
    pub(super) fn in_grid(&self) -> bool {
        self.stack.visible_child_name().as_deref() == Some("grid")
    }

    // Shows the image at `position` (wrapping around) in the view.
    pub(super) fn show_at(&self, position: i64) {
        let count = self.selection.n_items() as i64;
        if count == 0 {
            return;
        }
        let at = |offset: i64| {
            self.selection
                .item((position + offset).rem_euclid(count) as u32)
                .map(|object| image(&object).clone())
        };
        let Some(shown) = at(0) else { return };
        self.selection
            .set_selected(position.rem_euclid(count) as u32);
        // Nearest first, the way ←/→ would reach them.
        let neighbours = [1, -1, 2, -2]
            .into_iter()
            .filter_map(at)
            .map(|image| image.path)
            .collect();
        let (width, height) = view_size(&self.window, &self.preview.image);
        self.preview.show(
            shown.path.clone(),
            neighbours,
            self.tiles.cached(&shown),
            width,
            height,
        );
        self.stack.set_visible_child_name("preview");
    }

    pub(super) fn close_view(&self) {
        self.view.leave();
        self.stack.set_visible_child_name("grid");
        let selected = self.selection.selected();
        // Nothing selected when the last image was deleted.
        if selected < self.selection.n_items() {
            self.grid
                .scroll_to(selected, gtk::ListScrollFlags::FOCUS, None);
        } else {
            self.grid.grab_focus();
        }
        hide_after_paint(&self.window, &self.stack, &self.selection, &self.preview);
    }

    // The view follows the selection after a delete or restore; with nothing
    // left, back to the grid.
    pub(super) fn refresh_view_if_open(&self) {
        if self.in_grid() {
            return;
        }
        if self.selection.n_items() == 0 {
            self.close_view();
        } else if self.preview.shown() != self.selected_path() {
            self.show_at(self.selection.selected() as i64);
        }
    }

    /// Opens `file` in the view before the folder is scanned; the scan
    /// selects it in the grid when it turns up.
    pub(super) fn show_file(&self, file: std::path::PathBuf) {
        let (width, height) = view_size(&self.window, &self.preview.image);
        self.preview.show(file, Vec::new(), None, width, height);
        self.stack.set_visible_child_name("preview");
    }

    // Opening (Enter, double-click, View), the close button, and clicks at
    // the view's edges.
    pub(super) fn connect_preview(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.grid.connect_activate(move |_, position| {
            if let Some(this) = weak.upgrade() {
                this.show_at(position as i64);
            }
        });

        // The image selected in the grid is decoded in the background, so
        // opening it shows it sharp straight away. (In the view, `show`
        // preloads the neighbours instead.)
        let weak = Rc::downgrade(self);
        self.selection
            .connect_selected_item_notify(move |selection| {
                let Some(this) = weak.upgrade() else { return };
                if !this.in_grid() {
                    return;
                }
                if let Some(object) = selection.selected_item() {
                    let (width, height) = view_size(&this.window, &this.preview.image);
                    this.preview
                        .preload(image(&object).path.clone(), width, height);
                }
            });

        let weak = Rc::downgrade(self);
        self.view.connect_close(move || {
            if let Some(this) = weak.upgrade() {
                this.close_view();
            }
        });

        // At fit, a click in the left/right edge moves to the previous/next.
        let weak = Rc::downgrade(self);
        self.preview.image.connect_navigate(move |side| {
            if let Some(this) = weak.upgrade() {
                this.show_at(this.selection.selected() as i64 + side as i64);
            }
        });
    }
}

// The size an image fits within in the view, in device pixels: the window's
// (the view fills it; fractional scales included), less the colour
// assessment border, so at fit an image is drawn at exactly its decoded
// pixels. Before the window is shown (the grid's first selection), its
// default size; the view decodes again when its size changes.
fn view_size(window: &gtk::ApplicationWindow, image: &zoomable::ZoomableImage) -> (u32, u32) {
    let (width, height) = if window.width() > 0 {
        (window.width(), window.height())
    } else {
        (window.default_width(), window.default_height())
    };
    let scale = window
        .surface()
        .map_or(window.scale_factor() as f64, |surface| surface.scale());
    zoomable::image_area(
        (width as f64 * scale).round().max(1.0) as u32,
        (height as f64 * scale).round().max(1.0) as u32,
        image.assessment(),
    )
}

// Dropping the view's textures (all but the selected image's) once the grid's
// first frame is drawn, so freeing them doesn't delay it.
fn hide_after_paint(
    window: &gtk::ApplicationWindow,
    stack: &gtk::Stack,
    selection: &gtk::SingleSelection,
    preview: &Rc<Preview>,
) {
    let hide = {
        let (window, stack, selection, preview) = (
            window.clone(),
            stack.clone(),
            selection.clone(),
            preview.clone(),
        );
        move || {
            if stack.visible_child_name().as_deref() != Some("grid") {
                return;
            }
            let keep = selection
                .selected_item()
                .map(|object| image(&object).path.clone());
            let (width, height) = view_size(&window, &preview.image);
            preview.hide(keep, width, height);
        }
    };
    let Some(clock) = window.frame_clock() else {
        hide();
        return;
    };
    let handler = Rc::new(RefCell::new(None));
    let handler_ = handler.clone();
    let id = clock.connect_after_paint(move |clock| {
        if let Some(id) = handler_.borrow_mut().take() {
            clock.disconnect(id);
        }
        hide();
    });
    *handler.borrow_mut() = Some(id);
}
