//! The ⋯ menu (in the toolbar and in the view) and its actions, and the i
//! buttons' properties popovers.

use super::{Window, icon};
use crate::actions::{self, ActionsMenu, MenuAction};
use gtk::{gio, glib, prelude::*};
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
        let mut image = vec![
            action("copy-image", "Copy image", Self::copy_image).accel("<Control>c"),
            action("copy-path", "Copy path", Self::copy_path).accel("<Control><Shift>c"),
        ];
        if self.wallpaper_argv.is_some() {
            image.push(action("set-wallpaper", "Set as wallpaper", Self::set_wallpaper).accel("w"));
        }
        let menu = ActionsMenu::new(
            &self.window,
            vec![
                image,
                vec![
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
                ],
                vec![
                    action("show-in-file-manager", "Show in file manager", |this| {
                        if let Some(path) = this.selected_path() {
                            crate::desktop::show_in_file_manager(&path);
                        }
                    }),
                    action("rescan", "Rescan folder", |this| this.folder.rescan()).accel("r"),
                ],
                vec![
                    action("shortcuts", "Keyboard shortcuts", |this| {
                        crate::shortcuts::show(&this.window, this.wallpaper_argv.is_some())
                    })
                    .accel("question"),
                ],
            ],
        );
        let toolbar_end = &self.toolbar.end;
        let button = actions::more_button(&menu.model, 16);
        button.add_css_class("viewer-toolbar-menu");
        toolbar_end.prepend(&button);
        self.view
            .add_menu_button(&actions::more_button(&menu.model, 20));
        // The i buttons, before the menus.
        toolbar_end.prepend(&self.grid_properties);
        self.view.add_menu_button(&self.view_properties);
        self.stack.connect_visible_child_name_notify(move |stack| {
            menu.set_in_view(stack.visible_child_name().as_deref() != Some("grid"));
        });
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
