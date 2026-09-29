//! The ⋯ menu: a menu model of window actions ("win.…"), one section per
//! group.

use gtk::{gio, prelude::*};

pub struct MenuAction {
    pub name: &'static str,
    pub label: &'static str,
    // Only labels the item: the keys are handled by the window's key handler
    // (so Ctrl+C still copies text in the folder entry).
    pub accel: Option<&'static str>,
    // Only enabled in the full-screen view.
    pub view_only: bool,
    pub activate: Box<dyn Fn()>,
}

impl MenuAction {
    pub fn new(name: &'static str, label: &'static str, activate: impl Fn() + 'static) -> Self {
        Self {
            name,
            label,
            accel: None,
            view_only: false,
            activate: Box::new(activate),
        }
    }

    pub fn accel(mut self, accel: &'static str) -> Self {
        self.accel = Some(accel);
        self
    }

    pub fn view_only(mut self) -> Self {
        self.view_only = true;
        self
    }
}

pub struct ActionsMenu {
    pub model: gio::Menu,
    view_actions: Vec<gio::SimpleAction>,
}

impl ActionsMenu {
    /// The menu, with its actions added to `window`.
    pub fn new(window: &gtk::ApplicationWindow, sections: Vec<Vec<MenuAction>>) -> Self {
        let model = gio::Menu::new();
        let mut view_actions = Vec::new();
        for section in sections {
            let menu = gio::Menu::new();
            for action in section {
                let item =
                    gio::MenuItem::new(Some(action.label), Some(&format!("win.{}", action.name)));
                if let Some(accel) = action.accel {
                    item.set_attribute_value("accel", Some(&accel.to_variant()));
                }
                menu.append_item(&item);
                let simple = gio::SimpleAction::new(action.name, None);
                simple.set_enabled(!action.view_only);
                let activate = action.activate;
                simple.connect_activate(move |_, _| activate());
                window.add_action(&simple);
                if action.view_only {
                    view_actions.push(simple);
                }
            }
            model.append_section(None, &menu);
        }
        Self {
            model,
            view_actions,
        }
    }

    pub fn set_in_view(&self, in_view: bool) {
        for action in &self.view_actions {
            action.set_enabled(in_view);
        }
    }
}

/// A ⋯ button opening `model`.
pub fn more_button(model: &gio::Menu, pixel_size: i32) -> gtk::MenuButton {
    let button = gtk::MenuButton::builder()
        .tooltip_text("More actions")
        .menu_model(model)
        .build();
    button.set_child(Some(
        &gtk::Image::builder()
            .icon_name("ellipsis-awesome-symbolic")
            .pixel_size(pixel_size)
            .build(),
    ));
    button
}
