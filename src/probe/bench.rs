//! The benchmark (VITRINE_PROBE=1): the scenarios in bench/README.md, printed
//! as RESULT lines, then quits. Scenarios whose widgets don't exist yet are
//! reported as skipped.

use super::*;

const SCROLL_PX_PER_S: f64 = 4000.0;
const FILL_TIMEOUT_MS: i64 = 60_000;
const OPEN_POSITION: u32 = 0;
const HOLD_PRESSES: u32 = 60;
const HOLD_INTERVAL_MS: u64 = 33;
// open_selected: an image well away from the others opened, and how long it's
// selected before Enter.
const SELECT_POSITION: u32 = 10;
const SELECT_DWELL_MS: u64 = 300;
// hold_key: how long → is held, and watched after.
const HOLD_KEY_MS: u64 = 3000;
// pan: how long, and how fast (logical px per ms, sideways; half downwards).
const PAN_MS: i64 = 1500;
const PAN_PX_PER_MS: f64 = 1.0;
const SETTLE_WATCH_MS: i64 = 6000;

pub fn run(window: &gtk::ApplicationWindow) {
    let window = window.clone();
    glib::spawn_future_local(async move {
        let root: gtk::Widget = window.clone().upcast();
        let start = glib::monotonic_time();

        // scan: until the grid's item count stops changing.
        let recorder = Recorder::start(&window);
        let grid = wait_for_grid(&root).await;
        let (first, done, count) = match &grid {
            Some(grid) => wait_for_scan(grid, start).await,
            None => (-1, -1, 0),
        };
        recorder.finish(
            "scan",
            &format!("first_ms={first} done_ms={done} items={count}"),
        );

        let Some(grid) = grid else {
            println!("RESULT skipped scroll jump open hold (no grid yet)");
            idle(&window).await;
            return finish(&window);
        };
        let scrolled = grid
            .parent()
            .and_downcast::<gtk::ScrolledWindow>()
            .expect("the grid is in a ScrolledWindow");
        let adjustment = scrolled.vadjustment();

        // fill: the first screen of thumbnails.
        let fill = wait_until(FILL_TIMEOUT_MS, || tiles_filled(&grid)).await;
        let selected = grid
            .model()
            .and_downcast::<gtk::SingleSelection>()
            .map_or(-1, |selection| selection.selected() as i64);
        println!(
            "RESULT fill_first fill_ms={fill} top_px={:.0} selected={selected}",
            adjustment.value()
        );

        // scroll: top to bottom at a fixed speed.
        let recorder = Recorder::start(&window);
        let scroll_start = glib::monotonic_time();
        let end = adjustment.upper() - adjustment.page_size();
        while adjustment.value() < end {
            let elapsed = (glib::monotonic_time() - scroll_start) as f64 / 1e6;
            adjustment.set_value((elapsed * SCROLL_PX_PER_S).min(end));
            sleep(4).await;
        }
        let fill = wait_until(FILL_TIMEOUT_MS, || tiles_filled(&grid)).await;
        recorder.finish(
            "scroll",
            &format!("scroll_ms={} fill_after_ms={fill}", ms_since(scroll_start)),
        );

        // jump: to the middle, in one step.
        let recorder = Recorder::start(&window);
        adjustment.set_value(end / 2.0);
        let fill = wait_until(FILL_TIMEOUT_MS, || tiles_filled(&grid)).await;
        recorder.finish("jump", &format!("fill_ms={fill}"));

        // sort: by date, size, random, then name again, each keeping the
        // selected image selected.
        {
            let selection = grid
                .model()
                .and_downcast::<gtk::SingleSelection>()
                .expect("the grid's model is a SingleSelection");
            let selected_path = || {
                selection
                    .selected_item()
                    .and_downcast::<glib::BoxedAnyObject>()
                    .map(|object| object.borrow::<crate::library::Image>().path.clone())
            };
            let before = selected_path();
            let recorder = Recorder::start(&window);
            let (mut times, mut kept) = (Vec::new(), 0);
            for label in ["Date", "Size", "Random", "Name"] {
                let Some(button) = button(&root, label) else {
                    continue;
                };
                let started = glib::monotonic_time();
                button.emit_clicked();
                let took = glib::monotonic_time() - started;
                times.push(format!("{}_us={took}", label.to_lowercase()));
                kept += (selected_path() == before) as u32;
                sleep(300).await;
            }
            recorder.finish("sort", &format!("{} kept={kept}", times.join(" ")));
        }

        // open: from the top, the first image.
        adjustment.set_value(0.0);
        // Let the grid rebind its tiles first (they still show the middle).
        sleep(200).await;
        wait_until(FILL_TIMEOUT_MS, || tiles_filled(&grid)).await;
        let model = grid.model().expect("the grid has a model");
        let path_at = |position: u32| {
            model
                .item(position % model.n_items())
                .and_downcast::<glib::BoxedAnyObject>()
                .map(|object| object.borrow::<crate::library::Image>().path.clone())
                .expect("an image at the position")
        };
        let preview = find::<crate::view::zoomable::ZoomableImage>(&root)
            .expect("the full-screen view's image");
        let recorder = Recorder::start(&window);
        let open_start = glib::monotonic_time();
        grid.emit_by_name::<()>("activate", &[&OPEN_POSITION]);
        let placeholder = wait_until(5000, || preview.has_image()).await;
        let first = path_at(OPEN_POSITION);
        let sharp = wait_until(5000, || is_sharp(&first)).await;
        let sharp_ms = if sharp < 0 { -1 } else { ms_since(open_start) };
        sleep(1500u64.saturating_sub(ms_since(open_start) as u64)).await;
        recorder.finish(
            "open",
            &format!("placeholder_ms={placeholder} sharp_ms={sharp_ms}"),
        );

        // hold: → at 30 presses/s (the view moves on at its own, lower, rate),
        // then how long the last image takes.
        let recorder = Recorder::start(&window);
        for _ in 0..HOLD_PRESSES {
            press(&window, gdk::Key::Right);
            sleep(HOLD_INTERVAL_MS).await;
        }
        let moves = model
            .downcast_ref::<gtk::SingleSelection>()
            .map_or(0, |selection| selection.selected() - OPEN_POSITION);
        let last = path_at(OPEN_POSITION + moves);
        let settle = wait_until(5000, || is_sharp(&last)).await;
        recorder.finish(
            "hold",
            &format!("presses={HOLD_PRESSES} moves={moves} last_sharp_ms={settle}"),
        );

        let recorder = Recorder::start(&window);
        press(&window, gdk::Key::Escape);
        sleep(1000).await;
        recorder.finish("close", "");

        // open_selected: select an image in the grid, look at it for a
        // moment, then open it (the selection is decoded in the background).
        let selection = model
            .downcast_ref::<gtk::SingleSelection>()
            .expect("the grid's model is a SingleSelection");
        selection.set_selected(SELECT_POSITION);
        sleep(SELECT_DWELL_MS).await;
        let recorder = Recorder::start(&window);
        let open_start = glib::monotonic_time();
        grid.emit_by_name::<()>("activate", &[&SELECT_POSITION]);
        let selected = path_at(SELECT_POSITION);
        let sharp = wait_until(5000, || is_sharp(&selected)).await;
        let sharp_ms = if sharp < 0 { -1 } else { ms_since(open_start) };
        sleep(1000u64.saturating_sub(ms_since(open_start) as u64)).await;
        recorder.finish(
            "open_selected",
            &format!("dwell_ms={SELECT_DWELL_MS} sharp_ms={sharp_ms}"),
        );
        press(&window, gdk::Key::Escape);
        sleep(500).await;

        // zoom: open the first 48 MP image and zoom to 100% at the centre
        // (full resolution comes in tiles); pan: then drag across it.
        let huge = (0..model.n_items()).find(|&i| {
            path_at(i)
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("huge-"))
        });
        if let Some(position) = huge {
            grid.emit_by_name::<()>("activate", &[&position]);
            let path = path_at(position);
            wait_until(5000, || is_sharp(&path)).await;
            sleep(300).await;
            shot(&window, "zoom-fit");
            let recorder = Recorder::start(&window);
            let zoom_start = glib::monotonic_time();
            let (w, h) = (preview.width() as f64, preview.height() as f64);
            preview.zoom_at(preview.actual_scale() / preview.scale(), w / 2.0, h / 2.0);
            let detail = wait_until(10_000, || preview.is_detailed()).await;
            let detail_ms = if detail < 0 { -1 } else { ms_since(zoom_start) };
            sleep(1500u64.saturating_sub(ms_since(zoom_start) as u64)).await;
            recorder.finish("zoom", &format!("detail_ms={detail_ms}"));
            shot(&window, "zoom-100");

            let recorder = Recorder::start(&window);
            let pan_start = glib::monotonic_time();
            while ms_since(pan_start) < PAN_MS {
                preview.pan_by(-PAN_PX_PER_MS * 4.0, -PAN_PX_PER_MS * 2.0);
                sleep(4).await;
            }
            let detail = wait_until(10_000, || preview.is_detailed()).await;
            recorder.finish("pan", &format!("pan_ms={PAN_MS} detail_after_ms={detail}"));
            shot(&window, "pan");
            press(&window, gdk::Key::bracketright);
            press(&window, gdk::Key::h);
            sleep(300).await;
            shot(&window, "rotated-flipped");
            press(&window, gdk::Key::Escape);
            sleep(500).await;
        } else {
            println!("RESULT skipped zoom pan (no huge- image)");
        }

        // gif: play the first GIF for 3 s.
        let gif = (0..model.n_items()).find(|&i| crate::view::preview::is_animation(&path_at(i)));
        if let Some(position) = gif {
            grid.emit_by_name::<()>("activate", &[&position]);
            let path = path_at(position);
            wait_until(5000, || is_sharp(&path)).await;
            let frames_start = FRAMES.with(|frames| frames.get());
            let recorder = Recorder::start(&window);
            sleep(3000).await;
            let shown = FRAMES.with(|frames| frames.get()) - frames_start;
            recorder.finish("gif", &format!("played_ms=3000 frames_shown={shown}"));
            press(&window, gdk::Key::Escape);
            sleep(500).await;
        } else {
            println!("RESULT skipped gif (no GIF)");
        }

        // fullscreen: f in the view, the controls hiding after 2 s without
        // the mouse moving, and Esc restoring the window.
        {
            grid.emit_by_name::<()>("activate", &[&OPEN_POSITION]);
            sleep(500).await;
            press(&window, gdk::Key::f);
            let entered = wait_until(3000, || window.is_fullscreen()).await;
            sleep(2500).await;
            let boxes = find_all::<gtk::Box>(&root);
            let controls: Vec<_> = boxes
                .iter()
                .filter(|b| b.has_css_class("preview-controls"))
                .collect();
            let hidden = controls
                .iter()
                .filter(|b| b.has_css_class("hidden"))
                .count();
            press(&window, gdk::Key::Escape);
            let left = wait_until(3000, || !window.is_fullscreen()).await;
            println!(
                "RESULT fullscreen entered_ms={entered} controls={} hidden={hidden} left_ms={left}",
                controls.len()
            );
            sleep(500).await;
        }

        // hold_key: hold → for real (GTK repeats the key itself, so presses
        // queue up if showing each takes longer than the repeat interval),
        // then watch until things settle.
        if let Some(wtype) = std::env::var_os("VITRINE_PROBE_WTYPE") {
            grid.emit_by_name::<()>("activate", &[&OPEN_POSITION]);
            sleep(1000).await;
            SHOWS.with_borrow_mut(Vec::clear);
            let cpu_start = cpu_by_thread();
            let system_start = system_busy();
            let mut system_released = system_start;
            let pressed = glib::monotonic_time();
            let mut child = std::process::Command::new(wtype)
                // A pause first: a press sent as the virtual keyboard appears
                // arrives before GTK has taken it on, and is lost.
                .args([
                    "-s",
                    "300",
                    "-P",
                    "Right",
                    "-s",
                    &HOLD_KEY_MS.to_string(),
                    "-p",
                    "Right",
                ])
                .spawn()
                .expect("wtype starts");
            let mut released = None;
            let mut cpu_released = cpu_start.clone();
            // CPU in each 100 ms after the release, to see when it settles.
            let mut after = Vec::new();
            let mut last_total = cpu_total();
            while released.is_none() || ms_since(released.unwrap()) < SETTLE_WATCH_MS {
                sleep(100).await;
                let total = cpu_total();
                if released.is_none() && child.try_wait().ok().flatten().is_some() {
                    released = Some(glib::monotonic_time());
                    cpu_released = cpu_by_thread();
                    system_released = system_busy();
                } else if released.is_some() {
                    after.push(total - last_total);
                }
                last_total = total;
            }
            let _ = child.wait();
            let released = released.unwrap();
            let cpu_end = cpu_by_thread();
            let system_end = system_busy();
            let (during, since) = SHOWS.with_borrow(|shows| {
                let during = shows.iter().filter(|&&t| t <= released).count();
                (
                    during,
                    shows
                        .iter()
                        .filter(|&&t| t > released)
                        .copied()
                        .collect::<Vec<_>>(),
                )
            });
            let last_show = since.last().map_or(0, |&t| (t - released) / 1000);
            // Settled: the first 100 ms after the release using under 10 ms.
            let settle = after
                .iter()
                .position(|&ms| ms < 10.0)
                .map_or(-1, |i| i as i64 * 100);
            println!(
                "RESULT hold_key held_ms={} shows_held={during} shows_after={} last_show_after_ms={last_show} settle_ms={settle} cpu_held={} cpu_after={} system_held={:.0} system_after={:.0}",
                (released - pressed) / 1000,
                since.len(),
                cpu_delta(&cpu_start, &cpu_released),
                cpu_delta(&cpu_released, &cpu_end),
                system_released - system_start,
                system_end - system_released,
            );
            press(&window, gdk::Key::Escape);
            sleep(500).await;
        }

        idle(&window).await;
        finish(&window);
    });
}
