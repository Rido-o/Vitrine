//! Trash and restore through GVfs. Its trash:/// lists every trashed item
//! (from the home trash and per-mount ones like <mount>/.Trash-<uid>) with
//! its original path, and moving an item out of trash:/// restores it and
//! removes its trash record. Without GVfs, deleting and undo are disabled.
//! As Trash.ts.
//!
//! Deletion dates only have one-second resolution, so trashing the same path
//! twice in a second makes "newest" ambiguous. Instead, the items for a path
//! are listed just before and after trashing it: the new one is ours, and
//! undo restores exactly that item.

use gtk::{gio, glib, prelude::*};
use std::{collections::HashSet, path::Path, path::PathBuf};

// Trash items per round trip.
const LIST_BATCH: i32 = 200;

pub struct TrashedItem {
    pub original: PathBuf,
    uri: String,
}

pub fn available() -> bool {
    gio::Vfs::default()
        .supported_uri_schemes()
        .iter()
        .any(|scheme| scheme == "trash")
}

// URIs of the trash:/// items that came from `path`.
async fn items_from(path: &Path) -> Result<HashSet<String>, glib::Error> {
    let trash = gio::File::for_uri("trash:///");
    let enumerator = trash
        .enumerate_children_future(
            "standard::name,trash::orig-path",
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT,
        )
        .await?;
    let wanted = path.as_os_str().as_encoded_bytes();
    let mut uris = HashSet::new();
    loop {
        let infos = enumerator
            .next_files_future(LIST_BATCH, glib::Priority::DEFAULT)
            .await?;
        if infos.is_empty() {
            break;
        }
        for info in infos {
            let from = info.attribute_byte_string("trash::orig-path");
            if from.is_some_and(|from| from.as_bytes() == wanted) {
                uris.insert(trash.child(info.name()).uri().to_string());
            }
        }
    }
    let _ = enumerator.close_future(glib::Priority::DEFAULT).await;
    Ok(uris)
}

/// Trashes `path` (an error if that fails); the item to restore it from, or
/// None if it was trashed but can't be identified for undo.
pub async fn move_to_trash(path: &Path) -> Result<Option<TrashedItem>, glib::Error> {
    let before = items_from(path)
        .await
        .inspect_err(|error| eprintln!("Could not read the trash: {error}"))
        .ok();
    gio::File::for_path(path)
        .trash_future(glib::Priority::DEFAULT)
        .await?;
    let Some(before) = before else {
        return Ok(None);
    };
    match items_from(path).await {
        Ok(after) => {
            let added: Vec<String> = after.difference(&before).cloned().collect();
            if let [uri] = added.as_slice() {
                return Ok(Some(TrashedItem {
                    original: path.to_owned(),
                    uri: uri.clone(),
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

/// Moves a trashed item back to its original path; why not, if it failed.
/// GVfs never overwrites, so an existing file there is reported rather than
/// replaced.
pub async fn restore(item: &TrashedItem) -> Result<(), &'static str> {
    let trashed = gio::File::for_uri(&item.uri);
    if !trashed.query_exists(None::<&gio::Cancellable>) {
        return Err("it's no longer in the trash");
    }
    let (moved, _progress) = trashed.move_future(
        &gio::File::for_path(&item.original),
        gio::FileCopyFlags::NOFOLLOW_SYMLINKS,
        glib::Priority::DEFAULT,
    );
    match moved.await {
        Ok(()) => Ok(()),
        Err(error) if error.matches(gio::IOErrorEnum::Exists) => {
            Err("a file already exists at its old path")
        }
        Err(error) => {
            eprintln!("Could not restore {}: {error}", item.original.display());
            Err("moving it back failed")
        }
    }
}
