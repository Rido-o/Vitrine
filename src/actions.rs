//! The menus' window actions ("win.…") and the menu models made of them
//! (the ⋯ menu, a thumbnail's context menu), one section per group.

use gtk::{gio, prelude::*};

pub struct MenuAction {
    pub name: &'static str,
    pub label: &'static str,
    // Only labels the item: the keys are handled by the window's key handler
    // (so Ctrl+C still copies text in the folder entry).
    pub accel: Option<&'static str>,
    // Only enabled in the full-screen view.
    pub view_only: bool,
    // A check item, showing this after each activation.
    pub checked: Option<Box<dyn Fn() -> bool>>,
    pub activate: Box<dyn Fn()>,
}

impl MenuAction {
    pub fn new(name: &'static str, label: &'static str, activate: impl Fn() + 'static) -> Self {
        Self {
            name,
            label,
            accel: None,
            view_only: false,
            checked: None,
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

    pub fn checked(mut self, checked: impl Fn() -> bool + 'static) -> Self {
        self.checked = Some(Box::new(checked));
        self
    }
}

pub struct Actions {
    items: Vec<(&'static str, gio::MenuItem)>,
    view_actions: Vec<gio::SimpleAction>,
}

impl Actions {
    /// The actions, added to `window`.
    pub fn new(window: &gtk::ApplicationWindow, actions: Vec<MenuAction>) -> Self {
        let mut items = Vec::new();
        let mut view_actions = Vec::new();
        for action in actions {
            let item =
                gio::MenuItem::new(Some(action.label), Some(&format!("win.{}", action.name)));
            if let Some(accel) = action.accel {
                item.set_attribute_value("accel", Some(&accel.to_variant()));
            }
            items.push((action.name, item));
            let simple = match &action.checked {
                Some(checked) => {
                    gio::SimpleAction::new_stateful(action.name, None, &checked().to_variant())
                }
                None => gio::SimpleAction::new(action.name, None),
            };
            simple.set_enabled(!action.view_only);
            let (activate, checked) = (action.activate, action.checked);
            simple.connect_activate(move |simple, _| {
                activate();
                if let Some(checked) = &checked {
                    simple.set_state(&checked().to_variant());
                }
            });
            window.add_action(&simple);
            if action.view_only {
                view_actions.push(simple);
            }
        }
        Self {
            items,
            view_actions,
        }
    }

    /// A menu of the named actions, a section per group; names without an
    /// action (no wallpaper command) are left out.
    pub fn menu(&self, sections: &[&[&str]]) -> gio::Menu {
        let model = gio::Menu::new();
        for names in sections {
            let section = gio::Menu::new();
            for name in *names {
                if let Some((_, item)) = self.items.iter().find(|(action, _)| action == name) {
                    section.append_item(item);
                }
            }
            model.append_section(None, &section);
        }
        model
    }

    pub fn set_in_view(&self, in_view: bool) {
        for action in &self.view_actions {
            action.set_enabled(in_view);
        }
    }
}

/// An open button: `model` has the folder and image choosers.
pub fn open_button(model: &gio::Menu, pixel_size: i32) -> gtk::MenuButton {
    let button = gtk::MenuButton::builder()
        .tooltip_text("Open a folder or an image")
        .menu_model(model)
        .build();
    button.set_child(Some(&crate::icon("folder-awesome-symbolic", pixel_size)));
    button
}

/// A ⋯ button opening `model`.
pub fn more_button(model: &gio::Menu, pixel_size: i32) -> gtk::MenuButton {
    let button = gtk::MenuButton::builder()
        .tooltip_text("More actions")
        .menu_model(model)
        .build();
    button.set_child(Some(&crate::icon("ellipsis-awesome-symbolic", pixel_size)));
    button
}
