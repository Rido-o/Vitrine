// Trash and restore through GVfs. Its trash:/// lists every trashed item (from
// the home trash and per-mount ones like <mount>/.Trash-<uid>) with its
// original path, and moving an item out of trash:/// restores it and removes
// its trash record. Without GVfs, deleting and undo are disabled.
//
// Deletion dates only have one-second resolution, so trashing the same path
// twice in a second makes "newest" ambiguous. Instead, the items for a path
// are listed just before and after trashing it: the new one is ours, and undo
// restores exactly that item.
import Gio from "gi://Gio"
import GLib from "gi://GLib"

export type TrashedItem = { original: string; uri: string }

export type RestoreResult = { ok: true } | { ok: false; reason: string }

export function trashAvailable() {
  return Gio.Vfs.get_default().get_supported_uri_schemes().includes("trash")
}

// URIs of the trash:/// items that came from `path`.
async function itemsFrom(path: string) {
  const trash = Gio.File.new_for_uri("trash:///")
  const enumerator = await new Promise<Gio.FileEnumerator>((resolve, reject) =>
    trash.enumerate_children_async(
      "standard::name,trash::orig-path",
      Gio.FileQueryInfoFlags.NONE,
      GLib.PRIORITY_DEFAULT,
      null,
      (_source, result) => {
        try {
          resolve(trash.enumerate_children_finish(result))
        } catch (error) {
          reject(error)
        }
      },
    ),
  )
  const uris = new Set<string>()
  let infos: Gio.FileInfo[]
  while (
    (infos = await new Promise<Gio.FileInfo[]>((resolve, reject) =>
      enumerator.next_files_async(
        200,
        GLib.PRIORITY_DEFAULT,
        null,
        (_source, result) => {
          try {
            resolve(enumerator.next_files_finish(result))
          } catch (error) {
            reject(error)
          }
        },
      ),
    )).length > 0
  ) {
    for (const info of infos) {
      if (info.get_attribute_byte_string("trash::orig-path") === path) {
        uris.add(enumerator.get_child(info).get_uri())
      }
    }
  }
  enumerator.close(null)
  return uris
}

// Trashes `path` (rejects if that fails). Resolves to the item to restore it
// from, or null if it was trashed but can't be identified for undo.
export async function moveToTrash(path: string): Promise<TrashedItem | null> {
  let before: Set<string> | null = null
  try {
    before = await itemsFrom(path)
  } catch (error) {
    console.error("Could not read the trash:", error)
  }

  const file = Gio.File.new_for_path(path)
  await new Promise<void>((resolve, reject) =>
    file.trash_async(GLib.PRIORITY_DEFAULT, null, (_source, result) => {
      try {
        file.trash_finish(result)
        resolve()
      } catch (error) {
        reject(error)
      }
    }),
  )

  if (!before) return null
  try {
    const added = [...(await itemsFrom(path))].filter((uri) => !before.has(uri))
    if (added.length === 1) return { original: path, uri: added[0] }
    console.error(`Trashed ${path} but found ${added.length} new trash items`)
  } catch (error) {
    console.error("Could not read the trash:", error)
  }
  return null
}

// Moves a trashed item back to its original path. GVfs never overwrites, so
// an existing file there is reported rather than replaced.
export async function restore(item: TrashedItem): Promise<RestoreResult> {
  const trashed = Gio.File.new_for_uri(item.uri)
  if (!trashed.query_exists(null)) {
    return { ok: false, reason: "it's no longer in the trash" }
  }
  try {
    await new Promise<void>((resolve, reject) =>
      trashed.move_async(
        Gio.File.new_for_path(item.original),
        Gio.FileCopyFlags.NOFOLLOW_SYMLINKS,
        GLib.PRIORITY_DEFAULT,
        null,
        null,
        (_source, result) => {
          try {
            trashed.move_finish(result)
            resolve()
          } catch (error) {
            reject(error)
          }
        },
      ),
    )
  } catch (error) {
    if (
      error instanceof GLib.Error &&
      error.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.EXISTS)
    ) {
      return { ok: false, reason: "a file already exists at its old path" }
    }
    console.error(`Could not restore ${item.original}:`, error)
    return { ok: false, reason: "moving it back failed" }
  }
  return { ok: true }
}
