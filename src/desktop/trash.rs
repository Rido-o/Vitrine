//! Trash and restore through the freedesktop.org trash, written directly by
//! the `trash` crate (no GVfs): the home trash, and per-drive ones like
//! <mount>/.Trash-<uid>, each item a file and a record of its original path.
//! It all blocks (listing reads every record and looks at every mount), so it
//! runs on a worker thread.
//!
//! Deletion dates only have one-second resolution, so trashing the same path
//! twice in a second makes "newest" ambiguous. Instead, the items for a path
//! are listed just before and after trashing it: the new one is ours, and
//! undo restores exactly that item.

use gtk::gio;
use std::{
    collections::HashSet,
    ffi::OsString,
    io::ErrorKind,
    path::{Path, PathBuf},
};
use trash::{Error, TrashItem, os_limited};

pub struct TrashedItem {
    pub original: PathBuf,
    item: TrashItem,
}

// Where the trash records `path` came from: its folder resolved, as the
// crate does before trashing (the file itself may be a symlink, and is
// trashed as one).
fn recorded_path(path: &Path) -> Option<PathBuf> {
    Some(path.parent()?.canonicalize().ok()?.join(path.file_name()?))
}

// The trash items that came from `recorded`.
fn items_from(recorded: &Path) -> Result<Vec<TrashItem>, Error> {
    Ok(os_limited::list()?
        .into_iter()
        .filter(|item| item.original_path() == recorded)
        .collect())
}

/// Trashes `path` (an error if that fails); the item to restore it from, or
/// None if it was trashed but can't be identified for undo.
pub async fn move_to_trash(path: &Path) -> Result<Option<TrashedItem>, String> {
    let path = path.to_owned();
    gio::spawn_blocking(move || trash_blocking(path))
        .await
        .unwrap_or_else(|_| Err("trashing panicked".into()))
}

fn trash_blocking(path: PathBuf) -> Result<Option<TrashedItem>, String> {
    let recorded = recorded_path(&path);
    let modified = std::fs::symlink_metadata(&path)
        .ok()
        .filter(std::fs::Metadata::is_file)
        .and_then(|metadata| metadata.modified().ok());
    let before: Option<HashSet<OsString>> = recorded.as_deref().and_then(|recorded| {
        items_from(recorded)
            .inspect_err(|error| eprintln!("Could not read the trash: {error}"))
            .ok()
            .map(|items| items.into_iter().map(|item| item.id).collect())
    });
    trash::delete(&path).map_err(|error| error.to_string())?;
    let (Some(recorded), Some(before)) = (recorded, before) else {
        return Ok(None);
    };
    match items_from(&recorded) {
        Ok(after) => {
            let mut added: Vec<TrashItem> = after
                .into_iter()
                .filter(|item| !before.contains(&item.id))
                .collect();
            if added.len() == 1 {
                let item = added.remove(0);
                if let (Some(file), Some(modified)) = (trashed_file(&item), modified) {
                    keep_modified(&file, modified);
                }
                return Ok(Some(TrashedItem {
                    original: path,
                    item,
                }));
            }
            eprintln!(
                "Trashed {} but found {} new trash items",
                path.display(),
                added.len()
            );
        }
        Err(error) => eprintln!("Could not read the trash: {error}"),
    }
    Ok(None)
}

// The item's own file, beside its record: <trash>/files/<name>.
fn trashed_file(item: &TrashItem) -> Option<PathBuf> {
    let info = Path::new(&item.id);
    Some(
        info.parent()?
            .parent()?
            .join("files")
            .join(info.file_stem()?),
    )
}

// Trashing across filesystems (an image from a drive without a trash of its
// own goes to the home trash) copies it with a new modification time: the old
// one is put back, so a restored image keeps its place in Date order and its
// thumbnail.
fn keep_modified(file: &Path, modified: std::time::SystemTime) {
    let changed = std::fs::symlink_metadata(file)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|now| now != modified);
    if changed
        && let Err(error) = std::fs::File::options()
            .write(true)
            .open(file)
            .and_then(|opened| opened.set_modified(modified))
    {
        eprintln!("Could not keep the date of {}: {error}", file.display());
    }
}

/// Moves a trashed item back to its original path; why not, if it failed.
/// An existing file there is reported rather than replaced.
pub async fn restore(item: &TrashedItem) -> Result<(), &'static str> {
    let item = item.item.clone();
    gio::spawn_blocking(move || restore_blocking(item))
        .await
        .unwrap_or(Err("moving it back failed"))
}

fn restore_blocking(item: TrashItem) -> Result<(), &'static str> {
    let target = item.original_path();
    let info = PathBuf::from(&item.id);
    let Some(file) = trashed_file(&item) else {
        return Err("moving it back failed");
    };
    let existed = target.symlink_metadata().is_ok();
    match os_limited::restore_all([item]) {
        Ok(()) => Ok(()),
        Err(Error::RestoreCollision { .. }) => Err("a file already exists at its old path"),
        Err(Error::FileSystem { path, source }) if path == file => {
            remove_placeholder(&target, existed);
            match source.kind() {
                ErrorKind::NotFound => Err("it's no longer in the trash"),
                ErrorKind::CrossesDevices => copy_back(&file, &target, &info),
                _ => {
                    eprintln!("Could not restore {}: {source}", target.display());
                    Err("moving it back failed")
                }
            }
        }
        // The file is back; only its record is left over.
        Err(Error::FileSystem { path, source }) if path == info => {
            eprintln!(
                "Restored {} but not its trash record: {source}",
                target.display()
            );
            Ok(())
        }
        Err(error) => {
            remove_placeholder(&target, existed);
            eprintln!("Could not restore {}: {error}", target.display());
            Err("moving it back failed")
        }
    }
}

// Restoring creates an empty file at the old path first (so nothing else
// takes it), then renames the item over it; if that fails, the empty file
// would stay where the image was.
fn remove_placeholder(target: &Path, existed: bool) {
    let empty = std::fs::symlink_metadata(target).is_ok_and(|m| m.is_file() && m.len() == 0);
    if !existed && empty {
        let _ = std::fs::remove_file(target);
    }
}

// The crate restores with a rename, which can't cross filesystems: an item
// from another drive that went to the home trash (its drive had no trash of
// its own) is copied back instead, keeping its modification time, then
// removed from the trash.
fn copy_back(file: &Path, target: &Path, info: &Path) -> Result<(), &'static str> {
    let copied = (|| -> std::io::Result<()> {
        let metadata = file.symlink_metadata()?;
        if metadata.is_symlink() {
            std::os::unix::fs::symlink(std::fs::read_link(file)?, target)
        } else {
            std::fs::copy(file, target)?;
            std::fs::File::options()
                .write(true)
                .open(target)?
                .set_modified(metadata.modified()?)
        }
    })();
    if let Err(error) = copied {
        eprintln!("Could not restore {}: {error}", target.display());
        let _ = std::fs::remove_file(target);
        return Err("moving it back failed");
    }
    if let Err(error) = std::fs::remove_file(file).and_then(|()| std::fs::remove_file(info)) {
        eprintln!(
            "Restored {} but it's still in the trash: {error}",
            target.display()
        );
    }
    Ok(())
}
