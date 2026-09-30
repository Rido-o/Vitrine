//! Built-in probe, enabled with VITRINE_PROBE: the benchmark (`bench.rs`, 1)
//! and a check of the controls (`ui.rs`, ui), both driving the window from
//! the main loop and printing RESULT lines. The app reports what it shows
//! through the hooks here.

pub mod bench;
pub mod ui;

use gtk::gdk;
use gtk::glib::translate::IntoGlib;
use gtk::{glib, prelude::*};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::OnceLock,
    time::Duration,
};

// When the window was built (the scan started), for times reported later.
static START: OnceLock<i64> = OnceLock::new();

/// Whether VITRINE_PROBE is set: the hooks record nothing otherwise.
fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("VITRINE_PROBE").is_some())
}

/// Background generation is done: how many thumbnails it made, and when.
pub fn background_finished(generated: usize) {
    if !enabled() {
        return;
    }
    let start = START.get().copied().unwrap_or_default();
    println!(
        "RESULT background generated={generated} done_ms={}",
        (glib::monotonic_time() - start) / 1000
    );
}

// A main-loop iteration taking longer than this counts as a stall.
const STALL_US: i64 = 8_000;
const SCAN_SETTLE_MS: i64 = 1000;

thread_local! {
    // What the full-screen view shows, and whether it's the decoded image.
    static SHOWN: RefCell<Option<(PathBuf, bool)>> = const { RefCell::new(None) };
}

/// Called by the full-screen view when it shows `path` (`sharp`: decoded,
/// not the thumbnail placeholder).
pub fn preview_shown(path: &Path, sharp: bool) {
    if !enabled() {
        return;
    }
    SHOWN.with_borrow_mut(|shown| *shown = Some((path.to_owned(), sharp)));
    SHOWS.with_borrow_mut(|shows| shows.push(glib::monotonic_time()));
}

/// Called by the full-screen view for each animation frame it shows.
pub fn animation_frame() {
    if enabled() {
        FRAMES.with(|frames| frames.set(frames.get() + 1));
    }
}

thread_local! {
    static FRAMES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

thread_local! {
    // When the view showed something (a move, or the sharp image arriving).
    static SHOWS: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
}

// CPU time (ms) used so far by each named thread ("main" for the process's
// own), from /proc/self/task/*/stat.
fn cpu_by_thread() -> std::collections::BTreeMap<String, f64> {
    let tick_ms = 1000.0 / unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    let pid = std::process::id().to_string();
    let mut totals = std::collections::BTreeMap::new();
    let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
        return totals;
    };
    for task in tasks.flatten() {
        let stat = std::fs::read_to_string(task.path().join("stat")).unwrap_or_default();
        // "tid (comm) state …": utime and stime are the 12th and 13th fields
        // after the comm.
        let Some((head, rest)) = stat.rsplit_once(") ") else {
            continue;
        };
        let fields: Vec<&str> = rest.split_whitespace().collect();
        let ticks: f64 = fields
            .get(11)
            .and_then(|f| f.parse::<f64>().ok())
            .unwrap_or(0.0)
            + fields
                .get(12)
                .and_then(|f| f.parse::<f64>().ok())
                .unwrap_or(0.0);
        let name = if task.file_name().to_string_lossy() == pid {
            "main".to_owned()
        } else {
            head.split_once(" (")
                .map_or("?", |(_, comm)| comm)
                .to_owned()
        };
        *totals.entry(name).or_insert(0.0) += ticks * tick_ms;
    }
    totals
}

// CPU time (ms) the whole system has spent busy so far, all cores, from
// /proc/stat (to see work done outside the app, e.g. by NFS).
fn system_busy() -> f64 {
    let tick_ms = 1000.0 / unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    let stat = std::fs::read_to_string("/proc/stat").unwrap_or_default();
    let Some(line) = stat.lines().next() else {
        return 0.0;
    };
    let fields: Vec<f64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|f| f.parse().ok())
        .collect();
    // user nice system idle iowait irq softirq steal …: all but idle, iowait.
    let busy: f64 = fields
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 3 && *i != 4)
        .take(6)
        .map(|(_, v)| v)
        .sum();
    busy * tick_ms
}

fn cpu_total() -> f64 {
    cpu_by_thread().values().sum()
}

fn cpu_delta(
    from: &std::collections::BTreeMap<String, f64>,
    to: &std::collections::BTreeMap<String, f64>,
) -> String {
    to.iter()
        .map(|(name, ms)| (name, ms - from.get(name).copied().unwrap_or(0.0)))
        .filter(|(_, ms)| *ms >= 10.0)
        .map(|(name, ms)| format!("{name}:{ms:.0}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn is_sharp(path: &Path) -> bool {
    SHOWN.with_borrow(|shown| matches!(shown, Some((p, true)) if p == path))
}

#[derive(Default)]
struct Samples {
    frames: Vec<i64>,
    stalls: Vec<i64>,
}

struct Recorder {
    // Monotonic µs, printed as t0_us to line runs up with profiles.
    start: i64,
    samples: Rc<RefCell<Samples>>,
    tick: Option<gtk::TickCallbackId>,
    timer: Option<glib::SourceId>,
}

impl Recorder {
    fn start(widget: &impl IsA<gtk::Widget>) -> Self {
        let samples = Rc::new(RefCell::new(Samples::default()));
        let frames = samples.clone();
        let tick = widget.add_tick_callback(move |_, clock| {
            frames.borrow_mut().frames.push(clock.frame_time());
            glib::ControlFlow::Continue
        });
        let stalls = samples.clone();
        let mut last = glib::monotonic_time();
        let timer = glib::timeout_add_local_full(
            Duration::from_millis(2),
            glib::Priority::HIGH,
            move || {
                let now = glib::monotonic_time();
                if now - last > STALL_US {
                    stalls.borrow_mut().stalls.push(now - last);
                }
                last = now;
                glib::ControlFlow::Continue
            },
        );
        Self {
            start: glib::monotonic_time(),
            samples,
            tick: Some(tick),
            timer: Some(timer),
        }
    }

    fn finish(mut self, label: &str, extra: &str) {
        self.tick.take().unwrap().remove();
        self.timer.take().unwrap().remove();
        let samples = self.samples.borrow();
        let mut intervals: Vec<f64> = samples
            .frames
            .windows(2)
            .map(|pair| (pair[1] - pair[0]) as f64 / 1000.0)
            .collect();
        intervals.sort_by(f64::total_cmp);
        let pct = |p: f64| -> f64 {
            if intervals.is_empty() {
                return 0.0;
            }
            intervals[((intervals.len() - 1) as f64 * p).round() as usize]
        };
        let stall_sum: i64 = samples.stalls.iter().sum();
        let stall_max = samples.stalls.iter().copied().max().unwrap_or(0);
        println!(
            "RESULT {label} frames={} p50={:.1} p95={:.1} p99={:.1} max={:.1} \
             over25={} late={} stalls={} stall_sum={} stall_max={} {extra} t0_us={}",
            samples.frames.len(),
            pct(0.5),
            pct(0.95),
            pct(0.99),
            pct(1.0),
            intervals.iter().filter(|&&ms| ms > 25.0).count(),
            intervals.iter().filter(|&&ms| ms > late_ms()).count(),
            samples.stalls.len(),
            stall_sum / 1000,
            stall_max / 1000,
            self.start,
        );
    }
}

/// Timing of the scan as the app sees it (from starting the walk), printed as
/// `scan_detail`: when the first and last batch arrived, how many batches,
/// and the longest main-thread insert.
pub struct ScanTimes {
    start: i64,
    first: i64,
    batches: u32,
    insert_max: i64,
}

impl ScanTimes {
    pub fn start() -> Self {
        let start = glib::monotonic_time();
        let _ = START.set(start);
        Self {
            start,
            first: -1,
            batches: 0,
            insert_max: 0,
        }
    }

    // After inserting a batch whose insert began at `started`.
    pub fn batch(&mut self, started: i64) {
        if self.first < 0 {
            self.first = started - self.start;
        }
        self.batches += 1;
        self.insert_max = self.insert_max.max(glib::monotonic_time() - started);
    }

    pub fn finish(&self, items: u32) {
        if !enabled() {
            return;
        }
        println!(
            "RESULT scan_detail first_us={} done_us={} batches={} insert_max_us={} items={items}",
            self.first,
            glib::monotonic_time() - self.start,
            self.batches,
            self.insert_max,
        );
    }
}

// A frame is late when it takes over 1.5 refresh intervals.
fn late_ms() -> f64 {
    let hz: f64 = std::env::var("VITRINE_PROBE_HZ")
        .ok()
        .and_then(|hz| hz.parse().ok())
        .unwrap_or(60.0);
    1.5 * 1000.0 / hz
}

fn find<T: IsA<gtk::Widget>>(root: &gtk::Widget) -> Option<T> {
    if let Some(found) = root.downcast_ref::<T>() {
        return Some(found.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(found) = find::<T>(&widget) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn find_all<T: IsA<gtk::Widget>>(root: &gtk::Widget) -> Vec<T> {
    fn collect<T: IsA<gtk::Widget>>(widget: &gtk::Widget, out: &mut Vec<T>) {
        if let Some(found) = widget.downcast_ref::<T>() {
            out.push(found.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            collect(&widget, out);
            child = widget.next_sibling();
        }
    }
    let mut found = Vec::new();
    collect(root, &mut found);
    found
}

fn ms_since(start: i64) -> i64 {
    (glib::monotonic_time() - start) / 1000
}

async fn sleep(ms: u64) {
    glib::timeout_future(Duration::from_millis(ms)).await
}

// Waits a frame at a time until `done`, up to `timeout_ms`; the time taken in
// ms, or -1 on timeout.
async fn wait_until(timeout_ms: i64, done: impl Fn() -> bool) -> i64 {
    let start = glib::monotonic_time();
    while !done() {
        if ms_since(start) > timeout_ms {
            return -1;
        }
        sleep(5).await;
    }
    ms_since(start)
}

// Every bound tile shows a thumbnail (trivially, while tiles have none).
fn tiles_filled(grid: &gtk::GridView) -> bool {
    let pictures = find_all::<gtk::Picture>(grid.upcast_ref());
    pictures.iter().all(|p| p.paintable().is_some())
}

// VITRINE_PROBE_SHOTS=DIR: the window as drawn, to DIR/NAME.png (to check
// what the view shows).
fn shot(window: &gtk::ApplicationWindow, name: &str) {
    let Some(dir) = std::env::var_os("VITRINE_PROBE_SHOTS") else {
        return;
    };
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
    let (Some(node), Some(renderer)) = (snapshot.to_node(), window.renderer()) else {
        return;
    };
    let texture = renderer.render_texture(&node, None);
    // The view's texture too, to tell its pixels from how they're drawn.
    if let Some(base) = find::<crate::view::zoomable::ZoomableImage>(window.upcast_ref())
        .and_then(|image| image.base_texture())
    {
        let _ = base.save_to_png(std::path::Path::new(&dir).join(format!("{name}-texture.png")));
    }
    let _ = texture.save_to_png(std::path::Path::new(&dir).join(format!("{name}.png")));
    // VITRINE_PROBE_GRIM: also the compositor's output, in device pixels
    // (the render above is at scale 1).
    if let Some(grim) = std::env::var_os("VITRINE_PROBE_GRIM") {
        let _ = std::process::Command::new(grim)
            .arg(std::path::Path::new(&dir).join(format!("{name}-device.png")))
            .status();
    }
}

fn memory() -> String {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |name: &str| {
        status
            .lines()
            .find(|line| line.starts_with(name))
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|kb| kb.parse::<i64>().ok())
            .map_or(0, |kb| kb / 1024)
    };
    format!(
        "rss_mb={} hwm_mb={} {}",
        field("VmRSS:"),
        field("VmHWM:"),
        memory_breakdown()
    )
}

// Resident memory by kind of mapping, from /proc/self/smaps: the main heap,
// other anonymous memory (malloc's per-thread arenas, large allocations),
// GPU driver mappings, and files (libraries, the thumbnail cache).
fn memory_breakdown() -> String {
    let smaps = std::fs::read_to_string("/proc/self/smaps").unwrap_or_default();
    let (mut heap, mut anon, mut gpu, mut file) = (0, 0, 0, 0);
    let mut kind = 0;
    for line in smaps.lines() {
        if let Some(kb) = line.strip_prefix("Rss:") {
            let kb: i64 = kb.trim().trim_end_matches(" kB").parse().unwrap_or(0);
            match kind {
                0 => heap += kb,
                1 => anon += kb,
                2 => gpu += kb,
                _ => file += kb,
            }
            continue;
        }
        // A mapping's header: "start-end perms offset dev inode [path]".
        let mut fields = line.split_whitespace();
        let is_header = fields.next().is_some_and(|range| range.contains('-'))
            && fields.next().is_some_and(|perms| perms.len() == 4);
        if !is_header {
            continue;
        }
        let path = fields.nth(3).unwrap_or("");
        kind = match path {
            "[heap]" => 0,
            "" | "[stack]" | "[anon]" => 1,
            _ if path.starts_with("/dev/nvidia") || path.starts_with("/dev/dri") => 2,
            _ if path.starts_with("/memfd:") || path.starts_with("[anon:") => 1,
            _ => 3,
        };
    }
    format!(
        "heap_mb={} anon_mb={} gpu_mb={} file_mb={}",
        heap / 1024,
        anon / 1024,
        gpu / 1024,
        file / 1024
    )
}

// Like GTK: the window's key controllers in turn until one handles it.
fn press(window: &gtk::ApplicationWindow, key: gdk::Key) {
    press_with(window, key, gdk::ModifierType::empty());
}

fn press_with(window: &gtk::ApplicationWindow, key: gdk::Key, state: gdk::ModifierType) {
    let controllers = window.observe_controllers();
    for i in 0..controllers.n_items() {
        let Some(keys) = controllers
            .item(i)
            .and_downcast::<gtk::EventControllerKey>()
        else {
            continue;
        };
        let handled = keys.emit_by_name::<bool>("key-pressed", &[&key.into_glib(), &0u32, &state]);
        if handled {
            return;
        }
    }
}

// The first button labelled `label`.
fn button(root: &gtk::Widget, label: &str) -> Option<gtk::Button> {
    let buttons = find_all::<gtk::Button>(root);
    buttons
        .into_iter()
        .find(|button| button.label().as_deref() == Some(label))
}

async fn wait_for_grid(root: &gtk::Widget) -> Option<gtk::GridView> {
    for _ in 0..100 {
        if let Some(grid) = find::<gtk::GridView>(root) {
            return Some(grid);
        }
        sleep(10).await;
    }
    None
}

// (first item ms, done ms, items), both from `start`.
async fn wait_for_scan(grid: &gtk::GridView, start: i64) -> (i64, i64, u32) {
    let count = || grid.model().map_or(0, |model| model.n_items());
    let mut first = -1;
    let mut last_count = 0;
    let mut last_change = glib::monotonic_time();
    loop {
        let n = count();
        let now = glib::monotonic_time();
        if n != last_count {
            if first < 0 && n > 0 {
                first = (now - start) / 1000;
            }
            last_count = n;
            last_change = now;
        } else if n > 0 && (now - last_change) / 1000 > SCAN_SETTLE_MS {
            return (first, (last_change - start) / 1000, n);
        }
        sleep(5).await;
    }
}

async fn idle(window: &gtk::ApplicationWindow) {
    let recorder = Recorder::start(window);
    sleep(2000).await;
    recorder.finish("idle", "");
}

fn finish(window: &gtk::ApplicationWindow) {
    println!("RESULT memory {}", memory());
    if let Some(app) = window.application() {
        app.quit();
    }
}

// The file name of the image the full-screen view shows (its info label).
fn shown_name(root: &gtk::Widget) -> Option<String> {
    let info = find_all::<gtk::Box>(root)
        .into_iter()
        .find(|b| b.has_css_class("preview-image-info"))?;
    find_all::<gtk::Label>(info.upcast_ref())
        .first()
        .map(|label| label.label().to_string())
}

// The toast's message while it shows.
fn toast_text(root: &gtk::Widget) -> Option<String> {
    find_all::<gtk::Box>(root)
        .into_iter()
        .find(|b| b.has_css_class("viewer-toast"))
        .filter(|b| b.is_visible())
        .and_then(|b| b.first_child().and_downcast::<gtk::Label>())
        .map(|label| label.label().to_string())
}
