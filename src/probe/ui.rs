//! The UI check (VITRINE_PROBE=ui).

use super::*;

/// VITRINE_PROBE=ui: drives the grid's chrome (sorting, subfolders, rescan,
/// the folder entry and its history, the empty state) on a small folder and
/// prints what it sees, as `RESULT ui …` lines. Only in a folder holding a
/// `.vitrine-probe-scratch` file does it add and remove files (for rescan).
pub fn run(window: &gtk::ApplicationWindow) {
    let window = window.clone();
    glib::spawn_future_local(async move {
        let root: gtk::Widget = window.clone().upcast();
        let start = glib::monotonic_time();
        let Some(grid) = wait_for_grid(&root).await else {
            println!("RESULT ui no_grid");
            return finish(&window);
        };
        wait_for_scan(&grid, start).await;
        let selection = grid
            .model()
            .and_downcast::<gtk::SingleSelection>()
            .expect("the grid's model is a SingleSelection");
        let name_at = |i: u32| {
            selection
                .item(i)
                .and_downcast::<glib::BoxedAnyObject>()
                .map(|object| {
                    let image = object.borrow::<crate::library::Image>();
                    image
                        .path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                })
                .unwrap_or_default()
        };
        let state = |what: &str| {
            let names: Vec<String> = (0..selection.n_items().min(4)).map(name_at).collect();
            let mut labels = Vec::new();
            find_all::<gtk::Label>(&root, &mut labels);
            let info: Vec<String> = labels
                .iter()
                .filter(|label| {
                    label.is_drawable()
                        && label
                            .ancestor(gtk::Box::static_type())
                            .is_some_and(|parent| {
                                parent.has_css_class("viewer-info")
                                    || parent
                                        .parent()
                                        .is_some_and(|p| p.has_css_class("viewer-info"))
                                    || parent.has_css_class("viewer-empty")
                            })
                })
                .map(|label| label.label().to_string())
                .collect();
            let empty = labels
                .iter()
                .find(|label| label.has_css_class("viewer-empty") && label.is_visible())
                .map(|label| label.label().to_string());
            println!(
                "RESULT ui {what} page={:?} items={} first={names:?} selected={:?} title={:?} info={info:?} empty={empty:?}",
                find::<gtk::Stack>(&root)
                    .and_then(|stack| stack.visible_child_name())
                    .unwrap_or_default(),
                selection.n_items(),
                name_at(selection.selected()),
                window.title().unwrap_or_default(),
            );
        };
        sleep(500).await;
        state("load");
        shot(&window, "ui-grid");

        selection.set_selected(4);
        sleep(300).await;
        state("select_5th");
        for label in ["Date", "Size", "Random", "Random", "Name"] {
            if let Some(button) = button(&root, label) {
                button.emit_clicked();
            }
            sleep(200).await;
            state(&format!("sort_{label}"));
        }
        if let Some(button) = button(&root, "↑") {
            button.emit_clicked();
            sleep(200).await;
            state("direction");
            button.emit_clicked();
        }

        // The ⋯ menu's actions, the clipboard, the toast, the shortcuts
        // window, and the menu in the view.
        {
            let enabled = |name: &str| {
                window
                    .lookup_action(name)
                    .is_some_and(|action| action.is_enabled())
            };
            let toast = || {
                let mut boxes = Vec::new();
                find_all::<gtk::Box>(&root, &mut boxes);
                boxes
                    .iter()
                    .find(|b| b.has_css_class("viewer-toast"))
                    .filter(|b| b.is_visible())
                    .and_then(|b| b.first_child().and_downcast::<gtk::Label>())
                    .map(|label| label.label().to_string())
            };
            println!(
                "RESULT ui menu_grid copy={} rotate={}",
                enabled("copy-image"),
                enabled("rotate-left")
            );
            let clipboard = window.clipboard();
            WidgetExt::activate_action(&window, "win.copy-path", None).ok();
            let text = clipboard.read_text_future().await.ok().flatten();
            println!("RESULT ui copy_path text={text:?} toast={:?}", toast());
            WidgetExt::activate_action(&window, "win.copy-image", None).ok();
            sleep(1000).await;
            let texture = clipboard.read_texture_future().await.ok().flatten();
            println!(
                "RESULT ui copy_image size={:?} toast={:?}",
                texture.map(|t| (t.width(), t.height())),
                toast()
            );
            sleep(2200).await;
            println!("RESULT ui toast_gone toast={:?}", toast());

            let mut menus = Vec::new();
            find_all::<gtk::MenuButton>(&root, &mut menus);
            if let Some(menu) = menus
                .iter()
                .filter(|m| m.tooltip_text().as_deref() == Some("More actions"))
                .find(|m| m.has_css_class("viewer-toolbar-menu"))
            {
                menu.popup();
                sleep(300).await;
                shot(&window, "ui-menu");
                menu.popdown();
            }

            // The wallpaper command (VITRINE_WALLPAPER_COMMAND), in the menu only.
            println!(
                "RESULT ui wallpaper action={}",
                window.lookup_action("set-wallpaper").is_some()
            );
            if window.lookup_action("set-wallpaper").is_some() {
                press(&window, gdk::Key::w);
                sleep(1000).await;
                println!("RESULT ui wallpaper_set toast={:?}", toast());
            }

            WidgetExt::activate_action(&window, "win.shortcuts", None).ok();
            sleep(500).await;
            let shortcuts = gtk::Window::list_toplevels()
                .into_iter()
                .filter_map(|w| w.downcast::<gtk::Window>().ok())
                .find(|w| w.has_css_class("viewer-shortcuts"));
            println!("RESULT ui shortcuts open={}", shortcuts.is_some());
            if let Some(shortcuts) = shortcuts {
                shot(&window, "ui-shortcuts");
                let controllers = shortcuts.observe_controllers();
                for i in 0..controllers.n_items() {
                    if let Some(keys) = controllers
                        .item(i)
                        .and_downcast::<gtk::EventControllerKey>()
                    {
                        keys.emit_by_name::<bool>(
                            "key-pressed",
                            &[&gdk::Key::q.into_glib(), &0u32, &gdk::ModifierType::empty()],
                        );
                    }
                }
                sleep(300).await;
                println!(
                    "RESULT ui shortcuts_closed visible={}",
                    shortcuts.is_visible()
                );
            }

            grid.emit_by_name::<()>("activate", &[&0u32]);
            sleep(800).await;
            println!(
                "RESULT ui menu_view copy={} rotate={}",
                enabled("copy-image"),
                enabled("rotate-left")
            );
            if let Some(menu) = menus
                .iter()
                .filter(|m| m.tooltip_text().as_deref() == Some("More actions"))
                .find(|m| !m.has_css_class("viewer-toolbar-menu"))
            {
                menu.popup();
                sleep(300).await;
                shot(&window, "ui-view-menu");
                menu.popdown();
            }
            // Colour assessment: b, the image decoded for the area inside the
            // border, the menu's check; b again.
            let assessment = |what: &str| {
                let image = find::<crate::view::zoomable::ZoomableImage>(window.upcast_ref());
                let checked = window
                    .lookup_action("color-assessment")
                    .and_then(|action| action.state())
                    .and_then(|state| state.get::<bool>());
                let texture = image
                    .as_ref()
                    .and_then(|image| image.base_texture())
                    .map(|texture| (texture.width(), texture.height()));
                let scale = window.surface().map_or(1.0, |surface| surface.scale());
                println!(
                    "RESULT ui {what} on={:?} checked={checked:?} texture={texture:?} view={:?}",
                    image.map(|image| image.assessment()),
                    (
                        (window.width() as f64 * scale).round(),
                        (window.height() as f64 * scale).round()
                    ),
                );
            };
            press(&window, gdk::Key::b);
            sleep(1000).await;
            assessment("assessment_on");
            shot(&window, "ui-assessment");
            if let Some(menu) = menus
                .iter()
                .filter(|m| m.tooltip_text().as_deref() == Some("More actions"))
                .find(|m| !m.has_css_class("viewer-toolbar-menu"))
            {
                menu.popup();
                sleep(300).await;
                shot(&window, "ui-assessment-menu");
                menu.popdown();
            }
            for _ in 0..4 {
                press(&window, gdk::Key::plus);
            }
            sleep(1000).await;
            shot(&window, "ui-assessment-zoomed");
            press(&window, gdk::Key::_0);
            press(&window, gdk::Key::b);
            sleep(1000).await;
            assessment("assessment_off");
            press(&window, gdk::Key::Escape);
            sleep(500).await;
        }

        let count = selection.n_items();
        if let Some(subfolders) = button(&root, "Subfolders").and_downcast::<gtk::ToggleButton>() {
            subfolders.set_active(true);
            wait_until(3000, || selection.n_items() != count).await;
            sleep(500).await;
            state("subfolders_on");
            shot(&window, "ui-subfolders");
        }

        let directory = crate::library::image(&selection.item(0).unwrap())
            .path
            .clone();
        let directory = directory.parent().unwrap().to_owned();
        let directory = if directory.ends_with("sub") {
            directory.parent().unwrap().to_owned()
        } else {
            directory
        };
        if directory.join(".vitrine-probe-scratch").exists() {
            let added = directory.join("img-00-added.jpg");
            let removed = directory.join("img-02-orange.jpg");
            let _ = std::fs::copy(directory.join("img-01-red.jpg"), &added);
            let kept = directory.join("img-02-kept.jpg");
            let _ = std::fs::rename(&removed, &kept);
            selection.set_selected(2);
            sleep(200).await;
            state("before_rescan");
            press(&window, gdk::Key::r);
            sleep(1000).await;
            state("rescan");
            let _ = std::fs::remove_file(&added);
            let _ = std::fs::rename(&kept, &removed);
            press(&window, gdk::Key::r);
            sleep(1000).await;
            state("rescan_back");

            // Watching: the same without pressing r.
            let count = selection.n_items();
            let watched = directory.join("img-00-watched.jpg");
            let _ = std::fs::copy(directory.join("img-01-red.jpg"), &watched);
            let added = wait_until(5000, || selection.n_items() == count + 1).await;
            let _ = std::fs::remove_file(&watched);
            let removed = wait_until(5000, || selection.n_items() == count).await;
            println!("RESULT ui watch added_ms={added} removed_ms={removed}");

            // In the view: a change elsewhere keeps the zoom; a change to the
            // image shown shows the new version.
            let view = find::<crate::view::zoomable::ZoomableImage>(&root).expect("the view");
            let base_width = || view.base_texture().map_or(0, |texture| texture.width());
            let gold = directory.join("img-03-gold.jpg");
            let position = (0..selection.n_items())
                .find(|&i| name_at(i) == "img-03-gold.jpg")
                .unwrap_or(0);
            grid.emit_by_name::<()>("activate", &[&position]);
            sleep(800).await;
            press(&window, gdk::Key::plus);
            sleep(200).await;
            let zoomed = view.scale();
            let _ = std::fs::copy(directory.join("img-01-red.jpg"), &watched);
            wait_until(5000, || selection.n_items() == count + 1).await;
            sleep(300).await;
            println!(
                "RESULT ui watch_elsewhere zoom_kept={} shown={:?}",
                (view.scale() - zoomed).abs() < 1e-9,
                name_at(selection.selected())
            );
            let before = base_width();
            let backup = directory.join(".img-03-gold.backup");
            let mtime = std::fs::metadata(&gold).and_then(|m| m.modified()).ok();
            let _ = std::fs::copy(&gold, &backup);
            let _ = std::fs::copy(directory.join("img-10-gray.jpg"), &gold);
            let reloaded = wait_until(5000, || base_width() != before).await;
            println!(
                "RESULT ui watch_shown reloaded_ms={reloaded} width={before}->{} shown={:?}",
                base_width(),
                name_at(selection.selected())
            );
            shot(&window, "ui-watch-shown");
            let _ = std::fs::rename(&backup, &gold);
            if let (Some(mtime), Ok(file)) =
                (mtime, std::fs::File::options().write(true).open(&gold))
            {
                let _ = file.set_modified(mtime);
            }
            let _ = std::fs::remove_file(&watched);
            wait_until(5000, || selection.n_items() == count).await;
            sleep(1500).await;
            press(&window, gdk::Key::Escape);
            sleep(300).await;
            state("watch_back");

            // Trash and undo: they trash files for real, so only when asked
            // (VITRINE_PROBE_TRASH=1, on a private bus with its own trash:
            // bench/README.md).
            if std::env::var_os("VITRINE_PROBE_TRASH").is_some() {
                let toast = || {
                    let mut boxes = Vec::new();
                    find_all::<gtk::Box>(&root, &mut boxes);
                    boxes
                        .iter()
                        .find(|b| b.has_css_class("viewer-toast"))
                        .filter(|b| b.is_visible())
                        .and_then(|b| b.first_child().and_downcast::<gtk::Label>())
                        .map(|label| label.label().to_string())
                };
                let select_named = |name: &str| {
                    let position = (0..selection.n_items()).find(|&i| name_at(i) == name);
                    if let Some(position) = position {
                        selection.set_selected(position);
                    }
                    position
                };
                let undo = || press_with(&window, gdk::Key::z, gdk::ModifierType::CONTROL_MASK);
                let teal = directory.join("img-05-teal.jpg");
                let count = selection.n_items();
                println!(
                    "RESULT ui trash available={}",
                    crate::desktop::trash::available()
                );
                select_named("img-05-teal.jpg");
                sleep(200).await;
                press(&window, gdk::Key::Delete);
                wait_until(5000, || {
                    toast().is_some_and(|t| t.starts_with("Moved") || t.contains("GVfs"))
                })
                .await;
                println!(
                    "RESULT ui delete items={} selected={:?} on_disk={} toast={:?}",
                    selection.n_items(),
                    name_at(selection.selected()),
                    teal.exists(),
                    toast()
                );
                shot(&window, "ui-toast-undo");
                if crate::desktop::trash::available() {
                    undo();
                    wait_until(5000, || toast().is_some_and(|t| t.starts_with("Restored"))).await;
                    println!(
                        "RESULT ui undo items={} selected={:?} on_disk={} toast={:?}",
                        selection.n_items(),
                        name_at(selection.selected()),
                        teal.exists(),
                        toast()
                    );

                    // The same path twice, quickly: undo restores the newer one;
                    // the older can't come back over it.
                    select_named("img-05-teal.jpg");
                    let first = glib::monotonic_time();
                    press(&window, gdk::Key::Delete);
                    wait_until(5000, || !teal.exists()).await;
                    let _ = std::fs::copy(directory.join("img-10-gray.jpg"), &teal);
                    press(&window, gdk::Key::r);
                    wait_until(3000, || select_named("img-05-teal.jpg").is_some()).await;
                    let apart = (glib::monotonic_time() - first) / 1000;
                    press(&window, gdk::Key::Delete);
                    wait_until(5000, || !teal.exists()).await;
                    sleep(500).await;
                    println!("RESULT ui delete_twice apart_ms={apart}");
                    undo();
                    wait_until(5000, || teal.exists()).await;
                    sleep(300).await;
                    println!(
                        "RESULT ui undo_newer size={:?} toast={:?}",
                        std::fs::metadata(&teal).map(|m| m.len()).ok(),
                        toast()
                    );
                    undo();
                    sleep(1000).await;
                    println!("RESULT ui undo_older toast={:?}", toast());
                    // Put the original back by hand.
                    let _ = std::fs::remove_file(&teal);
                    let trash = gio::File::for_uri("trash:///");
                    if let Ok(enumerator) = trash.enumerate_children(
                        "standard::name,trash::orig-path",
                        gio::FileQueryInfoFlags::NONE,
                        None::<&gio::Cancellable>,
                    ) {
                        for info in enumerator.flatten() {
                            if info.attribute_byte_string("trash::orig-path").as_deref()
                                == teal.to_str()
                            {
                                let _ = trash.child(info.name()).move_(
                                    &gio::File::for_path(&teal),
                                    gio::FileCopyFlags::NONE,
                                    None::<&gio::Cancellable>,
                                    None,
                                );
                            }
                        }
                    }
                    println!(
                        "RESULT ui original_back size={:?}",
                        std::fs::metadata(&teal).map(|m| m.len()).ok()
                    );
                    sleep(1500).await;

                    // Undo once the item has left the trash (emptied): said
                    // so, and nothing restored.
                    let gone = directory.join("img-00-gone.jpg");
                    let _ = std::fs::copy(directory.join("img-01-red.jpg"), &gone);
                    press(&window, gdk::Key::r);
                    wait_until(3000, || select_named("img-00-gone.jpg").is_some()).await;
                    press(&window, gdk::Key::Delete);
                    wait_until(5000, || toast().is_some_and(|t| t.starts_with("Moved"))).await;
                    let trash = gio::File::for_uri("trash:///");
                    if let Ok(enumerator) = trash.enumerate_children(
                        "standard::name,trash::orig-path",
                        gio::FileQueryInfoFlags::NONE,
                        None::<&gio::Cancellable>,
                    ) {
                        for info in enumerator.flatten() {
                            if info.attribute_byte_string("trash::orig-path").as_deref()
                                == gone.to_str()
                            {
                                let _ = trash.child(info.name()).delete(None::<&gio::Cancellable>);
                            }
                        }
                    }
                    undo();
                    wait_until(5000, || toast().is_some_and(|t| t.starts_with("Couldn't"))).await;
                    println!(
                        "RESULT ui undo_gone toast={:?} on_disk={} items={}",
                        toast(),
                        gone.exists(),
                        selection.n_items()
                    );
                    sleep(1500).await;

                    // In the view: deleting shows the next image, undo the
                    // restored one.
                    select_named("img-06-blue.jpg");
                    grid.emit_by_name::<()>("activate", &[&selection.selected()]);
                    sleep(500).await;
                    press(&window, gdk::Key::Delete);
                    wait_until(5000, || toast().is_some_and(|t| t.starts_with("Moved"))).await;
                    let after_delete = shown_name(&root);
                    undo();
                    wait_until(5000, || toast().is_some_and(|t| t.starts_with("Restored"))).await;
                    println!(
                        "RESULT ui view_delete shown={after_delete:?} after_undo={:?} selected={:?} items={}",
                        shown_name(&root),
                        name_at(selection.selected()),
                        selection.n_items()
                    );
                    press(&window, gdk::Key::Escape);
                    sleep(300).await;
                }
                println!(
                    "RESULT ui trash_back items={} expected={count}",
                    selection.n_items()
                );
            }
        }

        let entry = find::<gtk::Entry>(&root).expect("the folder entry");
        entry.set_text("/nonexistent");
        entry.emit_activate();
        println!(
            "RESULT ui bad_folder error={}",
            entry.has_css_class("error")
        );
        entry.grab_focus();
        press(&window, gdk::Key::Escape);
        println!(
            "RESULT ui escape_resets text={:?} error={}",
            entry.text(),
            entry.has_css_class("error")
        );
        entry.set_text(&format!("{}/sub", directory.display()));
        entry.emit_activate();
        sleep(800).await;
        state("open_sub");
        entry.emit_by_name::<()>("icon-release", &[&gtk::EntryIconPosition::Secondary]);
        sleep(300).await;
        if let Some(panel) = find::<gtk::Popover>(entry.upcast_ref()) {
            let mut buttons = Vec::new();
            find_all::<gtk::Button>(panel.upcast_ref(), &mut buttons);
            let focused = buttons.iter().position(|b| b.has_focus() || b.is_focus());
            println!(
                "RESULT ui history visible={} entries={} focused={focused:?}",
                panel.is_visible(),
                buttons.len()
            );
            shot(&window, "ui-history");
            // (Esc is the popover's own shortcut, which a probe can't press.)
            panel.popdown();
            println!(
                "RESULT ui history_closed visible={} entry_focused={}",
                panel.is_visible(),
                gtk::prelude::GtkWindowExt::focus(&window)
                    .is_some_and(|focus| focus.is_ancestor(&entry)
                        || focus == *entry.upcast_ref::<gtk::Widget>())
            );
        }
        // Properties: the i popover in the grid and the view, on a photo with
        // EXIF and a PNG without.
        let props = directory.join("props");
        if props.is_dir() {
            entry.set_text(&props.to_string_lossy());
            entry.emit_activate();
            sleep(500).await;
            let rows = |view: bool| {
                let mut buttons = Vec::new();
                find_all::<gtk::MenuButton>(&root, &mut buttons);
                let button = buttons
                    .into_iter()
                    .filter(|b| b.tooltip_text().as_deref() == Some("Image properties (i)"))
                    .find(|b| b.has_css_class("viewer-toolbar-menu") != view)?;
                let popover = button.popover()?;
                let mut labels = Vec::new();
                find_all::<gtk::Label>(popover.upcast_ref(), &mut labels);
                Some(
                    labels
                        .iter()
                        .map(|label| label.label().to_string())
                        .collect::<Vec<_>>()
                        .join(" | "),
                )
            };
            for (name, view) in [
                ("camera.jpg", false),
                ("plain.png", false),
                ("camera.jpg", true),
            ] {
                let position = (0..selection.n_items())
                    .find(|&i| name_at(i) == name)
                    .unwrap_or(0);
                selection.set_selected(position);
                if view {
                    grid.emit_by_name::<()>("activate", &[&position]);
                    sleep(500).await;
                }
                press(&window, gdk::Key::i);
                sleep(600).await;
                println!(
                    "RESULT ui properties {name} view={view} rows={:?}",
                    rows(view)
                );
                state(&format!("properties_{name}_view={view}"));
                shot(
                    &window,
                    &format!(
                        "ui-properties-{}{}",
                        name.replace('.', "-"),
                        if view { "-view" } else { "" }
                    ),
                );
                press(&window, gdk::Key::i);
                sleep(200).await;
                if view {
                    press(&window, gdk::Key::Escape);
                    sleep(300).await;
                }
            }
            // EXIF orientation outside JPEG, when present: the info bar's
            // size and the view's texture should both be turned.
            for name in ["rotated.png", "rotated.webp"] {
                let Some(position) = (0..selection.n_items()).find(|&i| name_at(i) == name) else {
                    continue;
                };
                selection.set_selected(position);
                sleep(500).await;
                state(&format!("orientation_{name}"));
                grid.emit_by_name::<()>("activate", &[&position]);
                sleep(800).await;
                let texture = find::<crate::view::zoomable::ZoomableImage>(&root)
                    .and_then(|image| image.base_texture())
                    .map(|texture| (texture.width(), texture.height()));
                println!("RESULT ui orientation {name} texture={texture:?}");
                press(&window, gdk::Key::Escape);
                sleep(300).await;
            }
        }
        // Deleting the last image in the view goes back to the (empty) grid.
        let single = directory.join("single");
        if single.is_dir()
            && directory.join(".vitrine-probe-scratch").exists()
            && std::env::var_os("VITRINE_PROBE_TRASH").is_some()
            && crate::desktop::trash::available()
        {
            entry.set_text(&single.to_string_lossy());
            entry.emit_activate();
            sleep(500).await;
            grid.emit_by_name::<()>("activate", &[&0u32]);
            sleep(500).await;
            press(&window, gdk::Key::Delete);
            sleep(1500).await;
            state("delete_last");
            println!("RESULT ui delete_last_toast {:?}", toast_text(&root));
            press_with(&window, gdk::Key::z, gdk::ModifierType::CONTROL_MASK);
            sleep(1500).await;
            state("undo_last");
            println!("RESULT ui undo_last_toast {:?}", toast_text(&root));
        }
        let empty = directory.join("empty");
        if empty.is_dir() {
            entry.set_text(&empty.to_string_lossy());
            entry.emit_activate();
            sleep(500).await;
            state("empty");
            shot(&window, "ui-empty");
            entry.emit_by_name::<()>("icon-release", &[&gtk::EntryIconPosition::Secondary]);
            sleep(300).await;
            shot(&window, "ui-empty-history");
            if let Some(panel) = find::<gtk::Popover>(entry.upcast_ref()) {
                panel.popdown();
            }
        }
        idle(&window).await;
        finish(&window);
    });
}
