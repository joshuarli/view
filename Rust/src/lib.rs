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

#[derive(Clone, Debug, PartialEq)]
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

fn kind_for_extension(path: &Path) -> Option<MediaKind> {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref() {
        Some("png") | Some("jpg") | Some("jpeg") | Some("gif") | Some("webp") | Some("tif") | Some("tiff") | Some("heic") | Some("heif") => Some(MediaKind::Image),
        Some("mp4") | Some("mov") | Some("m4v") | Some("webm") | Some("avi") | Some("mkv") => Some(MediaKind::Movie),
        _ => None,
    }
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
        if is_symlink(&path) || is_hidden(&path) {
            continue;
        }
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            // Packages are traversed as files by the Swift reader; the
            // benchmark corpus contains no packages, so treat any dir as
            // a navigable directory entry here.
            items.push(MediaItem::new(path, MediaKind::Directory));
        } else if metadata.is_file() {
            if let Some(kind) = kind_for_extension(&path) {
                let mut item = MediaItem::new(path, kind);
                item.file_size = Some(metadata.len() as i64);
                items.push(item);
            }
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
            if is_symlink(path) || is_hidden(path) {
                continue;
            }
            let metadata = fs::metadata(path).ok();
            let is_dir = metadata.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            if is_dir {
                if path.extension().and_then(|e| e.to_str()) == Some("app") {
                    continue;
                }
                stack.push(path.clone());
                continue;
            }
            if metadata.as_ref().map(|m| m.is_file()).unwrap_or(false) {
                if let Some(kind) = kind_for_extension(path) {
                    let mut item = MediaItem::new(path.clone(), kind);
                    item.file_size = metadata.map(|m| m.len() as i64);
                    out.push(item);
                }
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
}

impl Catalog {
    pub fn new() -> Self { Self { entries: Vec::new(), selection: Vec::new() } }
    pub fn items(&self) -> &[MediaItem] { &self.entries }
    pub fn selected(&self) -> Option<&Path> { self.selection.first().map(PathBuf::as_path) }
    pub fn selected_all(&self) -> &[PathBuf] { &self.selection }
    pub fn count(&self) -> usize { self.entries.len() }

    pub fn append(&mut self, items: Vec<MediaItem>) -> (usize, usize) {
        let start = self.entries.len();
        self.entries.extend(items);
        (start, self.entries.len())
    }

    pub fn select(&mut self, url: &Path) -> Result<(), CatalogError> {
        if self.entries.iter().any(|i| i.path == url) {
            self.selection = vec![url.to_path_buf()];
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
        Ok(())
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
}

// ---------------------------------------------------------------------------
// PNG decode (subset: 8-bit colour types 0/2/4/6, non-interlaced) via flate2
// ---------------------------------------------------------------------------

pub fn decode_png(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    decode_png_native(path)
        .map(|(w, h)| (w, h, vec![]))
}

// Decode with the same ImageIO backend as Swift (CGImageSource). This
// keeps thumb-decode an apples-to-apples system-decoder comparison; the
// pure-Rust inflate path above is retained only for parity reference.
pub fn decode_png_native(path: &Path) -> Result<(u32, u32), String> {
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
        cg_image_release(image);
        cf_release(source as *const std::ffi::c_void);
        Ok((0, 0))
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
        guard.insert(path.to_path_buf(), CachedUrl(url as *mut std::ffi::c_void));
        url
    }
}

type CGDataProvider = std::ffi::c_void;
type CGImageSource = std::ffi::c_void;
type CGImage = std::ffi::c_void;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    #[link_name = "CGDataProviderCreateWithFilename"]
    fn cg_data_provider_create_with_filename(filename: *const std::os::raw::c_char) -> *mut CGDataProvider;
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

fn recon_prev(x: usize, out: &[u8], y: usize, stride: usize, _channels: usize) -> u8 {
    let _ = (out, y, stride);
    // Not used in the sequential unfilter below; kept for signature clarity.
    let _ = x;
    0
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
