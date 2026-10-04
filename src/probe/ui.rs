//! The UI check (VITRINE_PROBE=ui).

use super::*;

/// VITRINE_PROBE=ui: drives the grid's chrome (sorting, subfolders, rescan,
/// the folder entry, the open choosers, the empty state) on a small folder and
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
            let labels = find_all::<gtk::Label>(&root);
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

        // A thumbnail's context menu: a right click on a tile that isn't
        // selected (its gesture's handler), then the Menu key.
        sleep(300).await;
        if let Some(menu) = find::<gtk::PopoverMenu>(grid.upcast_ref()) {
            let before = selection.selected();
            let click = find_all::<gtk::AspectFrame>(grid.upcast_ref())
                .into_iter()
                .filter_map(|frame| frame.parent())
                .filter(|cell| {
                    cell.parent()
                        .is_some_and(|item| !item.state_flags().contains(gtk::StateFlags::SELECTED))
                })
                .find_map(|cell| {
                    let controllers = cell.observe_controllers();
                    (0..controllers.n_items())
                        .find_map(|i| controllers.item(i).and_downcast::<gtk::GestureClick>())
                });
            if let Some(click) = click {
                click.emit_by_name::<()>("pressed", &[&1i32, &20f64, &20f64]);
            }
            sleep(300).await;
            let labels: Vec<String> = find_all::<gtk::Label>(menu.upcast_ref())
                .iter()
                .map(|label| label.label().to_string())
                .collect();
            println!(
                "RESULT ui context_menu open={} moved={} selected={:?} at={:?} items={labels:?}",
                menu.is_visible(),
                selection.selected() != before,
                name_at(selection.selected()),
                menu.pointing_to().1,
            );
            shot(&window, "ui-context-menu");
            menu.popdown();
            sleep(200).await;
            press(&window, gdk::Key::Menu);
            sleep(300).await;
            println!(
                "RESULT ui context_menu_key open={} selected={:?} at={:?}",
                menu.is_visible(),
                name_at(selection.selected()),
                menu.pointing_to().1,
            );
            menu.popdown();
            selection.set_selected(before);
            sleep(200).await;
        } else {
            println!("RESULT ui context_menu missing");
        }

        // The ⋯ menu's actions, the clipboard, the toast, the shortcuts
        // window, and the menu in the view.
        {
            let enabled = |name: &str| {
                window
                    .lookup_action(name)
                    .is_some_and(|action| action.is_enabled())
            };
            println!(
                "RESULT ui menu_grid copy={} rotate={}",
                enabled("copy-image"),
                enabled("rotate-left")
            );
            let clipboard = window.clipboard();
            WidgetExt::activate_action(&window, "win.copy-path", None).ok();
            let text = clipboard.read_text_future().await.ok().flatten();
            println!(
                "RESULT ui copy_path text={text:?} toast={:?}",
                toast_text(&root)
            );
            WidgetExt::activate_action(&window, "win.copy-image", None).ok();
            sleep(1000).await;
            let texture = clipboard.read_texture_future().await.ok().flatten();
            println!(
                "RESULT ui copy_image size={:?} toast={:?}",
                texture.map(|t| (t.width(), t.height())),
                toast_text(&root)
            );
            sleep(2200).await;
            println!("RESULT ui toast_gone toast={:?}", toast_text(&root));

            menu_shot(&window, false, "ui-menu").await;

            // The wallpaper command (VITRINE_WALLPAPER_COMMAND), in the menu only.
            println!(
                "RESULT ui wallpaper action={}",
                window.lookup_action("set-wallpaper").is_some()
            );
            if window.lookup_action("set-wallpaper").is_some() {
                press(&window, gdk::Key::w);
                sleep(1000).await;
                println!("RESULT ui wallpaper_set toast={:?}", toast_text(&root));
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
            menu_shot(&window, true, "ui-view-menu").await;
            // The view's context menu: a right click on the image (its
            // gesture's handler), then the Menu key.
            let image = find::<crate::view::zoomable::ZoomableImage>(&root);
            let menu = image
                .as_ref()
                .and_then(|image| find::<gtk::PopoverMenu>(image.upcast_ref()));
            if let (Some(image), Some(menu)) = (image, menu) {
                let controllers = image.observe_controllers();
                let click = (0..controllers.n_items())
                    .filter_map(|i| controllers.item(i).and_downcast::<gtk::GestureClick>())
                    .find(|click| click.button() == gdk::BUTTON_SECONDARY);
                if let Some(click) = click {
                    click.emit_by_name::<()>("pressed", &[&1i32, &300f64, &200f64]);
                }
                sleep(300).await;
                let labels: Vec<String> = find_all::<gtk::Label>(menu.upcast_ref())
                    .iter()
                    .map(|label| label.label().to_string())
                    .collect();
                println!(
                    "RESULT ui view_context_menu open={} at={:?} items={labels:?}",
                    menu.is_visible(),
                    menu.pointing_to().1,
                );
                shot(&window, "ui-view-context-menu");
                menu.popdown();
                sleep(200).await;
                press(&window, gdk::Key::Menu);
                sleep(300).await;
                println!(
                    "RESULT ui view_context_menu_key open={} at={:?}",
                    menu.is_visible(),
                    menu.pointing_to().1,
                );
                menu.popdown();
                sleep(200).await;
                // Its Properties item opens the i button's popover.
                press(&window, gdk::Key::Menu);
                sleep(300).await;
                let item = find_all::<gtk::Label>(menu.upcast_ref())
                    .into_iter()
                    .find(|label| label.label() == "Properties")
                    .and_then(|label| {
                        let mut widget = label.parent();
                        while let Some(parent) = &widget {
                            if parent.css_name() == "modelbutton" {
                                break;
                            }
                            widget = parent.parent();
                        }
                        widget
                    });
                if let Some(item) = item {
                    item.emit_by_name::<()>("clicked", &[]);
                }
                sleep(400).await;
                let properties = find_all::<gtk::MenuButton>(&root)
                    .into_iter()
                    .find(|button| button.is_active());
                println!(
                    "RESULT ui view_context_menu_properties menu_open={} properties_open={} focus_on_image={}",
                    menu.is_visible(),
                    properties.is_some(),
                    gtk::prelude::RootExt::focus(&window).is_some_and(|focus| focus == image),
                );
                if let Some(properties) = properties {
                    properties.set_active(false);
                }
                sleep(200).await;
            } else {
                println!("RESULT ui view_context_menu missing");
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
            menu_shot(&window, true, "ui-assessment-menu").await;
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
            // (VITRINE_PROBE_TRASH=1, with its own XDG_DATA_HOME on this
            // folder's filesystem: bench/README.md).
            if std::env::var_os("VITRINE_PROBE_TRASH").is_some() {
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
                select_named("img-05-teal.jpg");
                sleep(200).await;
                press(&window, gdk::Key::Delete);
                wait_until(5000, || {
                    toast_text(&root)
                        .is_some_and(|t| t.starts_with("Moved") || t.starts_with("Could"))
                })
                .await;
                println!(
                    "RESULT ui delete items={} selected={:?} on_disk={} toast={:?}",
                    selection.n_items(),
                    name_at(selection.selected()),
                    teal.exists(),
                    toast_text(&root)
                );
                shot(&window, "ui-toast-undo");
                undo();
                wait_until(5000, || {
                    toast_text(&root).is_some_and(|t| t.starts_with("Restored"))
                })
                .await;
                println!(
                    "RESULT ui undo items={} selected={:?} on_disk={} toast={:?}",
                    selection.n_items(),
                    name_at(selection.selected()),
                    teal.exists(),
                    toast_text(&root)
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
                    toast_text(&root)
                );
                undo();
                sleep(1000).await;
                println!("RESULT ui undo_older toast={:?}", toast_text(&root));
                // Put the original back by hand.
                let _ = std::fs::remove_file(&teal);
                for item in trashed_from(&teal) {
                    let _ = trash::os_limited::restore_all([item]);
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
                wait_until(5000, || {
                    toast_text(&root).is_some_and(|t| t.starts_with("Moved"))
                })
                .await;
                let _ = trash::os_limited::purge_all(trashed_from(&gone));
                undo();
                wait_until(5000, || {
                    toast_text(&root).is_some_and(|t| t.starts_with("Couldn't"))
                })
                .await;
                println!(
                    "RESULT ui undo_gone toast={:?} on_disk={} items={}",
                    toast_text(&root),
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
                wait_until(5000, || {
                    toast_text(&root).is_some_and(|t| t.starts_with("Moved"))
                })
                .await;
                let after_delete = shown_name(&root);
                undo();
                wait_until(5000, || {
                    toast_text(&root).is_some_and(|t| t.starts_with("Restored"))
                })
                .await;
                println!(
                    "RESULT ui view_delete shown={after_delete:?} after_undo={:?} selected={:?} items={}",
                    shown_name(&root),
                    name_at(selection.selected()),
                    selection.n_items()
                );
                press(&window, gdk::Key::Escape);
                sleep(300).await;
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
            "RESULT ui bad_folder error={} toast={:?}",
            entry.has_css_class("error"),
            toast_text(&root)
        );
        press(&window, gdk::Key::Escape);
        press_with(&window, gdk::Key::l, gdk::ModifierType::CONTROL_MASK);
        println!(
            "RESULT ui ctrl_l entry_focused={}",
            entry.has_focus() || entry.first_child().is_some_and(|text| text.has_focus())
        );
        press_with(&window, gdk::Key::o, gdk::ModifierType::CONTROL_MASK);
        sleep(500).await;
        let choosers: Vec<gtk::Window> = gtk::Window::list_toplevels()
            .iter()
            .filter_map(|w| w.downcast_ref::<gtk::Window>())
            .filter(|w| w.is_visible() && w.transient_for().is_some_and(|parent| parent == window))
            .cloned()
            .collect();
        println!("RESULT ui ctrl_o_from_entry choosers={}", choosers.len());
        for chooser in &choosers {
            chooser.close();
        }
        sleep(300).await;
        entry.grab_focus();
        press(&window, gdk::Key::Escape);
        println!(
            "RESULT ui escape_resets text={:?} error={}",
            entry.text(),
            entry.has_css_class("error")
        );
        // Typing after a bad path clears the error; leaving the entry puts
        // the folder back.
        entry.set_text("/nonexistent");
        entry.emit_activate();
        entry.grab_focus();
        entry.set_text("/nonexisten");
        println!(
            "RESULT ui typing_clears_error error={}",
            entry.has_css_class("error")
        );
        grid.grab_focus();
        sleep(100).await;
        println!("RESULT ui leaving_resets text={:?}", entry.text());
        entry.set_text(&format!("{}/sub", directory.display()));
        entry.emit_activate();
        sleep(800).await;
        state("open_sub");
        // The open buttons (the grid's and the view's) and Ctrl+O,
        // Ctrl+Shift+O: a chooser each (GTK's own window here; none to count
        // where a portal shows it), which a probe can only close.
        let open: Vec<gtk::MenuButton> = find_all::<gtk::MenuButton>(&root)
            .into_iter()
            .filter(|b| {
                b.tooltip_text()
                    .is_some_and(|tip| tip.starts_with("Open a folder"))
            })
            .collect();
        println!("RESULT ui open_buttons found={}", open.len());
        if let Some(button) = open.iter().find(|b| b.is_mapped()) {
            button.popup();
            sleep(300).await;
            let items: Vec<String> = button
                .popover()
                .map(|popover| find_all::<gtk::Label>(popover.upcast_ref()))
                .unwrap_or_default()
                .iter()
                .map(|label| label.label().to_string())
                .collect();
            println!("RESULT ui open_menu items={items:?}");
            shot(&window, "ui-open-menu");
            button.popdown();
        }
        for (what, state_) in [
            ("folder", gdk::ModifierType::CONTROL_MASK),
            (
                "image",
                gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
            ),
        ] {
            press_with(&window, gdk::Key::o, state_);
            sleep(500).await;
            let choosers: Vec<gtk::Window> = gtk::Window::list_toplevels()
                .iter()
                .filter_map(|w| w.downcast_ref::<gtk::Window>())
                .filter(|w| {
                    w.is_visible() && w.transient_for().is_some_and(|parent| parent == window)
                })
                .cloned()
                .collect();
            let titles: Vec<String> = choosers
                .iter()
                .filter_map(|w| w.title().map(Into::into))
                .collect();
            for chooser in &choosers {
                chooser.close();
            }
            sleep(300).await;
            println!(
                "RESULT ui choose what={what} choosers={titles:?} text={:?} error={}",
                entry.text(),
                entry.has_css_class("error")
            );
        }
        // What a chosen path opens (through "win.open-path", as the choosers
        // can't be driven): in the grid, an image's folder with it selected;
        // in the view, the image, or a folder's first.
        let choose = |path: &Path| {
            WidgetExt::activate_action(&window, "win.open-path", Some(&path.to_variant())).ok();
        };
        // The view's info: name, resolution, position.
        let shown = || {
            find_all::<gtk::Box>(&root)
                .into_iter()
                .find(|b| b.has_css_class("preview-image-info"))
                .map(|info| {
                    find_all::<gtk::Label>(info.upcast_ref())
                        .iter()
                        .filter(|label| label.is_visible())
                        .map(|label| label.label().to_string())
                        .collect::<Vec<_>>()
                })
        };
        if let Some(third) = selection.item(2) {
            let third = crate::library::image(&third).path.clone();
            choose(&directory);
            sleep(800).await;
            choose(&third);
            sleep(800).await;
            state("grid_chose_image");
            press(&window, gdk::Key::e);
            sleep(500).await;
            choose(&directory);
            sleep(1000).await;
            state("view_chose_folder");
            window.set_state_flags(gtk::StateFlags::DROP_ACTIVE, false);
            sleep(300).await;
            shot(&window, "ui-drop-active");
            window.unset_state_flags(gtk::StateFlags::DROP_ACTIVE);
            println!("RESULT ui view_chose_folder shown={:?}", shown());
            // The controls fade in a window too (not only in fullscreen).
            sleep(2500).await;
            let hidden = find_all::<gtk::Box>(&root)
                .iter()
                .filter(|b| b.has_css_class("preview-controls") && b.has_css_class("hidden"))
                .count();
            println!(
                "RESULT ui view_idle fullscreen={} hidden_controls={hidden}",
                window.is_fullscreen()
            );
            shot(&window, "ui-view-idle");
            choose(&third);
            sleep(1000).await;
            state("view_chose_image");
            println!("RESULT ui view_chose_image shown={:?}", shown());
            let empty = directory.join("empty");
            if empty.is_dir() {
                choose(&empty);
                sleep(800).await;
                state("view_chose_empty");
            } else {
                press(&window, gdk::Key::Escape);
            }
            choose(&directory.join("no-such.txt"));
            println!("RESULT ui chose_other toast={:?}", toast_text(&root));
            sleep(300).await;
        }
        // Properties: the i popover in the grid and the view, on a photo with
        // EXIF and a PNG without.
        let props = directory.join("props");
        if props.is_dir() {
            entry.set_text(&props.to_string_lossy());
            entry.emit_activate();
            sleep(500).await;
            let rows = |view: bool| {
                let buttons = find_all::<gtk::MenuButton>(&root);
                let button = buttons
                    .into_iter()
                    .filter(|b| b.tooltip_text().as_deref() == Some("Image properties (i)"))
                    .find(|b| b.has_css_class("viewer-toolbar-menu") != view)?;
                let popover = button.popover()?;
                let labels = find_all::<gtk::Label>(popover.upcast_ref());
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
        }
        idle(&window).await;
        finish(&window);
    });
}

// The trash items that came from `path` (recorded with its folder resolved).
fn trashed_from(path: &Path) -> Vec<trash::TrashItem> {
    let recorded = path
        .parent()
        .and_then(|folder| folder.canonicalize().ok())
        .zip(path.file_name())
        .map(|(folder, name)| folder.join(name));
    trash::os_limited::list()
        .unwrap_or_default()
        .into_iter()
        .filter(|item| Some(item.original_path()) == recorded)
        .collect()
}

// Opens the ⋯ menu (the toolbar's, or the view's), saves a screenshot and
// closes it.
async fn menu_shot(window: &gtk::ApplicationWindow, in_view: bool, name: &str) {
    let menu = find_all::<gtk::MenuButton>(window.upcast_ref())
        .into_iter()
        .filter(|menu| menu.tooltip_text().as_deref() == Some("More actions"))
        .find(|menu| menu.has_css_class("viewer-toolbar-menu") != in_view);
    if let Some(menu) = menu {
        menu.popup();
        sleep(300).await;
        shot(window, name);
        menu.popdown();
    }
}
