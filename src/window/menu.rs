//! The ⋯ menu (in the toolbar and in the view), the context menus (a
//! thumbnail's, and the view's image's), their actions, and the i buttons'
//! properties popovers.

use super::Window;
use crate::{
    actions::{self, Actions, MenuAction},
    icon,
};
use gtk::{gdk, gio, glib, prelude::*};
use std::rc::Rc;

/// An i button: the selected image's properties.
pub fn properties_button(selection: &gtk::SingleSelection, pixel_size: i32) -> gtk::MenuButton {
    let selection = selection.clone();
    let popover = crate::properties::popover(move || super::selected_path(&selection));
    let button = gtk::MenuButton::builder()
        .tooltip_text("Image properties (i)")
        .popover(&popover)
        .build();
    button.set_child(Some(&icon("circle-info-awesome-symbolic", pixel_size)));
    button
}

impl Window {
    // The view's own actions (rotate, flip, colour assessment) are only
    // enabled there.
    pub(super) fn build_menu(self: &Rc<Self>) {
        let action = |name, label, run: fn(&Rc<Self>)| {
            let weak = Rc::downgrade(self);
            MenuAction::new(name, label, move || {
                if let Some(this) = weak.upgrade() {
                    run(&this);
                }
            })
        };
        let mut all = vec![
            action("open", "Open", |this| {
                this.show_at(this.selection.selected() as i64)
            })
            .accel("Return"),
            action("choose-folder", "Open folder…", Self::choose_folder).accel("<Control>o"),
            action("choose-image", "Open image…", Self::choose_image).accel("<Control><Shift>o"),
            action("copy-image", "Copy image", Self::copy_image).accel("<Control>c"),
            action("copy-path", "Copy path", Self::copy_path).accel("<Control><Shift>c"),
            action("rotate-left", "Rotate left", |this| {
                this.preview.image.rotate(false)
            })
            .accel("bracketleft")
            .view_only(),
            action("rotate-right", "Rotate right", |this| {
                this.preview.image.rotate(true)
            })
            .accel("bracketright")
            .view_only(),
            action("flip-horizontally", "Flip horizontally", |this| {
                this.preview.image.flip(true)
            })
            .accel("h")
            .view_only(),
            action("flip-vertically", "Flip vertically", |this| {
                this.preview.image.flip(false)
            })
            .accel("v")
            .view_only(),
            action("color-assessment", "Colour assessment", |this| {
                this.view.toggle_assessment()
            })
            .accel("b")
            .view_only()
            .checked({
                let image = self.preview.image.clone();
                move || image.assessment()
            }),
            action("show-in-file-manager", "Show in file manager", |this| {
                if let Some(path) = this.selected_path() {
                    crate::desktop::show_in_file_manager(&path);
                }
            }),
            action("properties", "Properties", |this| this.toggle_properties()).accel("i"),
            action("rescan", "Rescan folder", |this| this.folder.rescan()).accel("r"),
            action("delete", "Move to trash", Self::delete_selected).accel("Delete"),
            action("shortcuts", "Keyboard shortcuts", |this| {
                crate::shortcuts::show(&this.window, this.wallpaper_argv.is_some())
            })
            .accel("question"),
        ];
        if self.wallpaper_argv.is_some() {
            all.push(action("set-wallpaper", "Set as wallpaper", Self::set_wallpaper).accel("w"));
        }
        let menu = Actions::new(&self.window, all);
        let model = menu.menu(&[
            &["copy-image", "copy-path", "set-wallpaper"],
            &[
                "rotate-left",
                "rotate-right",
                "flip-horizontally",
                "flip-vertically",
                "color-assessment",
            ],
            &["show-in-file-manager", "rescan"],
            &["shortcuts"],
        ]);
        self.context_menu.set_menu_model(Some(&menu.menu(&[
            &["open"],
            &["copy-image", "copy-path", "set-wallpaper"],
            &["show-in-file-manager", "properties"],
            &["delete"],
        ])));
        self.view.set_context_menu(&menu.menu(&[
            &["copy-image", "copy-path", "set-wallpaper"],
            &[
                "rotate-left",
                "rotate-right",
                "flip-horizontally",
                "flip-vertically",
                "color-assessment",
            ],
            &["show-in-file-manager", "properties"],
            &["delete"],
        ]));
        self.connect_context_menu();
        let toolbar_end = &self.toolbar.end;
        let button = actions::more_button(&model, 16);
        button.add_css_class("viewer-toolbar-menu");
        toolbar_end.prepend(&button);
        self.view.add_menu_button(&actions::more_button(&model, 20));
        // The i buttons, before the menus.
        toolbar_end.prepend(&self.grid_properties);
        self.view.add_menu_button(&self.view_properties);
        // The open buttons, first in each.
        let open = menu.menu(&[&["choose-folder", "choose-image"]]);
        let button = actions::open_button(&open, 16);
        button.add_css_class("viewer-toolbar-menu");
        self.toolbar.root.prepend(&button);
        self.view.add_open_button(&actions::open_button(&open, 20));
        self.stack.connect_visible_child_name_notify(move |stack| {
            menu.set_in_view(stack.visible_child_name().as_deref() != Some("grid"));
        });
    }

    // A thumbnail's context menu: a right click on it (`grid.rs`, through
    // "win.context-menu"), or the Menu key on the selected one.
    fn connect_context_menu(self: &Rc<Self>) {
        let open = gio::SimpleAction::new(
            "context-menu",
            Some(&<(u32, f64, f64)>::static_variant_type()),
        );
        let weak = Rc::downgrade(self);
        open.connect_activate(move |_, parameter| {
            if let Some(this) = weak.upgrade()
                && let Some((position, x, y)) = parameter.and_then(|p| p.get::<(u32, f64, f64)>())
            {
                this.show_context_menu(position, x, y);
            }
        });
        self.window.add_action(&open);
    }

    // Selects the image at `position`, as a click would, and opens the menu
    // at that point in the grid.
    fn show_context_menu(&self, position: u32, x: f64, y: f64) {
        if position >= self.selection.n_items() {
            return;
        }
        if self.selection.selected() != position {
            self.select(position, true);
        }
        self.context_menu
            .set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        self.context_menu.popup();
    }

    // The menu on the selected thumbnail's middle; at the grid's, when it's
    // scrolled out of sight.
    pub(super) fn show_context_menu_at_selected(&self) {
        let grid = &self.grid;
        let mut cell = grid.first_child();
        while let Some(widget) = &cell {
            if widget.state_flags().contains(gtk::StateFlags::SELECTED) {
                break;
            }
            cell = widget.next_sibling();
        }
        let visible = gtk::graphene::Rect::new(0.0, 0.0, grid.width() as f32, grid.height() as f32);
        let (x, y) = cell
            .and_then(|cell| cell.compute_bounds(grid))
            .and_then(|bounds| bounds.intersection(&visible))
            .map_or_else(
                || (visible.width() / 2.0, visible.height() / 2.0),
                |shown| (shown.center().x(), shown.center().y()),
            );
        self.show_context_menu(self.selection.selected(), x as f64, y as f64);
    }

    pub(super) fn toggle_properties(&self) {
        let button = if self.in_grid() {
            &self.grid_properties
        } else {
            &self.view_properties
        };
        button.set_active(!button.is_active());
    }

    // Decodes the whole image (a GIF's first frame) on a worker for the
    // clipboard.
    pub(super) fn copy_image(self: &Rc<Self>) {
        let Some(path) = self.selected_path() else {
            return;
        };
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let read = path.clone();
            let decoded = gio::spawn_blocking(move || {
                crate::decode::to_fit(&read, u32::MAX, u32::MAX)
                    .map(|fitted| fitted.rgba.premultiplied())
            })
            .await
            .unwrap_or_else(|_| Err("the decode panicked".into()));
            let Some(this) = weak.upgrade() else { return };
            match decoded {
                Ok(pixels) => {
                    this.window.clipboard().set_texture(&pixels.texture());
                    this.toast.show("Copied image", false);
                }
                Err(error) => {
                    eprintln!("Could not copy {}: {error}", path.display());
                    this.toast.show("Could not copy image", false);
                }
            }
        });
    }

    pub(super) fn copy_path(self: &Rc<Self>) {
        if let Some(path) = self.selected_path() {
            self.window.clipboard().set_text(&path.to_string_lossy());
            self.toast.show("Copied path", false);
        }
    }

    pub(super) fn set_wallpaper(self: &Rc<Self>) {
        let (Some(argv), Some(path)) = (self.wallpaper_argv.clone(), self.selected_path()) else {
            return;
        };
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = crate::desktop::set_wallpaper(&argv, &path).await;
            let Some(this) = weak.upgrade() else { return };
            match result {
                Ok(()) => this.toast.show("Wallpaper set", false),
                Err(error) => {
                    eprintln!("Could not set wallpaper {}: {error}", path.display());
                    this.toast.show("Could not set wallpaper", false);
                }
            }
        });
    }
}
