// Head-to-head workflow benchmarks: Rust port vs Swift `ViewBenchmarks`.
// JSON line shape matches the Swift harness so median/p95/RSS can be
// compared directly.

use std::env;
use std::path::{Path, PathBuf};
use std::time::Instant;

use view_core::{list_browser, scan_recursive, Catalog, MediaItem, MediaKind, Scheduler, ThumbKey};

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = values.len();
    if n % 2 == 1 { values[n / 2] } else { (values[n / 2 - 1] + values[n / 2]) / 2.0 }
}

fn p95(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = ((0.95 * sorted.len() as f64).ceil() as usize).saturating_sub(1).min(sorted.len().saturating_sub(1));
    sorted[idx]
}

fn emit(name: &str, scale: &str, samples: usize, mut times: Vec<f64>, before_kib: i64, after_kib: i64) {
    let min = times.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = times.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let med = median(times.clone());
    let p = p95(&times);
    println!(
        r#"{{"name":"{name}","scale":"{scale}","samples":{samples},"medianMs":{med:.3},"p95Ms":{p:.3},"minMs":{min:.3},"maxMs":{max:.3},"rssBeforeKiB":{before},"rssAfterKiB":{after},"rssDeltaKiB":{delta}}}"#,
        before = before_kib,
        after = after_kib,
        delta = after_kib - before_kib,
    );
    let _ = &mut times;
}

fn rss_kib() -> i64 {
    // Cheap RSS proxy via /proc-independent mallinfo is unavailable; fall
    // back to the same getrusage approach used on Darwin through libc is
    // available on macOS only. This binary is benchmarked on macOS, so
    // use the same rusage call.
    unsafe {
        let mut usage: libc_sys::Rusage = std::mem::zeroed();
        let rc = libc_sys::getrusage(libc_sys::RUSAGE_SELF, &mut usage);
        if rc == 0 {
            return (usage.ru_maxrss / 1024) as i64;
        }
    }
    0
}

fn measure<F>(name: &str, scale: &str, samples: usize, mut op: F)
where
    F: FnMut(usize),
{
    let before = rss_kib();
    let mut times = Vec::with_capacity(samples);
    for i in 0..samples {
        let start = Instant::now();
        op(i);
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    let after = rss_kib();
    emit(name, scale, samples, times, before, after);
}

fn args() -> (Option<PathBuf>, Option<PathBuf>, usize) {
    let argv: Vec<String> = env::args().skip(1).collect();
    let mut browser = None;
    let mut recursive = None;
    let mut samples = 5usize;
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--browser-corpus" => { i += 1; browser = argv.get(i).map(PathBuf::from); }
            "--recursive-corpus" => { i += 1; recursive = argv.get(i).map(PathBuf::from); }
            "--samples" => { i += 1; samples = argv.get(i).and_then(|s| s.parse().ok()).unwrap_or(5); }
            _ => {}
        }
        i += 1;
    }
    (browser, recursive, samples)
}

fn image_files(root: &Path, limit: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>, limit: usize) {
        if out.len() >= limit { return; }
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out, limit);
            } else if path.extension().and_then(|e| e.to_str()) == Some("png") {
                out.push(path);
                if out.len() >= limit { return; }
            }
        }
    }
    walk(root, &mut out, limit);
    out
}

fn main() {
    let (browser, recursive, samples) = args();
    let start = Instant::now();

    if let Some(dir) = browser {
        measure("browser-list", &dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), samples, |_| {
            let entries = list_browser(&dir).expect("browser list");
            let mut catalog = Catalog::new();
            catalog.append(entries);
            std::hint::black_box(catalog.count());
        });
    }

    measure("catalog-interact", "5000", samples, |sample| {
        let mut catalog = Catalog::new();
        let items: Vec<MediaItem> = (0..5000)
            .map(|i| {
                let mut item = MediaItem::new(format!("/bench/item-{i}.png"), MediaKind::Image);
                item.file_size = Some(((i * 137) % 400_000 + 1) as i64);
                item
            })
            .collect();
        catalog.append(items.clone());
        for offset in (0..200).map(|k| k * 25 + sample % 25) {
            let path = catalog.items().get(offset).map(|item| item.path.clone());
            if let Some(path) = path {
                let _ = catalog.select(&path);
            }
        }
        if let (Some(a), Some(b)) = (catalog.items().get(100).map(|i| i.path.clone()), catalog.items().get(700).map(|i| i.path.clone())) {
            let _ = catalog.select_range(&a, &b);
        }
        let target = items[2500].path.clone();
        catalog.remove(&target);
        catalog.insert(items[2500].clone(), 2500);
        std::hint::black_box(catalog.selected().map(|p| p.to_path_buf()));
    });

    if let Some(root) = recursive {
        measure("recursive-scan", &root.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), samples, |_| {
            let items = scan_recursive(&root).expect("scan");
            std::hint::black_box(items.len());
        });

        measure("scroll-prefetch", &root.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), samples, |sample| {
            let files = image_files(&root, 2000);
            let keys: Vec<ThumbKey> = files
                .iter()
                .map(|f| ThumbKey { path: f.clone(), pixel_size: 320, scale: 2 })
                .collect();
            if keys.len() < 80 { return; }
            let mut scheduler = Scheduler::new(4);
            let viewport = 24;
            let prefetch = 48;
            let mut offset = 0;
            while offset + viewport < keys.len() {
                let visible: Vec<_> = keys[offset..offset + viewport].to_vec();
                let end = (offset + viewport + prefetch).min(keys.len());
                let next: Vec<_> = keys[offset + viewport..end].to_vec();
                scheduler.update_demand(visible, next);
                offset += 8;
            }
            std::hint::black_box(sample);
        });

        if let Some(first) = image_files(&root, 400).into_iter().next() {
            measure("thumb-decode", &first.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), samples, |_| {
                let files = image_files(&root, 100);
                let chunk = files.len().div_ceil(4).max(1);
                std::thread::scope(|scope| {
                    for part in files.chunks(chunk) {
                        scope.spawn(move || {
                            for file in part {
                                let _ = view_core::decode_png(file);
                            }
                        });
                    }
                });
            });
        }
    }

    eprintln!("bench total-ms={:.0}", start.elapsed().as_secs_f64() * 1000.0);
}

// Minimal FFI shim so RSS reads the same value as the Swift harness on
// macOS without pulling in `libc` directly.
mod libc_sys {
    pub const RUSAGE_SELF: i32 = 0;
    #[repr(C)]
    pub struct Rusage {
        pub ru_utime: Timeval,
        pub ru_stime: Timeval,
        pub ru_maxrss: i64,
        pub ru_ixrss: i64,
        pub ru_idrss: i64,
        pub ru_isrss: i64,
        pub ru_minflt: i64,
        pub ru_majflt: i64,
        pub ru_nswap: i64,
        pub ru_inblock: i64,
        pub ru_oublock: i64,
        pub ru_msgsnd: i64,
        pub ru_msgrcv: i64,
        pub ru_nsignals: i64,
        pub ru_nvcsw: i64,
        pub ru_nivcsw: i64,
    }
    #[repr(C)]
    struct Timeval {
        pub tv_sec: i64,
        pub tv_usec: i32,
        pub tv_pad: i32,
    }
    unsafe extern "C" {
        pub fn getrusage(who: i32, usage: *mut Rusage) -> i32;
    }
}
