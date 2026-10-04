//! Fading the full-screen view's controls out, and hiding the cursor, after a
//! while without the mouse moving; they come back on movement. Never while
//! the pointer is on a control.

use gtk::{glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
    time::Duration,
};

pub struct AutoHide {
    delay: Duration,
    // Whether the controls hide at all (in the full-screen view).
    active: Box<dyn Fn() -> bool>,
    set_cursor_hidden: Box<dyn Fn(bool)>,
    // Whether something keeps the controls up (an open menu).
    busy: RefCell<Option<Box<dyn Fn() -> bool>>>,
    controls: RefCell<Vec<(gtk::Widget, gtk::EventControllerMotion)>>,
    timeout: RefCell<Option<glib::SourceId>>,
    last: Cell<(f64, f64)>,
    this: Weak<Self>,
}

impl AutoHide {
    pub fn new(
        delay: Duration,
        active: impl Fn() -> bool + 'static,
        set_cursor_hidden: impl Fn(bool) + 'static,
    ) -> Rc<Self> {
        Rc::new_cyclic(|this| Self {
            delay,
            active: Box::new(active),
            set_cursor_hidden: Box::new(set_cursor_hidden),
            busy: RefCell::default(),
            controls: RefCell::default(),
            timeout: RefCell::default(),
            last: Cell::new((-1.0, -1.0)),
            this: this.clone(),
        })
    }

    /// While `busy` is true the controls stay (checked when they'd hide).
    pub fn set_busy(&self, busy: impl Fn() -> bool + 'static) {
        *self.busy.borrow_mut() = Some(Box::new(busy));
    }

    /// A control to hide; its "hidden" CSS class fades it.
    pub fn add(&self, widget: &impl IsA<gtk::Widget>) {
        let motion = gtk::EventControllerMotion::new();
        widget.add_controller(motion.clone());
        self.controls
            .borrow_mut()
            .push((widget.clone().upcast(), motion));
    }

    /// The area whose mouse movement shows the controls again. GTK also
    /// reports motion when widgets change under a still pointer (as when the
    /// controls stop taking input), so only a real move counts.
    pub fn watch(&self, area: &impl IsA<gtk::Widget>) {
        let motion = gtk::EventControllerMotion::new();
        let this = self.this.clone();
        motion.connect_motion(move |_, x, y| {
            let Some(this) = this.upgrade() else { return };
            if this.last.get() == (x, y) {
                return;
            }
            this.last.set((x, y));
            this.show();
        });
        area.add_controller(motion);
    }

    /// Shows the controls and cursor, and (when active) hides them again
    /// later.
    pub fn show(&self) {
        for (widget, _) in self.controls.borrow().iter() {
            widget.remove_css_class("hidden");
            widget.set_can_target(true);
        }
        (self.set_cursor_hidden)(false);
        self.cancel();
        if !(self.active)() {
            return;
        }
        let this = self.this.clone();
        let id = glib::timeout_add_local_once(self.delay, move || {
            if let Some(this) = this.upgrade() {
                this.timeout.borrow_mut().take();
                this.hide();
            }
        });
        *self.timeout.borrow_mut() = Some(id);
    }

    fn hide(&self) {
        if !(self.active)() {
            return;
        }
        let on_control = self
            .controls
            .borrow()
            .iter()
            .any(|(_, motion)| motion.contains_pointer());
        let busy = self.busy.borrow().as_ref().is_some_and(|busy| busy());
        if on_control || busy {
            return self.show();
        }
        for (widget, _) in self.controls.borrow().iter() {
            widget.add_css_class("hidden");
            widget.set_can_target(false);
        }
        (self.set_cursor_hidden)(true);
    }

    // Stops a pending hide.
    fn cancel(&self) {
        if let Some(id) = self.timeout.borrow_mut().take() {
            id.remove();
        }
    }
}
