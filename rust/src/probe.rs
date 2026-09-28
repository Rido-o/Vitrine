//! Built-in benchmark, enabled with VITRINE_PROBE=1. Runs the scenarios in
//! bench/README.md and prints RESULT lines in the same format as
//! bench/probe-ts.tsx, then quits. Scenarios whose widgets don't exist yet are
//! reported as skipped.

use gtk::{glib, prelude::*};
use std::{cell::RefCell, rc::Rc, sync::OnceLock, time::Duration};

// When the window was built (the scan started), for times reported later.
static START: OnceLock<i64> = OnceLock::new();

/// Background generation is done: how many thumbnails it made, and when.
pub fn background_finished(generated: usize) {
    if std::env::var_os("VITRINE_PROBE").is_none() {
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
const SCROLL_PX_PER_S: f64 = 4000.0;
const SCAN_SETTLE_MS: i64 = 1000;
const FILL_TIMEOUT_MS: i64 = 60_000;

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
             over25={} stalls={} stall_sum={} stall_max={} {extra} t0_us={}",
            samples.frames.len(),
            pct(0.5),
            pct(0.95),
            pct(0.99),
            pct(1.0),
            intervals.iter().filter(|&&ms| ms > 25.0).count(),
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
        if std::env::var_os("VITRINE_PROBE").is_none() {
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

fn find_all<T: IsA<gtk::Widget>>(root: &gtk::Widget, out: &mut Vec<T>) {
    if let Some(found) = root.downcast_ref::<T>() {
        out.push(found.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        find_all(&widget, out);
        child = widget.next_sibling();
    }
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
    let mut pictures = Vec::new();
    find_all::<gtk::Picture>(grid.upcast_ref(), &mut pictures);
    pictures.iter().all(|p| p.paintable().is_some())
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
    format!("rss_mb={} hwm_mb={}", field("VmRSS:"), field("VmHWM:"))
}

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
        println!("RESULT fill_first fill_ms={fill}");

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

        println!("RESULT skipped open hold (no full-screen view yet)");
        idle(&window).await;
        finish(&window);
    });
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
