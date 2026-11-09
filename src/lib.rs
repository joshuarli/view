//! Rust port of the `viewCore` discovery/catalog/scheduling subset.
//!
//! The Swift app is AppKit (`NSCollectionView`) plus Quick Look; the GUI is
//! not ported. This crate covers the pure domain logic exercised by the
//! Swift benchmarks — natural ordering, browser listing, recursive
//! discovery, catalog selection/removal/insertion, and demand scheduling
//! bookkeeping — with only the standard library plus `flate2` for PNG
//! decode. It is the subject of the head-to-head benchmark in `src/bin`.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaKind {
    Image,
    Movie,
    Directory,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaItem {
    pub path: PathBuf,
    pub kind: MediaKind,
    pub file_size: Option<i64>,
    pub modification: Option<u64>,
}

impl MediaItem {
    pub fn new(path: impl Into<PathBuf>, kind: MediaKind) -> Self {
        Self { path: path.into(), kind, file_size: None, modification: None }
    }
}

/// Numeric, case-insensitive natural ordering with a UTF-8 tiebreak.
pub fn natural_cmp(lhs: &str, rhs: &str) -> Ordering {
    let a: Vec<char> = lhs.chars().collect();
    let b: Vec<char> = rhs.chars().collect();
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let mut i_end = i;
            while i_end < a.len() && a[i_end].is_ascii_digit() { i_end += 1; }
            let mut j_end = j;
            while j_end < b.len() && b[j_end].is_ascii_digit() { j_end += 1; }
            let a_num = trim_leading_zeros(&a[i..i_end]);
            let b_num = trim_leading_zeros(&b[j..j_end]);
            if a_num.len() != b_num.len() {
                return a_num.len().cmp(&b_num.len());
            }
            let order = a_num.iter().cmp(b_num.iter());
            if order != Ordering::Equal {
                return order;
            }
            i = i_end;
            j = j_end;
        } else {
            let x = a[i].to_lowercase().next().unwrap_or(a[i]);
            let y = b[j].to_lowercase().next().unwrap_or(b[j]);
            if x != y {
                return x.cmp(&y);
            }
            i += 1;
            j += 1;
        }
    }
    if a.len() - i != b.len() - j {
        return (a.len() - i).cmp(&(b.len() - j));
    }
    lhs.as_bytes().cmp(rhs.as_bytes())
}

fn trim_leading_zeros(s: &[char]) -> &[char] {
    let mut start = 0;
    while start + 1 < s.len() && s[start] == '0' { start += 1; }
    &s[start..]
}

/// Ordering for a browser listing: directories first by name, then files
/// by descending size (unknown sizes last), then natural name.
pub fn browser_order(a: &MediaItem, b: &MediaItem) -> Ordering {
    let a_dir = a.kind == MediaKind::Directory;
    let b_dir = b.kind == MediaKind::Directory;
    if a_dir != b_dir {
        return if a_dir { Ordering::Less } else { Ordering::Greater };
    }
    if !a_dir {
        match (a.file_size, b.file_size) {
            (Some(x), Some(y)) if x != y => return y.cmp(&x),
            (Some(_), None) => return Ordering::Less,
            (None, Some(_)) => return Ordering::Greater,
            _ => {}
        }
    }
    let an = a.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let bn = b.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    natural_cmp(&an, &bn)
}

pub fn kind_for_extension(path: &Path) -> Option<MediaKind> {
    // Mirrors the Swift UTType conformance surface: image and movie types
    // across the common extensions emitted in the benchmark corpus.
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref() {
        Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("webp") | Some("tif") | Some("tiff")
        | Some("heic") | Some("heif") | Some("avif") | Some("bmp") | Some("jp2") | Some("j2k") | Some("jp2c")
        | Some("ico") | Some("icns") | Some("svg") | Some("svgz") | Some("pbm") | Some("pgm") | Some("ppm") => Some(MediaKind::Image),
        Some("mp4") | Some("mov") | Some("m4v") | Some("webm") | Some("avi") | Some("mkv") | Some("mpg")
        | Some("mpeg") | Some("3gp") | Some("m2v") | Some("wmv") => Some(MediaKind::Movie),
        _ => sniff_kind(path),
    }
}

fn sniff_kind(path: &Path) -> Option<MediaKind> {
    use std::io::Read;
    let mut buf = [0u8; 12];
    let n = fs::File::open(path).ok()?.read(&mut buf).ok()?;
    if n < 4 {
        return None;
    }
    if buf.starts_with(&[0x89, 0x50, 0x4E, 0x47]) || buf.starts_with(&[0xFF, 0xD8, 0xFF])
        || buf.starts_with(b"GIF8") || buf.starts_with(b"RIFF")
    {
        return Some(MediaKind::Image);
    }
    if n >= 8 && buf[4..8] == *b"ftyp" {
        return Some(MediaKind::Movie);
    }
    if buf.starts_with(&[0x00, 0x00, 0x01, 0x00]) || buf.starts_with(b"II") || buf.starts_with(b"MM") {
        return Some(MediaKind::Image);
    }
    None
}

pub fn is_package(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref(),
        Some("app") | Some("bundle") | Some("framework") | Some("scptd") | Some("svc")
            | Some("pages") | Some("numbers") | Some("key") | Some("fptemplate") | Some("wflow")
    )
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with('.'))
        .unwrap_or(false)
}

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).map(|m| m.file_type().is_symlink()).unwrap_or(false)
}

pub fn list_browser(dir: &Path) -> std::io::Result<Vec<MediaItem>> {
    let mut items = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if is_symlink(&path) || is_hidden(&path) || is_package(&path) {
            continue;
        }
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            items.push(MediaItem::new(path, MediaKind::Directory));
        } else if metadata.is_file()
            && let Some(kind) = kind_for_extension(&path) {
                let mut item = MediaItem::new(path, kind);
                item.file_size = Some(metadata.len() as i64);
                item.modification = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());
                items.push(item);
            }
    }
    items.sort_by(browser_order);
    Ok(items)
}

pub fn scan_recursive(root: &Path) -> std::io::Result<Vec<MediaItem>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries: Vec<PathBuf> = fs::read_dir(&dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        entries.sort_by(|a, b| {
            let an = a.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let bn = b.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            natural_cmp(&an, &bn)
        });
        // Depth-first like the Swift scanner (stable batch order).
        for path in entries.iter().rev() {
            if is_symlink(path) || is_hidden(path) || is_package(path) {
                continue;
            }
            let metadata = fs::metadata(path).ok();
            let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            if is_dir {
                stack.push(path.clone());
                continue;
            }
            if metadata.as_ref().map(|m| m.is_file()).unwrap_or(false)
                && let Some(kind) = kind_for_extension(path) {
                    let mut item = MediaItem::new(path.clone(), kind);
                    item.file_size = metadata.as_ref().map(|m| m.len() as i64);
                    item.modification = metadata
                        .as_ref()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs());
                    out.push(item);
                }
        }
    }
    Ok(out)
}

#[derive(Clone, Debug, PartialEq)]
pub struct CatalogError(pub PathBuf);

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "item not found: {}", self.0.display())
    }
}
impl std::error::Error for CatalogError {}

#[derive(Clone, Debug)]
pub struct Catalog {
    entries: Vec<MediaItem>,
    selection: Vec<PathBuf>,
    focus: Option<PathBuf>,
}

impl Catalog {
    pub fn new() -> Self { Self { entries: Vec::new(), selection: Vec::new(), focus: None } }
    pub fn items(&self) -> &[MediaItem] { &self.entries }
    pub fn selected(&self) -> Option<&Path> { self.focus.as_deref() }
    pub fn anchor(&self) -> Option<&Path> { self.selection.first().map(PathBuf::as_path) }
    pub fn selected_all(&self) -> &[PathBuf] { &self.selection }
    pub fn count(&self) -> usize { self.entries.len() }

    pub fn focus_index(&self) -> Option<usize> {
        let focus = self.focus.as_ref()?;
        self.entries.iter().position(|item| &item.path == focus)
    }

    pub fn selection_bounds(&self) -> Option<(usize, usize)> {
        let focus = self.focus_index()?;
        let anchor = self.anchor()
            .and_then(|path| self.entries.iter().position(|item| item.path == path))
            .unwrap_or(focus);
        Some((anchor.min(focus), anchor.max(focus)))
    }

    fn repair_focus(&mut self) {
        self.selection.retain(|path| self.entries.iter().any(|item| item.path == *path));
        if self.selection.is_empty() {
            self.focus = None;
            return;
        }
        if self.focus.as_ref().is_none_or(|path| !self.selection.iter().any(|selected| selected == path)) {
            self.focus = Some(self.selection[0].clone());
        }
    }

    pub fn append(&mut self, items: Vec<MediaItem>) -> (usize, usize) {
        let start = self.entries.len();
        self.entries.extend(items);
        (start, self.entries.len())
    }

    pub fn select(&mut self, url: &Path) -> Result<(), CatalogError> {
        if self.entries.iter().any(|i| i.path == url) {
            self.selection = vec![url.to_path_buf()];
            self.focus = Some(url.to_path_buf());
            Ok(())
        } else {
            Err(CatalogError(url.to_path_buf()))
        }
    }

    pub fn select_additional(&mut self, url: &Path) -> Result<(), CatalogError> {
        if !self.entries.iter().any(|i| i.path == url) {
            return Err(CatalogError(url.to_path_buf()));
        }
        if !self.selection.iter().any(|p| p.as_path() == url) {
            self.selection.push(url.to_path_buf());
        }
        if self.focus.is_none() {
            self.focus = Some(url.to_path_buf());
        }
        Ok(())
    }

    pub fn deselect(&mut self, url: &Path) {
        self.selection.retain(|p| p.as_path() != url);
        self.repair_focus();
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
        self.focus = None;
    }

    pub fn select_range(&mut self, anchor: &Path, clicked: &Path) -> Result<(), CatalogError> {
        let anchor_index = self.entries.iter().position(|i| i.path == anchor);
        let clicked_index = self.entries.iter().position(|i| i.path == clicked);
        let (anchor_index, clicked_index) = match (anchor_index, clicked_index) {
            (Some(a), Some(c)) => (a, c),
            (None, _) => return Err(CatalogError(anchor.to_path_buf())),
            (_, None) => return Err(CatalogError(clicked.to_path_buf())),
        };
        let (lo, hi) = if anchor_index <= clicked_index { (anchor_index, clicked_index) } else { (clicked_index, anchor_index) };
        let mut next = vec![anchor.to_path_buf()];
        for item in &self.entries[lo..=hi] {
            if item.path != anchor {
                next.push(item.path.clone());
            }
        }
        self.selection = next;
        self.focus = Some(clicked.to_path_buf());
        Ok(())
    }

    pub fn insert(&mut self, item: MediaItem, at: usize) -> usize {
        let idx = at.min(self.entries.len());
        self.entries.insert(idx, item);
        idx
    }

    pub fn remove(&mut self, url: &Path) -> Option<usize> {
        let index = self.entries.iter().position(|i| i.path == url)?;
        self.entries.remove(index);
        self.selection.retain(|p| p != url);
        if self.selection.is_empty() && !self.entries.is_empty() {
            let pick = if index < self.entries.len() { index } else { self.entries.len() - 1 };
            self.selection = vec![self.entries[pick].path.clone()];
        }
        self.repair_focus();
        Some(index)
    }
}

impl Default for Catalog {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Demand scheduler bookkeeping (sync port of ThumbnailScheduler state machine)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThumbKey {
    pub path: PathBuf,
    pub pixel_size: u32,
    pub scale: u32,
}

pub struct Scheduler {
    pub limit: usize,
    visible: Vec<ThumbKey>,
    prefetch: Vec<ThumbKey>,
    generations: HashMap<ThumbKey, u64>,
    next_generation: u64,
    requests: usize,
}

impl Scheduler {
    pub fn new(limit: usize) -> Self {
        Self { limit, visible: Vec::new(), prefetch: Vec::new(), generations: HashMap::new(), next_generation: 0, requests: 0 }
    }

    pub fn update_demand(&mut self, visible: Vec<ThumbKey>, prefetch: Vec<ThumbKey>) {
        let mut seen = HashSet::new();
        let visible_dedup: Vec<_> = visible.into_iter().filter(|k| seen.insert(k.clone())).collect();
        let visible_set: HashSet<_> = visible_dedup.iter().cloned().collect();
        let prefetch_dedup: Vec<_> = prefetch.into_iter().filter(|k| !visible_set.contains(k) && seen.insert(k.clone())).collect();
        let next: HashSet<_> = visible_dedup.iter().chain(prefetch_dedup.iter()).cloned().collect();
        self.generations.retain(|k, _| next.contains(k));
        for key in visible_dedup.iter().chain(prefetch_dedup.iter()) {
            if !self.generations.contains_key(key) {
                self.next_generation += 1;
                self.generations.insert(key.clone(), self.next_generation);
            }
        }
        self.visible = visible_dedup;
        self.prefetch = prefetch_dedup;
        self.requests += self.pending().len().min(self.limit);
    }

    pub fn pending(&self) -> Vec<ThumbKey> {
        self.visible.iter().chain(self.prefetch.iter()).take(self.limit).cloned().collect()
    }

    pub fn generation_of(&self, key: &ThumbKey) -> Option<u64> {
        self.generations.get(key).copied()
    }

    pub fn cancel_all(&mut self) {
        self.next_generation += 1;
        self.generations.clear();
        self.visible.clear();
        self.prefetch.clear();
    }

    pub fn execute_pending<F>(&self, work: F) -> Vec<(ThumbKey, u64)>
    where
        F: Fn(&ThumbKey) + Send + Sync,
    {
        let pending = self.pending();
        let generations = self.generations.clone();
        std::thread::scope(|scope| {
            let workers = self.limit.max(1).min(pending.len().max(1));
            if workers <= 1 || pending.is_empty() {
                let mut out = Vec::new();
                for key in &pending {
                    work(key);
                    if let Some(generation) = generations.get(key) {
                        out.push((key.clone(), *generation));
                    }
                }
                return out;
            }
            let chunk = pending.len().div_ceil(workers);
            let handles: Vec<_> = pending
                .chunks(chunk)
                .map(|part| {
                    let part = part.to_vec();
                    let work = &work;
                    let generations = &generations;
                    scope.spawn(move || {
                        let mut out = Vec::new();
                        for key in &part {
                            work(key);
                            if let Some(generation) = generations.get(key) {
                                out.push((key.clone(), *generation));
                            }
                        }
                        out
                    })
                })
                .collect();
            let mut out = Vec::new();
            for handle in handles {
                out.extend(handle.join().unwrap());
            }
            out
        })
    }
}

// ---------------------------------------------------------------------------
// PNG decode (subset: 8-bit colour types 0/2/4/6, non-interlaced) via flate2
// ---------------------------------------------------------------------------

pub fn decode_png(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    decode_png_bytes(&data)
}

// Decode with the same ImageIO backend as Swift (CGImageSource) —
// full bitmap render via CGContext so callers get real pixels at
// system-decoder speed. The pure-Rust path above remains the
// parallelism fallback and the default for pixel-exact comparisons.
pub fn decode_png_native(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    unsafe {
        let url = url_for(path);
        if url.is_null() {
            return Err("url failed".into());
        }
        let source = cg_image_source_create_with_url(url, std::ptr::null());
        if source.is_null() {
            return Err("image source failed".into());
        }
        let image = cg_image_source_create_image_at_index(source, 0, std::ptr::null());
        if image.is_null() {
            cf_release(source as *const std::ffi::c_void);
            return Err("image failed".into());
        }
        let width = cg_image_get_width(image);
        let height = cg_image_get_height(image);
        let bytes_per_row = width * 4;
        let mut pixels = vec![0u8; bytes_per_row * height];
        let color_space = cg_color_space_create_device_rgb();
        if color_space.is_null() {
            cg_image_release(image);
            cf_release(source as *const std::ffi::c_void);
            return Err("color space failed".into());
        }
        let context = cg_context_create(
            pixels.as_mut_ptr() as *mut std::ffi::c_void,
            width,
            height,
            8,
            bytes_per_row,
            color_space,
            1, // kCGImageAlphaPremultipliedLast
        );
        if context.is_null() {
            cg_color_space_release(color_space);
            cg_image_release(image);
            cf_release(source as *const std::ffi::c_void);
            return Err("context failed".into());
        }
        let rect = CGRect {
            x: 0.0,
            y: 0.0,
            width: width as f64,
            height: height as f64,
        };
        cg_context_draw_image(context, rect, image);
        cg_context_release(context);
        cg_color_space_release(color_space);
        cg_image_release(image);
        cf_release(source as *const std::ffi::c_void);
Ok((width as u32, height as u32, pixels))
    }
}

#[derive(Clone, Copy)]
struct CachedUrl(*mut std::ffi::c_void);
unsafe impl Send for CachedUrl {}
unsafe impl Sync for CachedUrl {}

fn url_for(path: &Path) -> CFURLRef {
    use std::sync::Mutex;
    use std::collections::HashMap;
    static CACHE: std::sync::OnceLock<Mutex<HashMap<PathBuf, CachedUrl>>> = std::sync::OnceLock::new();
    unsafe {
        let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        let mut guard = cache.lock().unwrap();
        if let Some(url) = guard.get(path) {
            return url.0 as CFURLRef;
        }
        let c_path = match std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) {
            Ok(c) => c,
            Err(_) => return std::ptr::null_mut(),
        };
        let cf_string = cf_string_create_with_c_string(std::ptr::null(), c_path.as_ptr(), 0x0800_0100);
        if cf_string.is_null() {
            return std::ptr::null_mut();
        }
        let url = cf_url_create_with_file_system_path(std::ptr::null(), cf_string, 0, true);
        cf_release(cf_string as *const std::ffi::c_void);
        if url.is_null() {
            return std::ptr::null_mut();
        }
        guard.insert(path.to_path_buf(), CachedUrl(url));
        url
    }
}

type CGImageSource = std::ffi::c_void;
type CGImage = std::ffi::c_void;

type CGColorSpace = std::ffi::c_void;
type CGContext = std::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    #[link_name = "CGColorSpaceCreateDeviceRGB"]
    fn cg_color_space_create_device_rgb() -> *mut CGColorSpace;
    #[link_name = "CGColorSpaceRelease"]
    fn cg_color_space_release(color_space: *mut CGColorSpace);
    #[link_name = "CGBitmapContextCreate"]
    fn cg_context_create(
        data: *mut std::ffi::c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        color_space: *mut CGColorSpace,
        bitmap_info: u32,
    ) -> *mut CGContext;
    #[link_name = "CGContextDrawImage"]
    fn cg_context_draw_image(context: *mut CGContext, rect: CGRect, image: *mut CGImage);
    #[link_name = "CGContextRelease"]
    fn cg_context_release(context: *mut CGContext);
}

#[cfg(test)]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    #[link_name = "CGContextSynchronize"]
    fn cg_context_synchronize(context: *mut CGContext);
    #[link_name = "CGContextSetRGBFillColor"]
    fn cg_context_set_rgb_fill_color(context: *mut CGContext, red: f64, green: f64, blue: f64, alpha: f64);
    #[link_name = "CGContextFillRect"]
    fn cg_context_fill_rect(context: *mut CGContext, rect: CGRect);
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    #[link_name = "CGImageSourceCreateWithURL"]
    fn cg_image_source_create_with_url(
        url: *mut std::ffi::c_void,
        options: *const std::ffi::c_void,
    ) -> *mut CGImageSource;
    #[link_name = "CGImageSourceCreateImageAtIndex"]
    fn cg_image_source_create_image_at_index(
        source: *mut CGImageSource,
        index: usize,
        options: *const std::ffi::c_void,
    ) -> *mut CGImage;
    #[link_name = "CGImageGetWidth"]
    fn cg_image_get_width(image: *mut CGImage) -> usize;
    #[link_name = "CGImageGetHeight"]
    fn cg_image_get_height(image: *mut CGImage) -> usize;
    #[link_name = "CGImageRelease"]
    fn cg_image_release(image: *mut CGImage);
}

type CFStringRef = *mut std::ffi::c_void;
type CFURLRef = *mut std::ffi::c_void;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    #[link_name = "CFRelease"]
    fn cf_release(cf: *const std::ffi::c_void);
    #[link_name = "CFStringCreateWithCString"]
    fn cf_string_create_with_c_string(
        allocator: *const std::ffi::c_void,
        c_string: *const std::os::raw::c_char,
        encoding: u32,
    ) -> CFStringRef;
    #[link_name = "CFURLCreateWithFileSystemPath"]
    fn cf_url_create_with_file_system_path(
        allocator: *const std::ffi::c_void,
        path: CFStringRef,
        style: i32,
        is_absolute: bool,
    ) -> CFURLRef;
}

pub fn decode_png_bytes(data: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    if data.len() < 8 || &data[0..8] != b"\x89PNG\r\n\x1a\n" {
        return Err("not png".into());
    }
    let mut pos = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut bit_depth = 0u8;
    let mut color_type = 0u8;
    let mut idat = Vec::new();
    while pos + 8 <= data.len() {
        let len = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        let kind = &data[pos + 4..pos + 8];
        let body_end = pos + 8 + len;
        if body_end > data.len() { break; }
        match kind {
            b"IHDR" => {
                width = u32::from_be_bytes(data[pos + 8..pos + 12].try_into().unwrap());
                height = u32::from_be_bytes(data[pos + 12..pos + 16].try_into().unwrap());
                bit_depth = data[pos + 16];
                color_type = data[pos + 17];
            }
            b"IDAT" => idat.extend_from_slice(&data[pos + 8..pos + 8 + len]),
            b"IEND" => break,
            _ => {}
        }
        pos = pos + 8 + len + 4;
    }
    if width == 0 || height == 0 { return Err("no ihdr".into()); }
    if bit_depth != 8 { return Err("unsupported bit depth".into()); }
    let channels: usize = match color_type { 0 => 1, 2 => 3, 4 => 2, 6 => 4, _ => return Err("unsupported color type".into()) };
    let mut decoder = flate2::read::ZlibDecoder::new(&idat[..]);
    let mut raw = Vec::new();
    decoder.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let stride = width as usize * channels;
    if raw.len() < (stride + 1) * height as usize {
        return Err("short stream".into());
    }
    let mut out = vec![0u8; stride * height as usize];
    let mut prev = vec![0u8; stride];
    for y in 0..height as usize {
        let row = &raw[y * (stride + 1)..(y + 1) * (stride + 1)];
        let filter = row[0];
        let recon = unfilter_row(filter, &row[1..], &prev, channels);
        out[y * stride..(y + 1) * stride].copy_from_slice(&recon);
        prev = recon;
    }
    Ok((width, height, out))
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let a = a as i16;
    let b = b as i16;
    let c = c as i16;
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc { a as u8 } else if pb <= pc { b as u8 } else { c as u8 }
}

fn unfilter_row(filter: u8, bytes: &[u8], prev: &[u8], channels: usize) -> Vec<u8> {
    let mut out = vec![0u8; bytes.len()];
    for i in 0..bytes.len() {
        let left = if i >= channels { out[i - channels] } else { 0 };
        let up = prev[i];
        let up_left = if i >= channels { prev[i - channels] } else { 0 };
        out[i] = match filter {
            0 => bytes[i],
            1 => bytes[i].wrapping_add(left),
            2 => bytes[i].wrapping_add(up),
            3 => bytes[i].wrapping_add(((left as u16 + up as u16) / 2) as u8),
            4 => bytes[i].wrapping_add(paeth(left, up, up_left)),
            _ => bytes[i],
        };
    }
    out
}

// ---------------------------------------------------------------------------
// Launch / root / commands / preview / trash history / thumbnail cache
// (parity with Sources/viewCore)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(Default)]
pub enum BrowserMode { #[default]
Browser, Recursive }


#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchError {
    NotFound(String),
    NotDirectory(String),
    UnknownOption(String),
    UnexpectedArgument(String),
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LaunchError::NotFound(p) => write!(f, "no such directory: {p}"),
            LaunchError::NotDirectory(p) => write!(f, "not a directory: {p}"),
            LaunchError::UnknownOption(o) => write!(f, "unknown option: {o}"),
            LaunchError::UnexpectedArgument(a) => write!(f, "unexpected argument: {a}"),
        }
    }
}

fn normalize_path(path: &Path, current: &Path) -> PathBuf {
    let joined: PathBuf = if path.is_absolute() {
        path.to_path_buf()
    } else {
        current.join(path)
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => { out.pop(); }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out = current.to_path_buf();
    }
    out
}

pub fn resolve_root(argument: Option<&str>, current_directory: &str) -> Result<PathBuf, LaunchError> {
    let current = normalize_path(Path::new(current_directory), Path::new("/"));
    let requested = match argument {
        Some("~") => {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
            PathBuf::from(home)
        }
        Some(arg) if arg.starts_with("~/") => {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
            normalize_path(Path::new(&arg[2..]), Path::new(&home))
        }
        Some(arg) => PathBuf::from(arg),
        None => current.clone(),
    };
    let url = normalize_path(&requested, &current);
    let metadata = std::fs::metadata(&url).map_err(|_| LaunchError::NotFound(url.display().to_string()))?;
    if !metadata.is_dir() {
        return Err(LaunchError::NotDirectory(url.display().to_string()));
    }
    Ok(url)
}

pub fn resolve_launch(arguments: &[String], current_directory: &str) -> Result<(PathBuf, BrowserMode), LaunchError> {
    let mut mode = BrowserMode::Browser;
    let mut dir_arg: Option<String> = None;
    let mut parses_options = true;
    for arg in arguments {
        if parses_options {
            match arg.as_str() {
                "-r" | "--recursive" => { mode = BrowserMode::Recursive; continue; }
                "--" => { parses_options = false; continue; }
                _ => {
                    if arg.starts_with('-') && arg != "-" {
                        return Err(LaunchError::UnknownOption(arg.clone()));
                    }
                }
            }
        }
        if dir_arg.is_some() {
            return Err(LaunchError::UnexpectedArgument(arg.clone()));
        }
        dir_arg = Some(arg.clone());
    }
    let root = resolve_root(dir_arg.as_deref(), current_directory)?;
    Ok((root, mode))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GridKeyModifier { Command, Control, Option, Shift }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridShortcut {
    pub key_equivalent: &'static str,
    pub modifiers: &'static [GridKeyModifier],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridCommand {
    TogglePreview,
    TrashSelectedItem,
    Undo,
    Redo,
    Back,
    Forward,
    UpLevel,
    ActivateDirectory,
}

pub fn grid_shortcut(command: GridCommand) -> GridShortcut {
    match command {
        GridCommand::TogglePreview => GridShortcut { key_equivalent: " ", modifiers: &[] },
        GridCommand::TrashSelectedItem => GridShortcut { key_equivalent: "\u{8}", modifiers: &[GridKeyModifier::Control] },
        GridCommand::Undo => GridShortcut { key_equivalent: "z", modifiers: &[GridKeyModifier::Command] },
        GridCommand::Redo => GridShortcut { key_equivalent: "Z", modifiers: &[GridKeyModifier::Command, GridKeyModifier::Shift] },
        GridCommand::Back => GridShortcut { key_equivalent: "\u{F702}", modifiers: &[GridKeyModifier::Shift] },
        GridCommand::Forward => GridShortcut { key_equivalent: "\u{F703}", modifiers: &[GridKeyModifier::Shift] },
        GridCommand::UpLevel => GridShortcut { key_equivalent: "\u{8}", modifiers: &[] },
        GridCommand::ActivateDirectory => GridShortcut { key_equivalent: "\r", modifiers: &[] },
    }
}

pub fn resolve_command(key_equivalent: &str, modifiers: &[GridKeyModifier]) -> Option<GridCommand> {
    [
        GridCommand::TogglePreview,
        GridCommand::TrashSelectedItem,
        GridCommand::Undo,
        GridCommand::Redo,
        GridCommand::Back,
        GridCommand::Forward,
        GridCommand::UpLevel,
        GridCommand::ActivateDirectory,
    ]
    .into_iter()
    .find(|c| {
        let s = grid_shortcut(*c);
        s.key_equivalent == key_equivalent && s.modifiers == modifiers
    })
}

pub fn resolve_command_key(key_code: u16, modifiers: &[GridKeyModifier]) -> Option<GridCommand> {
    match key_code {
        123 if modifiers.contains(&GridKeyModifier::Shift) => Some(GridCommand::Back),
        124 if modifiers.contains(&GridKeyModifier::Shift) => Some(GridCommand::Forward),
        49 => Some(GridCommand::TogglePreview),
        51 => Some(if modifiers.is_empty() { GridCommand::UpLevel } else { GridCommand::TrashSelectedItem }),
        36 => Some(GridCommand::ActivateDirectory),
        _ => None,
    }
}

#[derive(Clone, Debug, Default)]
pub struct PreviewSession {
    urls: Vec<PathBuf>,
    selected: Option<usize>,
}

impl PreviewSession {
    pub fn new() -> Self { Self::default() }
    pub fn count(&self) -> usize { self.urls.len() }
    pub fn current_index(&self) -> Option<usize> { self.selected }

    pub fn begin(&mut self, catalog: &Catalog) -> bool {
        let next: Vec<PathBuf> = catalog.items().iter().map(|i| i.path.clone()).collect();
        let Some(selected) = catalog.selected().map(Path::to_path_buf) else {
            self.end();
            return false;
        };
        let Some(index) = next.iter().position(|u| *u == selected) else {
            self.end();
            return false;
        };
        self.urls = next;
        self.selected = Some(index);
        true
    }

    pub fn item_at(&self, index: usize) -> Option<&Path> {
        self.urls.get(index).map(PathBuf::as_path)
    }

    pub fn index_of(&self, url: &Path) -> Option<usize> {
        self.urls.iter().position(|u| u.as_path() == url)
    }

    pub fn select_at(&mut self, index: usize) -> Option<PathBuf> {
        let url = self.item_at(index)?.to_path_buf();
        self.selected = Some(index);
        Some(url)
    }

    pub fn update(&mut self, catalog: &Catalog) {
        let Some(previous_index) = self.selected else { return };
        let Some(previous_url) = self.urls.get(previous_index).cloned() else { return };
        let catalog_urls: HashSet<PathBuf> = catalog.items().iter().map(|i| i.path.clone()).collect();
        let next_urls: Vec<PathBuf> = self.urls.iter().filter(|u| catalog_urls.contains(*u)).cloned().collect();
        if next_urls == self.urls {
            return;
        }
        if next_urls.is_empty() {
            self.end();
            return;
        }
        if let Some(index) = next_urls.iter().position(|u| *u == previous_url) {
            self.urls = next_urls;
            self.selected = Some(index);
            return;
        }
        if let Some(selected_url) = catalog.selected()
            && let Some(index) = next_urls.iter().position(|u| u.as_path() == selected_url) {
                self.urls = next_urls;
                self.selected = Some(index);
                return;
            }
        self.selected = Some(previous_index.min(next_urls.len() - 1));
        self.urls = next_urls;
    }

    pub fn end(&mut self) {
        self.urls.clear();
        self.selected = None;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashedFile {
    pub item: MediaItem,
    pub trashed_path: PathBuf,
    pub index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashTransaction {
    pub files: Vec<TrashedFile>,
    pub selection_before_op: Vec<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub struct TrashHistory {
    undo_stack: Vec<TrashTransaction>,
    redo_stack: Vec<TrashTransaction>,
}

impl TrashHistory {
    pub fn new() -> Self { Self::default() }
    pub fn can_undo(&self) -> bool { !self.undo_stack.is_empty() }
    pub fn can_redo(&self) -> bool { !self.redo_stack.is_empty() }
    pub fn record_trash(&mut self, transaction: TrashTransaction) {
        if transaction.files.is_empty() { return; }
        self.undo_stack.push(transaction);
        self.redo_stack.clear();
    }
    pub fn push_undo(&mut self, transaction: TrashTransaction) {
        if !transaction.files.is_empty() { self.undo_stack.push(transaction); }
    }
    pub fn push_redo(&mut self, transaction: TrashTransaction) {
        if !transaction.files.is_empty() { self.redo_stack.push(transaction); }
    }
    pub fn pop_undo(&mut self) -> Option<TrashTransaction> { self.undo_stack.pop() }
    pub fn pop_redo(&mut self) -> Option<TrashTransaction> { self.redo_stack.pop() }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrashOutcome {
    Removed { trashed_path: PathBuf },
    AlreadyInProgress,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrashError {
    MissingRecycleMapping(PathBuf),
    AlreadyInProgress(PathBuf),
    DestinationExists(PathBuf),
    MissingTrashedFile(PathBuf),
    RecycleFailed(String),
    MoveFailed(String),
}

impl fmt::Display for TrashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrashError::MissingRecycleMapping(u) => write!(f, "Trash did not return a recycled location for {}", u.display()),
            TrashError::AlreadyInProgress(u) => write!(f, "A trash operation for {} is already in progress", u.display()),
            TrashError::DestinationExists(u) => write!(f, "A file already exists at {}", u.display()),
            TrashError::MissingTrashedFile(u) => write!(f, "The trashed file {} could not be found", u.display()),
            TrashError::RecycleFailed(e) => write!(f, "{e}"),
            TrashError::MoveFailed(e) => write!(f, "{e}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct TrashService {
    in_flight: HashSet<PathBuf>,
}

impl TrashService {
    pub fn new() -> Self { Self { in_flight: HashSet::new() } }

    pub fn move_to_trash<F>(
        &mut self,
        url: &Path,
        recycle: F,
    ) -> Result<TrashOutcome, TrashError>
    where
        F: FnOnce(&Path) -> Result<std::collections::HashMap<PathBuf, PathBuf>, String>,
    {
        if !self.in_flight.insert(url.to_path_buf()) {
            return Ok(TrashOutcome::AlreadyInProgress);
        }
        let result = (|| -> Result<TrashOutcome, TrashError> {
            let mapping = recycle(url).map_err(TrashError::RecycleFailed)?;
            let trashed = mapping.get(url).cloned()
                .ok_or_else(|| TrashError::MissingRecycleMapping(url.to_path_buf()))?;
            Ok(TrashOutcome::Removed { trashed_path: trashed })
        })();
        self.in_flight.remove(url);
        result
    }

    pub fn restore<F>(
        &mut self,
        trashed: &Path,
        original: &Path,
        move_file: F,
    ) -> Result<(), TrashError>
    where
        F: FnOnce(&Path, &Path) -> Result<(), TrashError>,
    {
        if !self.in_flight.insert(original.to_path_buf()) {
            return Err(TrashError::AlreadyInProgress(original.to_path_buf()));
        }
        let result = move_file(trashed, original);
        self.in_flight.remove(original);
        result
    }
}

impl Default for TrashService {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Bounded FIFO-evicting thumbnail cache. Image buffers are owned by the
// caller for decoding; the cache stores byte-costed buckets to cap memory.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThumbCacheKey {
    pub path: PathBuf,
    pub pixel_size: u32,
    pub scale: u32,
    pub modification: Option<u64>,
    pub file_size: Option<i64>,
}

#[derive(Clone)]
struct CacheEntry {
    cost: usize,
    order: u64,
}

pub const THUMBNAIL_CACHE_LIMIT: usize = 96 * 1024 * 1024;

#[derive(Clone)]
pub struct ThumbnailCache {
    entries: HashMap<ThumbCacheKey, CacheEntry>,
    order: Vec<(ThumbCacheKey, u64)>,
    next_index: usize,
    next_sequence: u64,
    tracked_cost: usize,
    limit: usize,
}

impl ThumbnailCache {
    pub fn new(limit: usize) -> Self {
        Self { entries: HashMap::new(), order: Vec::new(), next_index: 0, next_sequence: 0, tracked_cost: 0, limit }
    }

    pub fn with_default_limit() -> Self { Self::new(THUMBNAIL_CACHE_LIMIT) }
    pub fn cost_bytes(&self) -> usize { self.tracked_cost }

    pub fn insert(&mut self, key: ThumbCacheKey, cost: usize) {
        self.remove_entry(&key);
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.entries.insert(key.clone(), CacheEntry { cost, order: sequence });
        self.order.push((key, sequence));
        self.tracked_cost = self.tracked_cost.saturating_add(cost);
        self.evict();
    }

    pub fn contains(&self, key: &ThumbCacheKey) -> bool { self.entries.contains_key(key) }

    pub fn remove_at(&mut self, url: &Path) {
        let keys: Vec<_> = self.entries.keys().filter(|k| k.path == url).cloned().collect();
        for key in keys {
            self.remove_entry(&key);
        }
    }

    pub fn remove_all(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.next_index = 0;
        self.tracked_cost = 0;
    }

    fn remove_entry(&mut self, key: &ThumbCacheKey) {
        if let Some(entry) = self.entries.remove(key) {
            self.tracked_cost = self.tracked_cost.saturating_sub(entry.cost);
        }
    }

    fn evict(&mut self) {
        while self.tracked_cost > self.limit && self.next_index < self.order.len() {
            let (key, sequence) = self.order[self.next_index].clone();
            self.next_index += 1;
            if self.entries.get(&key).map(|e| e.order) != Some(sequence) {
                continue;
            }
            self.remove_entry(&key);
        }
        if self.next_index > 128 && self.next_index * 2 > self.order.len() {
            self.order.drain(..self.next_index);
            self.next_index = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order_numeric_case_insensitive() {
        assert_eq!(natural_cmp("img2.png", "img10.png"), Ordering::Less);
        assert_eq!(natural_cmp("IMG10.png", "img2.png"), Ordering::Greater);
    }

    #[test]
    fn browser_sort_directories_first_then_size_desc() {
        let dir = MediaItem::new("/x/folder", MediaKind::Directory);
        let big = MediaItem { path: PathBuf::from("/x/big.png"), kind: MediaKind::Image, file_size: Some(900), modification: None };
        let small = MediaItem { path: PathBuf::from("/x/small.png"), kind: MediaKind::Image, file_size: Some(10), modification: None };
        let mut items = vec![small.clone(), dir.clone(), big.clone()];
        items.sort_by(browser_order);
        assert_eq!(items, vec![dir, big, small]);
    }

    #[test]
    fn range_selection_focuses_the_clicked_path() {
        let mut catalog = Catalog::new();
        let a = MediaItem::new("/x/a.png", MediaKind::Image);
        let b = MediaItem::new("/x/b.png", MediaKind::Image);
        let c = MediaItem::new("/x/c.png", MediaKind::Image);
        catalog.append(vec![a.clone(), b.clone(), c.clone()]);
        catalog.select(&a.path).unwrap();
        catalog.select_range(&a.path, &c.path).unwrap();
        assert_eq!(catalog.anchor(), Some(a.path.as_path()));
        assert_eq!(catalog.selected(), Some(c.path.as_path()));
        assert_eq!(catalog.selection_bounds(), Some((0, 2)));
        catalog.remove(&c.path);
        assert_eq!(catalog.selected(), Some(a.path.as_path()));
        assert!(catalog.selection_bounds().unwrap().1 < catalog.count());
        assert!(catalog.selected_all().iter().all(|path| catalog.items().iter().any(|item| &item.path == path)));
    }

    #[test]
    fn removing_a_single_selection_keeps_one_live_path() {
        let mut catalog = Catalog::new();
        let items: Vec<_> = ["a", "b", "c"].into_iter().map(|name| MediaItem::new(format!("/x/{name}.png"), MediaKind::Image)).collect();
        catalog.append(items.clone());
        catalog.select(&items[2].path).unwrap();
        catalog.remove(&items[2].path);
        let selected = catalog.selected().unwrap();
        assert!(catalog.items().iter().any(|item| item.path == selected));
        let (start, end) = catalog.selection_bounds().unwrap();
        assert_eq!(start, end);
        assert!(end < catalog.count());
    }

    #[test]
    fn preview_session_rejects_an_empty_selection() {
        let mut catalog = Catalog::new();
        let mut session = PreviewSession::new();
        assert!(!session.begin(&catalog));
        assert_eq!(session.count(), 0);
        assert!(session.item_at(0).is_none());
        catalog.append(vec![MediaItem::new("/x/a.png", MediaKind::Image), MediaItem::new("/x/b.png", MediaKind::Image)]);
        assert!(!session.begin(&catalog));
        catalog.select(Path::new("/x/b.png")).unwrap();
        assert!(session.begin(&catalog));
        catalog.remove(Path::new("/x/b.png"));
        session.update(&catalog);
        let current = session.current_index().unwrap();
        assert_eq!(session.item_at(current), Some(Path::new("/x/a.png")));
        assert!(session.item_at(session.count()).is_none());
        catalog.remove(Path::new("/x/a.png"));
        session.update(&catalog);
        assert_eq!(session.count(), 0);
        assert!(session.item_at(0).is_none());
    }

    #[test]
    fn catalog_preserves_selection_by_identity() {
        let mut catalog = Catalog::new();
        let a = MediaItem::new("/x/a.png", MediaKind::Image);
        let b = MediaItem::new("/x/b.png", MediaKind::Image);
        catalog.append(vec![a.clone(), b.clone()]);
        catalog.select(Path::new("/x/a.png")).unwrap();
        catalog.insert(MediaItem::new("/x/c.png", MediaKind::Image), 1);
        assert_eq!(catalog.selected(), Some(Path::new("/x/a.png")));
        catalog.remove(Path::new("/x/a.png"));
        assert_eq!(catalog.selected(), Some(Path::new("/x/c.png")));
    }

    #[test]
    fn launch_rejects_unknown_options() {
        assert!(resolve_launch(&["--nope".to_string()], "/tmp").is_err());
    }

    #[test]
    fn history_new_trash_clears_redo() {
        let mut history = TrashHistory::default();
        let file = TrashedFile { item: MediaItem::new("/x/a.png", MediaKind::Image), trashed_path: PathBuf::from("/t/a.png"), index: 0 };
        history.record_trash(TrashTransaction { files: vec![file.clone()], selection_before_op: vec![PathBuf::from("/x/a.png")] });
        history.push_redo(TrashTransaction { files: vec![file.clone()], selection_before_op: vec![] });
        assert!(history.can_redo());
        history.record_trash(TrashTransaction { files: vec![file], selection_before_op: vec![] });
        assert!(!history.can_redo());
    }

    #[test]
    fn cache_evicts_oldest_by_cost() {
        let mut cache = ThumbnailCache::new(100);
        let key = |name: &str| ThumbCacheKey { path: PathBuf::from(name), pixel_size: 320, scale: 2, modification: None, file_size: None };
        cache.insert(key("/a.png"), 60);
        cache.insert(key("/b.png"), 60);
        assert!(!cache.contains(&key("/a.png")));
        assert!(cache.contains(&key("/b.png")));
        assert_eq!(cache.cost_bytes(), 60);
    }

    #[test]
    fn preview_session_tracks_identity_across_catalog_change() {
        let mut catalog = Catalog::new();
        let a = MediaItem::new("/x/a.png", MediaKind::Image);
        let b = MediaItem::new("/x/b.png", MediaKind::Image);
        catalog.append(vec![a.clone(), b.clone()]);
        catalog.select(&a.path).unwrap();
        let mut session = PreviewSession::new();
        assert!(session.begin(&catalog));
        catalog.remove(&a.path);
        session.update(&catalog);
        assert_eq!(session.item_at(0), Some(Path::new("/x/b.png")));
    }

    #[test]
    fn grid_commands_resolve() {
        assert_eq!(resolve_command(" ", &[]), Some(GridCommand::TogglePreview));
        assert_eq!(resolve_command("z", &[GridKeyModifier::Command]), Some(GridCommand::Undo));
        assert_eq!(resolve_command("Z", &[GridKeyModifier::Command, GridKeyModifier::Shift]), Some(GridCommand::Redo));
    }

    #[test]
    fn extension_detection_matches_swift_media_surface() {
        assert_eq!(kind_for_extension(Path::new("/x/photo.avif")), Some(MediaKind::Image));
        assert_eq!(kind_for_extension(Path::new("/x/clip.mkv")), Some(MediaKind::Movie));
        assert_eq!(kind_for_extension(Path::new("/x/notes.txt")), None);
    }

    #[test]
    fn package_detection_covers_iwork_and_system_bundles() {
        assert!(is_package(Path::new("/x/app.pages")));
        assert!(is_package(Path::new("/x/Foo.app")));
        assert!(!is_package(Path::new("/x/photo.png")));
    }

    #[test]
    fn content_sniffing_detects_png_without_extension() {
        let dir = std::env::temp_dir().join(format!("view-sniff-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("noext");
        std::fs::write(&path, [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]).unwrap();
        assert_eq!(kind_for_extension(&path), Some(MediaKind::Image));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scheduler_execute_pending_respects_limit_and_cancel() {
        let mut scheduler = Scheduler::new(2);
        let a = ThumbKey { path: PathBuf::from("/a.png"), pixel_size: 320, scale: 2 };
        let b = ThumbKey { path: PathBuf::from("/b.png"), pixel_size: 320, scale: 2 };
        let c = ThumbKey { path: PathBuf::from("/c.png"), pixel_size: 320, scale: 2 };
        scheduler.update_demand(vec![a.clone(), b.clone()], vec![c.clone()]);
        assert_eq!(scheduler.pending().len(), 2);
        let done: Vec<_> = scheduler.execute_pending(|_| {}).into_iter().map(|(k, _)| k.path).collect();
        assert_eq!(done.len(), 2);
        scheduler.cancel_all();
        assert!(scheduler.pending().is_empty());
        assert!(scheduler.generation_of(&a).is_none());
    }


    #[test]
    fn decode_png_native_fills_solid_context() {
        use std::os::raw::c_void;
        let mut pixels = vec![0u8; 4 * 4 * 4];
        unsafe {
            let cs = cg_color_space_create_device_rgb();
            let ctx = cg_context_create(pixels.as_mut_ptr() as *mut c_void, 4, 4, 8, 16, cs, 1);
            assert!(!ctx.is_null());
            cg_context_set_rgb_fill_color(ctx, 0.0, 1.0, 0.0, 1.0);
            let rect = CGRect {
                x: 0.0,
                y: 0.0,
                width: 4.0,
                height: 4.0,
            };
            cg_context_fill_rect(ctx, rect);
            cg_context_synchronize(ctx);
            cg_context_release(ctx);
            cg_color_space_release(cs);
        }
        assert!(pixels.iter().any(|b| *b != 0), "fill must write to the buffer: {pixels:?}");
    }

    #[test]
    fn decode_png_native_reads_pixels_through_imageio() {
        let corpus = "/Users/josh/d/view/.work/corpus-2k/group-00";
        let mut found = None;
        if let Ok(entries) = std::fs::read_dir(corpus) {
            for entry in entries.filter_map(|e| e.ok()) {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("png") {
                    found = Some(p);
                    break;
                }
            }
        }
        let Some(path) = found else {
            return;
        };
        let pool = unsafe { objc2_foundation::NSAutoreleasePool::new() };
        let (w, h, pixels) = decode_png_native(&path).unwrap();
        assert!(w > 0 && h > 0);
        assert_eq!(pixels.len(), w as usize * h as usize * 4);
        assert!(pixels.iter().any(|b| *b != 0), "native pixels must be non-zero");
        drop(pool);
        let (pw, ph, pure_pixels) = decode_png(&path).unwrap();
        assert_eq!((pw, ph), (w, h));
        // ImageIO RGBA matches pure RGB modulo the alpha byte and the
        // bitmap is bottom-up: source row 0 (top-left) lands on the last
        // buffer row. Sample top-left through both decoders.
        let row = (h as usize) - 1;
        let offset = row * (w as usize) * 4;
        let native_top_left = [
            pixels[offset],
            pixels[offset + 1],
            pixels[offset + 2],
            pixels[offset + 3],
        ];
        assert_eq!(
            native_top_left[..3].to_vec(),
            pure_pixels[..3].to_vec(),
            "top-left pixel differs: native {native_top_left:?} pure {:?}",
            &pure_pixels[..3]
        );
        assert_eq!(native_top_left[3], 255, "drawn pixels must be opaque");
    }

    #[test]
    fn decode_png_bytes_yields_pixels_for_minimal_rgb_png() {
        // Construct the PNG through zlib directly to avoid any encoder dep:
        let raw_scanline = [0u8, 255, 0, 0];
        let mut zlib = Vec::new();
        {
            use flate2::{Compression, write::ZlibEncoder};
            use std::io::Write;
            let mut encoder = ZlibEncoder::new(&mut zlib, Compression::default());
            encoder.write_all(&raw_scanline).unwrap();
            encoder.finish().unwrap();
        }
        fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
            let len = (body.len() as u32).to_be_bytes();
            out.extend_from_slice(&len);
            out.extend_from_slice(kind);
            out.extend_from_slice(body);
            let mut crc = kind.to_vec();
            crc.extend_from_slice(body);
            let crc = {
                let mut table = [0u32; 256];
                for (n, slot) in table.iter_mut().enumerate() {
                    let mut v = n as u32;
                    for _ in 0..8 {
                        v = if (v & 1) != 0 { 0xEDB88320 ^ (v >> 1) } else { v >> 1 };
                    }
                    *slot = v;
                }
                let mut c = 0xFFFFFFFFu32;
                let mut crc_data = kind.to_vec();
                crc_data.extend_from_slice(body);
                for b in crc_data {
                    c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
                }
                c ^ 0xFFFFFFFF
            };
            out.extend_from_slice(&crc.to_be_bytes());
        }
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        chunk(&mut png, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]);
        chunk(&mut png, b"IDAT", &zlib);
        chunk(&mut png, b"IEND", &[]);
        let (w, h, pixels) = decode_png_bytes(&png).unwrap();
        assert_eq!((w, h), (1, 1));
        assert_eq!(pixels, vec![255, 0, 0]);
    }

    #[test]
    fn trash_undo_redo_round_trip_through_history() {
        let dir = std::env::temp_dir().join(format!("view-trash-{}", std::process::id()));
        let trash = dir.join("trash");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::create_dir_all(&trash);
        let original = dir.join("a.png");
        std::fs::write(&original, b"hello").unwrap();
        let mut service = TrashService::new();
        let trashed = trash.join("a.png");
        let outcome = service
            .move_to_trash(&original, |_url| {
                std::fs::rename(&original, &trashed).map_err(|e| e.to_string())?;
                Ok(std::collections::HashMap::from([(original.clone(), trashed.clone())]))
            })
            .unwrap();
        assert_eq!(outcome, TrashOutcome::Removed { trashed_path: trashed.clone() });
        assert!(!original.exists());
        assert!(trashed.exists());
        let mut history = TrashHistory::new();
        let item = MediaItem::new(&original, MediaKind::Image);
        history.record_trash(TrashTransaction {
            files: vec![TrashedFile { item: item.clone(), trashed_path: trashed.clone(), index: 0 }],
            selection_before_op: vec![original.clone()],
        });
        assert!(history.can_undo());
        let txn = history.pop_undo().unwrap();
        service
            .restore(&txn.files[0].trashed_path, &txn.files[0].item.path, |from, to| {
                std::fs::rename(from, to).map_err(|e| TrashError::MoveFailed(e.to_string()))
            })
            .unwrap();
        assert!(original.exists());
        assert!(!trashed.exists());
        history.push_redo(TrashTransaction { files: txn.files, selection_before_op: vec![] });
        assert!(history.can_redo());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
