//! The thumbnail grid: equal 16:9 cells whose thumbnails take their images'
//! own shapes, and keeping the first image selected while a folder loads.

use super::Window;
use crate::{
    library::image,
    thumbnails::tiles::{TILE_HEIGHT, TILE_WIDTH, Tiles},
};
use gtk::{gdk, glib, prelude::*};
use std::rc::Rc;

pub fn build(selection: &gtk::SingleSelection, tiles: &Rc<Tiles>) -> gtk::GridView {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        let picture = gtk::Picture::builder()
            .css_classes(["viewer-thumbnail", "empty"])
            .content_fit(gtk::ContentFit::Fill)
            .can_shrink(true)
            .overflow(gtk::Overflow::Hidden)
            .build();
        // Keeps every cell the same size, whatever the image's shape; the
        // picture takes the image's (`Tiles::show`).
        let frame = gtk::AspectFrame::builder()
            .ratio(TILE_WIDTH as f32 / TILE_HEIGHT as f32)
            .obey_child(false)
            .width_request(TILE_WIDTH)
            .child(&picture)
            .build();
        let cell = gtk::Box::builder()
            .layout_manager(&CellLayout::default())
            .build();
        cell.append(&frame);
        let item = item.downcast_ref::<gtk::ListItem>().expect("a ListItem");
        item.set_child(Some(&cell));

        // A right click opens the context menu on this cell's image
        // ("win.context-menu", in `menu.rs`), at the pointer.
        let click = gtk::GestureClick::builder()
            .button(gdk::BUTTON_SECONDARY)
            .build();
        let item = item.downgrade();
        click.connect_pressed(move |click, _, x, y| {
            let Some(cell) = click.widget() else { return };
            let (Some(item), Some(grid)) =
                (item.upgrade(), cell.ancestor(gtk::GridView::static_type()))
            else {
                return;
            };
            let Some(point) =
                cell.compute_point(&grid, &gtk::graphene::Point::new(x as f32, y as f32))
            else {
                return;
            };
            click.set_state(gtk::EventSequenceState::Claimed);
            let at = (item.position(), point.x() as f64, point.y() as f64);
            cell.activate_action("win.context-menu", Some(&at.to_variant()))
                .ok();
        });
        cell.add_controller(click);
    });
    let bind_tiles = tiles.clone();
    factory.connect_bind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a ListItem");
        let object = item.item().expect("a bound item");
        bind_tiles.bind(&tile_picture(item), &image(&object));
    });
    let unbind_tiles = tiles.clone();
    factory.connect_unbind(move |_, item| {
        let item = item.downcast_ref::<gtk::ListItem>().expect("a ListItem");
        unbind_tiles.unbind(&tile_picture(item));
    });
    gtk::GridView::builder()
        .css_classes(["viewer-grid"])
        .model(selection)
        .factory(&factory)
        .min_columns(1)
        .max_columns(12)
        .build()
}

/// The grid, scrolling, with `empty` (the empty folder's message) over it.
pub fn area(grid: &gtk::GridView, empty: &gtk::Label) -> gtk::Overlay {
    let scrolled = gtk::ScrolledWindow::builder()
        .hexpand(true)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(grid)
        .build();
    let overlay = gtk::Overlay::builder()
        .hexpand(true)
        .vexpand(true)
        .child(&scrolled)
        .build();
    overlay.add_overlay(empty);
    overlay
}

fn tile_picture(item: &gtk::ListItem) -> gtk::Picture {
    item.child()
        .and_then(|cell| cell.first_child())
        .and_downcast::<gtk::AspectFrame>()
        .and_then(|frame| frame.child())
        .and_downcast::<gtk::Picture>()
        .expect("a tile's Picture")
}

impl Window {
    // While a folder loads, keeps the first image selected and the grid at
    // the top, until the user clicks, types or scrolls in it. Batches arrive
    // in any order and are sorted as they come; the automatic selection
    // would otherwise stay on whichever image arrived first, wherever sorting
    // put it, and the grid kept it in view (opening halfway down a big
    // folder). An image to select (`pending`) is selected once the load
    // finishes (`finished`).
    pub(super) fn keep_first_while_loading(self: &Rc<Self>) {
        // Any press, key or scroll in the grid (seen, not consumed).
        let events = gtk::EventControllerLegacy::new();
        events.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        events.connect_event(move |_, event| {
            use gdk::EventType::*;
            if let Some(this) = weak.upgrade()
                && matches!(
                    event.event_type(),
                    ButtonPress | KeyPress | Scroll | TouchBegin
                )
            {
                this.touched.set(true);
            }
            glib::Propagation::Proceed
        });
        self.grid.add_controller(events);
        let weak = Rc::downgrade(self);
        self.selection
            .connect_items_changed(move |selection, _, _, _| {
                let Some(this) = weak.upgrade() else { return };
                if !this.folder.is_loading() || this.touched.get() || selection.n_items() == 0 {
                    return;
                }
                if selection.selected() != 0 {
                    selection.set_selected(0);
                }
                this.grid.scroll_to(0, gtk::ListScrollFlags::NONE, None);
            });
    }
}

mod imp {
    use super::*;
    use gtk::subclass::prelude::*;

    #[derive(Default)]
    pub struct CellLayout;

    #[glib::object_subclass]
    impl ObjectSubclass for CellLayout {
        const NAME: &'static str = "VitrineCellLayout";
        type Type = super::CellLayout;
        type ParentType = gtk::LayoutManager;
    }

    impl ObjectImpl for CellLayout {}

    impl LayoutManagerImpl for CellLayout {
        fn request_mode(&self, _widget: &gtk::Widget) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(
            &self,
            widget: &gtk::Widget,
            orientation: gtk::Orientation,
            for_size: i32,
        ) -> (i32, i32, i32, i32) {
            let Some(child) = widget.first_child() else {
                return (0, 0, -1, -1);
            };
            if orientation == gtk::Orientation::Horizontal {
                let (min, nat, _, _) = child.measure(orientation, -1);
                return (min, nat, -1, -1);
            }
            let width = if for_size < 0 { TILE_WIDTH } else { for_size };
            let height = width * TILE_HEIGHT / TILE_WIDTH;
            (height, height, -1, -1)
        }

        fn allocate(&self, widget: &gtk::Widget, width: i32, height: i32, baseline: i32) {
            if let Some(child) = widget.first_child() {
                child.allocate(width, height, baseline, None);
            }
        }
    }
}

glib::wrapper! {
    /// A grid cell's layout: its height follows the width the grid gives it,
    /// at the tile's ratio. Cells widen until another column fits (TILE_WIDTH
    /// at the least) and grow taller with them; the grid makes each row as
    /// tall as its cells' minimum height.
    pub struct CellLayout(ObjectSubclass<imp::CellLayout>)
        @extends gtk::LayoutManager;
}

impl Default for CellLayout {
    fn default() -> Self {
        glib::Object::new()
    }
}
