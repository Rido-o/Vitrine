//! The window's keys, in both pages (the view's own are in `View::key`).
//! Keep `shortcuts.rs` and the README in step.

use super::Window;
use gtk::{gdk, glib, prelude::*};
use std::rc::Rc;

impl Window {
    pub(super) fn connect_keys(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::builder()
            .propagation_phase(gtk::PropagationPhase::Capture)
            .build();
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, state| match weak.upgrade() {
            Some(this) if this.key(key, state) => glib::Propagation::Stop,
            _ => glib::Propagation::Proceed,
        });
        self.window.add_controller(keys);
    }

    // Whether `key` was handled.
    fn key(self: &Rc<Self>, key: gdk::Key, state: gdk::ModifierType) -> bool {
        // Letters match in either case (Shift, Caps Lock).
        let key = key.to_lower();
        if self.editing_directory() {
            if key != gdk::Key::Escape {
                return false;
            }
            self.reset_entry();
            self.grid.grab_focus();
            return true;
        }
        if state.contains(gdk::ModifierType::CONTROL_MASK) && key == gdk::Key::z {
            self.undo_delete();
            return true;
        }
        if state.contains(gdk::ModifierType::CONTROL_MASK) && key == gdk::Key::c {
            if state.contains(gdk::ModifierType::SHIFT_MASK) {
                self.copy_path();
            } else {
                self.copy_image();
            }
            return true;
        }
        if state.contains(gdk::ModifierType::CONTROL_MASK)
            && matches!(key, gdk::Key::w | gdk::Key::q)
        {
            self.window.close();
            return true;
        }
        // Everything else is a plain key (Shift is allowed, for + and ?), so
        // e.g. Ctrl+R stays free.
        let held = gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::ALT_MASK
            | gdk::ModifierType::SUPER_MASK;
        if state.intersects(held) {
            return false;
        }
        if key == gdk::Key::r {
            self.folder.rescan();
            return true;
        }
        if matches!(key, gdk::Key::Delete | gdk::Key::KP_Delete) {
            self.delete_selected();
            return true;
        }
        if key == gdk::Key::w && self.wallpaper_argv.is_some() {
            self.set_wallpaper();
            return true;
        }
        if key == gdk::Key::i {
            self.toggle_properties();
            return true;
        }
        if key == gdk::Key::question {
            crate::shortcuts::show(&self.window, self.wallpaper_argv.is_some());
            return true;
        }
        let selected = self.selection.selected() as i64;
        if self.in_grid() {
            if key != gdk::Key::e || self.selection.selected_item().is_none() {
                return false;
            }
            self.show_at(selected);
            return true;
        }
        match key {
            gdk::Key::Right => self.show_at(selected + 1),
            gdk::Key::Left => self.show_at(selected - 1),
            gdk::Key::Escape | gdk::Key::q => self.close_view(),
            // Through the action, so the menu's check follows.
            gdk::Key::b => {
                WidgetExt::activate_action(&self.window, "win.color-assessment", None).ok();
            }
            _ => return self.view.key(key),
        }
        true
    }
}
