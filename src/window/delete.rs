//! Delete (to the trash) and undo, repeatedly back through every delete in
//! the window (`desktop::trash`).

use super::{Window, file_name};
use crate::desktop::trash;
use gtk::{glib, prelude::*};
use std::{path::Path, rc::Rc};

impl Window {
    pub(super) fn connect_trash(self: &Rc<Self>, requests: async_channel::Receiver<()>) {
        let weak = Rc::downgrade(self);
        self.toast.connect_undo(move || {
            if let Some(this) = weak.upgrade() {
                this.undo_delete();
            }
        });
        // Undos one at a time: repeated presses go back through the deletes
        // in order.
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            while requests.recv().await.is_ok() {
                let Some(this) = weak.upgrade() else { return };
                this.undo_one().await;
            }
        });
    }

    // Takes the image out of the grid straight away (so repeated Delete keeps
    // going) and puts it back if trashing fails.
    pub(super) fn delete_selected(self: &Rc<Self>) {
        let Some(path) = self.selected_path() else {
            return;
        };
        if let Some(position) = self.folder.remove(&path) {
            let count = self.selection.n_items();
            if count > 0 {
                self.select(position.min(count - 1), self.in_grid());
            }
        }
        self.refresh_view_if_open();
        self.sync_info();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = trash::move_to_trash(&path).await;
            let Some(this) = weak.upgrade() else { return };
            let name = file_name(&path);
            match result {
                Ok(Some(item)) => {
                    this.undo_stack.borrow_mut().push(item);
                    this.toast.show(&format!("Moved {name} to the trash"), true);
                }
                Ok(None) => this.toast.show(
                    &format!("Moved {name} to the trash (can't be undone)"),
                    false,
                ),
                Err(error) => {
                    eprintln!("Could not delete {}: {error}", path.display());
                    this.toast
                        .show(&format!("Could not move {name} to the trash"), false);
                    this.reinsert(&path);
                }
            }
        });
    }

    pub(super) fn undo_delete(&self) {
        let _ = self.undo_requests.try_send(());
    }

    // Restores the most recent delete.
    async fn undo_one(self: &Rc<Self>) {
        let Some(item) = self.undo_stack.borrow_mut().pop() else {
            return self.toast.show("Nothing to undo", false);
        };
        let name = file_name(&item.original);
        if let Err(reason) = trash::restore(&item).await {
            return self
                .toast
                .show(&format!("Couldn't restore {name}: {reason}"), false);
        }
        if self.reinsert(&item.original).is_some() {
            self.toast.show(&format!("Restored {name}"), false);
        } else {
            let folder = item.original.parent().unwrap_or(Path::new("/"));
            self.toast
                .show(&format!("Restored {name} to {}", folder.display()), false);
        }
    }

    // Puts `path` back in the grid, selected, if it belongs to this folder.
    fn reinsert(self: &Rc<Self>, path: &Path) -> Option<u32> {
        let position = self.folder.add(path);
        if let Some(position) = position {
            self.select(position, self.in_grid());
        }
        self.refresh_view_if_open();
        self.sync_info();
        position
    }
}
