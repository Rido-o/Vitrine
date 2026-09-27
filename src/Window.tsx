import Gdk from "gi://Gdk?version=4.0"
import Gio from "gi://Gio"
import GLib from "gi://GLib"
import Graphene from "gi://Graphene"
import Gtk from "gi://Gtk?version=4.0"
import History from "./History"
import Library, { type SortKey } from "./Library"
import { getImageInfo, loadThumbnail } from "./Thumbnails"
import ZoomableImage from "./ZoomableImage"
import {
  APP_TITLE,
  isDirectory,
  normalizeDirectory,
  setWallpaper,
  showInFileManager,
  wallpaperCommand,
} from "./util"

const TILE_WIDTH = 272
const TILE_HEIGHT = 153
const TOAST_SECONDS = 2

// A viewer window on `directory` (and its subfolders with `subfolders`); with
// `file`, that image opens in the full-screen view.
export default function ViewerWindow(
  app: Gtk.Application,
  directory: string,
  file: string | null = null,
  subfolders = false,
) {
  let win: Gtk.ApplicationWindow
  let stack: Gtk.Stack
  let directoryEntry: Gtk.Entry
  let toast: Gtk.Label
  let toastTimeout = 0
  let emptyLabel: Gtk.Label
  let pendingSelection: string | null = null
  let autoSelected: string | null = null
  let directionButton: Gtk.Button
  const sortButtons = new Map<SortKey, Gtk.Button>()
  const filenameLabels: Gtk.Label[] = []
  const resolutionLabels: Gtk.Label[] = []
  const wallpaperArgv = wallpaperCommand()

  const library = new Library()
  library.recursive = subfolders
  const history = new History()
  const preview = new ZoomableImage({ hexpand: true, vexpand: true })

  const historyList = new Gtk.Box({
    orientation: Gtk.Orientation.VERTICAL,
    spacing: 2,
  })
  const historyPanel = new Gtk.Box({
    cssClasses: ["viewer-history"],
    halign: Gtk.Align.START,
    valign: Gtk.Align.START,
    visible: false,
  })
  historyPanel.append(historyList)

  const selection = new Gtk.SingleSelection({
    model: library.model,
    autoselect: false,
    canUnselect: false,
  })

  const factory = new Gtk.SignalListItemFactory()
  factory.connect("setup", (_, listItem) => {
    const picture = new Gtk.Picture({
      widthRequest: TILE_WIDTH,
      heightRequest: TILE_HEIGHT,
      contentFit: Gtk.ContentFit.CONTAIN,
      canShrink: true,
    })
    picture.add_css_class("viewer-thumbnail")
    ;(listItem as Gtk.ListItem).set_child(picture)
  })
  factory.connect("bind", (_, object) => {
    const listItem = object as Gtk.ListItem
    const path = (listItem.get_item() as Gtk.StringObject).get_string()
    const picture = listItem.get_child() as Gtk.Picture
    picture.set_paintable(null)
    loadThumbnail(path, library.mtime(path))
      .then((texture) => {
        const current = listItem.get_item() as Gtk.StringObject | null
        if (current?.get_string() === path) picture.set_paintable(texture)
      })
      .catch(() => {})
  })

  const grid = new Gtk.GridView({
    model: selection,
    factory,
    minColumns: 1,
    maxColumns: 12,
    singleClickActivate: false,
  })
  grid.add_css_class("viewer-grid")

  function getSelectedPath() {
    const item = selection.get_selected_item()
    return item ? (item as Gtk.StringObject).get_string() : null
  }

  function withSelectedPath(fn: (path: string) => void) {
    const path = getSelectedPath()
    if (path) fn(path)
  }

  function select(index: number, focus = false) {
    if (index < 0 || index >= library.paths.length) {
      selection.selected = library.paths.length > 0 ? 0 : Gtk.INVALID_LIST_POSITION
      return
    }
    selection.selected = index
    grid.scroll_to(
      index,
      focus ? Gtk.ListScrollFlags.FOCUS : Gtk.ListScrollFlags.NONE,
      null,
    )
  }

  function showToast(text: string) {
    toast.label = text
    toast.visible = true
    if (toastTimeout) GLib.source_remove(toastTimeout)
    toastTimeout = GLib.timeout_add_seconds(
      GLib.PRIORITY_DEFAULT,
      TOAST_SECONDS,
      () => {
        toast.visible = false
        toastTimeout = 0
        return GLib.SOURCE_REMOVE
      },
    )
  }

  // --- info labels ---------------------------------------------------------

  function syncInfoLabels() {
    const path = getSelectedPath()
    const info = path ? getImageInfo(path) : null
    for (const label of filenameLabels)
      label.label = info?.filename ?? "No image selected"
    for (const label of resolutionLabels)
      label.label = info?.resolution ?? "0 × 0"
    win.title = info ? `${info.filename} — ${APP_TITLE}` : APP_TITLE
    emptyLabel.visible = library.paths.length === 0
    emptyLabel.label = library.loading
      ? "Scanning…"
      : library.recursive
        ? "No images in this folder or its subfolders"
        : "No images in this folder — turn on Subfolders to include them"
  }

  // --- actions -------------------------------------------------------------

  function setSelectedAsWallpaper() {
    if (!wallpaperArgv) return
    withSelectedPath((path) => {
      setWallpaper(wallpaperArgv, path)
        .then(() => showToast("Wallpaper set"))
        .catch((error) => {
          console.error(`Could not set wallpaper ${path}:`, error)
          showToast("Could not set wallpaper")
        })
    })
  }

  function deleteSelected() {
    withSelectedPath((path) => {
      try {
        Gio.File.new_for_path(path).trash(null)
      } catch (error) {
        console.error(`Could not delete ${path}:`, error)
        showToast("Could not move to trash")
        return
      }
      const index = library.remove(path)
      if (index !== -1) select(Math.min(index, library.paths.length - 1), true)
      refreshPreviewIfOpen()
      syncInfoLabels()
    })
  }

  function rescan() {
    if (library.loading) return
    library.rescan().then((done) => {
      if (done) refreshPreviewIfOpen()
    })
  }

  // --- folder --------------------------------------------------------------

  function resetDirectoryEntry() {
    directoryEntry.text = library.directory
    directoryEntry.remove_css_class("error")
  }

  function isEditingDirectory() {
    const focus = win.get_focus()
    return (
      focus !== null &&
      (focus === directoryEntry || focus.is_ancestor(directoryEntry))
    )
  }

  // Runs after each scan batch: selects `pendingSelection` once it turns up,
  // otherwise the first image (after the scan when waiting for a file, so the
  // full-screen view doesn't jump to another image).
  function onLibraryChanged() {
    if (pendingSelection) {
      const index = library.paths.indexOf(pendingSelection)
      if (index !== -1) {
        pendingSelection = null
        select(index, stack.visibleChildName === "grid")
      } else if (!library.loading) {
        pendingSelection = null
      }
    }
    // Batches can insert images ahead of the auto-selected one; keep the
    // selection on the first image until the user picks something else.
    const autoSelectionKept =
      autoSelected !== null && getSelectedPath() === autoSelected
    if (
      library.paths.length > 0 &&
      !pendingSelection &&
      (selection.selected === Gtk.INVALID_LIST_POSITION || autoSelectionKept)
    ) {
      select(0, stack.visibleChildName === "grid")
      autoSelected = library.loading ? getSelectedPath() : null
    } else if (!autoSelectionKept) {
      autoSelected = null
    }
    syncInfoLabels()
  }
  library.onChanged = onLibraryChanged

  function openDirectory(input: string, selectFile: string | null = null) {
    const path = normalizeDirectory(input)
    if (!isDirectory(path)) {
      directoryEntry.add_css_class("error")
      return false
    }
    pendingSelection = selectFile
    library.load(path)
    resetDirectoryEntry()
    history.remember(path)
    renderHistory()
    grid.grab_focus()
    return true
  }

  function setRecursive(recursive: boolean) {
    library.recursive = recursive
    pendingSelection = getSelectedPath()
    library.load(library.directory)
  }

  // --- history panel -------------------------------------------------------

  function renderHistory() {
    let child: Gtk.Widget | null
    while ((child = historyList.get_first_child())) historyList.remove(child)
    for (const entry of history.entries) {
      const button = new Gtk.Button({
        child: new Gtk.Label({ label: entry, xalign: 0, ellipsize: 1 }),
        tooltipText: entry,
      })
      button.connect("clicked", () => {
        hideHistory()
        openDirectory(entry)
      })
      historyList.append(button)
    }
  }

  const historyKeys = new Gtk.EventControllerKey()
  historyKeys.connect("key-pressed", (_c, keyval) => {
    if (keyval === Gdk.KEY_Down)
      return historyList.child_focus(Gtk.DirectionType.TAB_FORWARD)
    if (keyval === Gdk.KEY_Up)
      return historyList.child_focus(Gtk.DirectionType.TAB_BACKWARD)
    return false
  })
  historyList.add_controller(historyKeys)

  function hideHistory() {
    historyPanel.visible = false
  }

  function showHistory() {
    historyPanel.widthRequest = directoryEntry.get_width()
    historyPanel.visible = true
    let button = historyList.get_first_child()
    const current = history.entries.indexOf(library.directory)
    for (let i = 0; i < current && button; i++) {
      button = button.get_next_sibling()
    }
    ;(button ?? historyList.get_first_child())?.grab_focus()
  }

  function toggleHistory() {
    if (historyPanel.visible) {
      hideHistory()
      directoryEntry.grab_focus()
    } else {
      showHistory()
    }
  }

  // --- sorting -------------------------------------------------------------

  function syncSortButtons() {
    for (const [key, button] of sortButtons) {
      if (key === library.sortKey) button.add_css_class("active")
      else button.remove_css_class("active")
    }
    directionButton.label = library.descending ? "↓" : "↑"
    directionButton.sensitive = library.sortKey !== "random"
    directionButton.tooltipText = library.descending
      ? "Descending"
      : "Ascending"
  }

  function keepingSelection(fn: () => void) {
    const selected = getSelectedPath()
    fn()
    select(selected ? library.paths.indexOf(selected) : 0)
    syncSortButtons()
  }

  // --- full-screen view ----------------------------------------------------

  function updatePreviewImage() {
    withSelectedPath((path) => preview.setFile(path))
  }

  function refreshPreviewIfOpen() {
    if (stack.visibleChildName !== "preview") return
    if (library.paths.length === 0) hidePreview()
    else updatePreviewImage()
  }

  function showPreview(path: string) {
    const index = library.paths.indexOf(path)
    if (index === -1) return
    selection.selected = index
    updatePreviewImage()
    stack.visibleChildName = "preview"
  }

  function movePreview(offset: number) {
    const count = library.paths.length
    if (count === 0) return
    const current = selection.selected
    const base = current === Gtk.INVALID_LIST_POSITION ? 0 : current
    selection.selected = (base + offset + count) % count
  }

  function hidePreview() {
    const index = selection.selected
    preview.setFile(null)
    stack.visibleChildName = "grid"
    GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
      if (index !== Gtk.INVALID_LIST_POSITION) {
        grid.scroll_to(
          index,
          Gtk.ListScrollFlags.FOCUS | Gtk.ListScrollFlags.SMOOTH,
          null,
        )
      }
      grid.grab_focus()
      return GLib.SOURCE_REMOVE
    })
  }

  grid.connect("activate", (_, position) => {
    const item = library.model.get_item(position) as Gtk.StringObject | null
    if (item) showPreview(item.get_string())
  })

  selection.connect("selection-changed", () => {
    syncInfoLabels()
    if (stack.visibleChildName === "preview") updatePreviewImage()
  })

  // --- input ---------------------------------------------------------------

  function onKey(_c: Gtk.EventControllerKey, keyval: number) {
    if (historyPanel.visible) {
      if (keyval !== Gdk.KEY_Escape) return false
      hideHistory()
      directoryEntry.grab_focus()
      return true
    }
    if (isEditingDirectory()) {
      if (keyval !== Gdk.KEY_Escape) return false
      resetDirectoryEntry()
      grid.grab_focus()
      return true
    }
    if (keyval === Gdk.KEY_Delete || keyval === Gdk.KEY_KP_Delete)
      return (deleteSelected(), true)
    if (keyval === Gdk.KEY_r || keyval === Gdk.KEY_R) return (rescan(), true)
    if (keyval === Gdk.KEY_w || keyval === Gdk.KEY_W)
      return (setSelectedAsWallpaper(), true)
    if (stack.visibleChildName === "preview") {
      if (keyval === Gdk.KEY_Right) return (movePreview(1), true)
      if (keyval === Gdk.KEY_Left) return (movePreview(-1), true)
      if (
        keyval === Gdk.KEY_plus ||
        keyval === Gdk.KEY_equal ||
        keyval === Gdk.KEY_KP_Add
      )
        return (preview.zoomIn(), true)
      if (keyval === Gdk.KEY_minus || keyval === Gdk.KEY_KP_Subtract)
        return (preview.zoomOut(), true)
      if (keyval === Gdk.KEY_0 || keyval === Gdk.KEY_KP_0)
        return (preview.resetZoom(), true)
      if (keyval === Gdk.KEY_s || keyval === Gdk.KEY_S)
        return (preview.toggleSharp(), true)
      if (keyval === Gdk.KEY_Escape || keyval === Gdk.KEY_q)
        return (hidePreview(), true)
      return false
    }
    if (keyval === Gdk.KEY_e || keyval === Gdk.KEY_E)
      return (withSelectedPath(showPreview), true)
    if (keyval === Gdk.KEY_Escape || keyval === Gdk.KEY_q)
      return (win.close(), true)
    return false
  }

  function contains(widget: Gtk.Widget, x: number, y: number) {
    const [ok, rect] = widget.compute_bounds(win)
    return ok && rect.contains_point(new Graphene.Point({ x, y }))
  }

  function onClick(_g: Gtk.GestureClick, _b: number, x: number, y: number) {
    if (
      historyPanel.visible &&
      !contains(historyPanel, x, y) &&
      !contains(directoryEntry, x, y)
    ) {
      hideHistory()
    }
  }

  function InfoLabels() {
    return (
      <>
        <button
          class="flat viewer-filename-button"
          tooltipText="Show in file manager"
          onClicked={() => withSelectedPath(showInFileManager)}
        >
          <label
            $={(self) => filenameLabels.push(self)}
            label="No image selected"
            ellipsize={3}
          />
        </button>
        <label $={(self) => resolutionLabels.push(self)} label="0 × 0" />
      </>
    )
  }

  function SortButton({ sort, label, tooltip }: {
    sort: SortKey
    label: string
    tooltip: string
  }) {
    return (
      <button
        $={(self) => sortButtons.set(sort, self)}
        label={label}
        tooltipText={tooltip}
        onClicked={() => keepingSelection(() => library.setSort(sort))}
      />
    )
  }

  // --- layout --------------------------------------------------------------

  win = (
    <Gtk.ApplicationWindow
      application={app}
      title={APP_TITLE}
      defaultWidth={1600}
      defaultHeight={1000}
    >
      <Gtk.EventControllerKey onKeyPressed={onKey} />
      <Gtk.GestureClick onReleased={onClick} />

      <overlay>
        <stack
          $={(self) => (stack = self)}
          transitionType={Gtk.StackTransitionType.CROSSFADE}
          transitionDuration={150}
          hexpand
          vexpand
        >
          <box
            $type="named"
            name="grid"
            class="viewer-library"
            orientation={Gtk.Orientation.VERTICAL}
          >
            <box class="viewer-toolbar" spacing={8}>
              <entry
                $={(self) => {
                  directoryEntry = self
                  const keys = new Gtk.EventControllerKey({
                    propagationPhase: Gtk.PropagationPhase.CAPTURE,
                  })
                  keys.connect("key-pressed", (_c, keyval) => {
                    if (keyval !== Gdk.KEY_Down) return false
                    showHistory()
                    return true
                  })
                  self.add_controller(keys)
                }}
                class="viewer-directory"
                primaryIconName="folder-awesome-symbolic"
                secondaryIconName="chevron-down-awesome-symbolic"
                secondaryIconTooltipText="Recent folders (↓)"
                tooltipText="Folder (Enter to open)"
                widthChars={36}
                onActivate={({ text }) => openDirectory(text)}
                onIconRelease={(_self, position) => {
                  if (position === Gtk.EntryIconPosition.SECONDARY)
                    toggleHistory()
                }}
              />
              <box class="viewer-pill" spacing={2}>
                <SortButton sort="name" label="Name" tooltip="Sort by path" />
                <SortButton
                  sort="date"
                  label="Date"
                  tooltip="Sort by date modified"
                />
                <SortButton sort="size" label="Size" tooltip="Sort by file size" />
                <SortButton
                  sort="random"
                  label="Random"
                  tooltip="Shuffle (click again to reshuffle)"
                />
                <button
                  $={(self) => (directionButton = self)}
                  label="↑"
                  onClicked={() =>
                    keepingSelection(() => library.toggleDirection())
                  }
                />
              </box>
              <box class="viewer-pill">
                <togglebutton
                  label="Subfolders"
                  active={subfolders}
                  tooltipText="Include images in subfolders"
                  onToggled={({ active }) => setRecursive(active)}
                />
              </box>
            </box>
            <Gtk.Separator
              class="viewer-toolbar-separator"
              orientation={Gtk.Orientation.HORIZONTAL}
            />
            <Gtk.Overlay
              $={(self) => self.add_overlay(historyPanel)}
              hexpand
              vexpand
            >
              <scrolledwindow
                hexpand
                vexpand
                hscrollbarPolicy={Gtk.PolicyType.NEVER}
                vscrollbarPolicy={Gtk.PolicyType.AUTOMATIC}
              >
                {grid}
              </scrolledwindow>
              <label
                $type="overlay"
                $={(self) => (emptyLabel = self)}
                class="viewer-empty"
                halign={Gtk.Align.CENTER}
                valign={Gtk.Align.CENTER}
                visible={false}
              />
            </Gtk.Overlay>
            <Gtk.Separator orientation={Gtk.Orientation.HORIZONTAL} />
            <box class="viewer-info" spacing={8}>
              <box hexpand halign={Gtk.Align.START} spacing={8}>
                <InfoLabels />
              </box>
              <box spacing={6} halign={Gtk.Align.END}>
                <button tooltipText="Rescan folder (r)" onClicked={rescan}>
                  <image
                    iconName="arrows-rotate-awesome-symbolic"
                    pixelSize={16}
                  />
                </button>
                <button
                  tooltipText="Full-screen view (Enter)"
                  onClicked={() => withSelectedPath(showPreview)}
                  label="View"
                />
                <button
                  visible={wallpaperArgv !== null}
                  tooltipText="Set as wallpaper (w)"
                  onClicked={setSelectedAsWallpaper}
                  label="Set wallpaper"
                />
              </box>
            </box>
          </box>

          <box
            $type="named"
            name="preview"
            class="viewer-preview"
            hexpand
            vexpand
          >
            <Gtk.Overlay hexpand vexpand>
              {preview}
              <button
                $type="overlay"
                class="preview-close-button"
                tooltipText="Back to grid (Esc)"
                halign={Gtk.Align.END}
                valign={Gtk.Align.START}
                marginTop={16}
                marginEnd={16}
                onClicked={hidePreview}
              >
                <image iconName="xmark-awesome-symbolic" pixelSize={20} />
              </button>
              <box
                $type="overlay"
                halign={Gtk.Align.CENTER}
                valign={Gtk.Align.END}
                spacing={8}
                marginBottom={24}
              >
                <button
                  class="preview-navigation-button"
                  tooltipText="Previous image (←)"
                  onClicked={() => movePreview(-1)}
                >
                  <image
                    iconName="arrow-left-awesome-symbolic"
                    pixelSize={20}
                  />
                </button>
                <box
                  class="preview-image-info"
                  spacing={8}
                  valign={Gtk.Align.CENTER}
                >
                  <InfoLabels />
                  <button
                    visible={wallpaperArgv !== null}
                    tooltipText="Set as wallpaper (w)"
                    onClicked={setSelectedAsWallpaper}
                    label="Set wallpaper"
                  />
                </box>
                <button
                  class="preview-navigation-button"
                  tooltipText="Next image (→)"
                  onClicked={() => movePreview(1)}
                >
                  <image
                    iconName="arrow-right-awesome-symbolic"
                    pixelSize={20}
                  />
                </button>
              </box>
            </Gtk.Overlay>
          </box>
        </stack>
        <label
          $type="overlay"
          $={(self) => (toast = self)}
          class="viewer-toast"
          halign={Gtk.Align.CENTER}
          valign={Gtk.Align.START}
          marginTop={72}
          visible={false}
        />
      </overlay>
    </Gtk.ApplicationWindow>
  ) as Gtk.ApplicationWindow

  win.connect("close-request", () => {
    if (toastTimeout) GLib.source_remove(toastTimeout)
    return false
  })

  renderHistory()
  syncSortButtons()
  // A file opens in the full-screen view straight away; the scan selects it
  // in the grid when it turns up.
  if (file) {
    preview.setFile(file)
    stack.visibleChildName = "preview"
  }
  if (!openDirectory(directory, file)) openDirectory(GLib.get_home_dir())

  return win
}
