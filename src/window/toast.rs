//! Brief messages over both pages ("Copied image", a delete with Undo).

use gtk::{glib, prelude::*};
use std::{cell::RefCell, rc::Rc, time::Duration};

const DURATION: Duration = Duration::from_secs(2);
const UNDO_DURATION: Duration = Duration::from_secs(5);

pub struct Toast {
    pub widget: gtk::Box,
    label: gtk::Label,
    undo: gtk::Button,
    timeout: RefCell<Option<glib::SourceId>>,
}

impl Toast {
    pub fn new() -> Rc<Self> {
        let label = gtk::Label::new(None);
        let widget = gtk::Box::builder()
            .css_classes(["viewer-toast"])
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Start)
            .margin_top(72)
            .spacing(12)
            .visible(false)
            .build();
        widget.append(&label);
        let undo = gtk::Button::builder()
            .label("Undo")
            .tooltip_text("Undo (Ctrl+Z)")
            .visible(false)
            .build();
        widget.append(&undo);
        Rc::new(Self {
            widget,
            label,
            undo,
            timeout: RefCell::default(),
        })
    }

    pub fn connect_undo(&self, undo: impl Fn() + 'static) {
        self.undo.connect_clicked(move |_| undo());
    }

    /// With `undoable`, the Undo button, and longer.
    pub fn show(self: &Rc<Self>, text: &str, undoable: bool) {
        self.label.set_label(text);
        self.undo.set_visible(undoable);
        self.widget.set_visible(true);
        if let Some(id) = self.timeout.borrow_mut().take() {
            id.remove();
        }
        let duration = if undoable { UNDO_DURATION } else { DURATION };
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(duration, move || {
            if let Some(this) = weak.upgrade() {
                this.timeout.borrow_mut().take();
                this.widget.set_visible(false);
            }
        });
        *self.timeout.borrow_mut() = Some(id);
    }
}
