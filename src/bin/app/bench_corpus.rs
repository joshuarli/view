use std::path::{Path, PathBuf};

const VERSION: &str = "1";
const STILL_WIDTH: usize = 1920;
const STILL_HEIGHT: usize = 1080;
const STILL_QUALITY: f32 = 0.55;
const SMALL_COUNT: usize = 42;
const TYPICAL_COUNT: usize = 160;
const LARGE_GROUPS: usize = 33;
const LARGE_PER_GROUP: usize = 100;
const NONE_SKIP_LAST: u32 = 5;

#[derive(Clone)]
pub struct Corpus {
    pub small: PathBuf,
    pub typical: PathBuf,
    pub large: PathBuf,
}

pub fn ensure() -> Corpus {
    static READY: std::sync::OnceLock<Corpus> = std::sync::OnceLock::new();
    READY.get_or_init(open_or_write).clone()
}

fn open_or_write() -> Corpus {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".work/bench-corpus");
    let published = root.join("current");
    if let Some(corpus) = published_corpus(&published) {
        return corpus;
    }
    let partial = root.join(format!("partial-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&partial);
    std::fs::create_dir_all(&partial).expect("create benchmark corpus directory");
    let cleanup = RemoveOnDrop(&partial);
    write_corpus(&partial);
    if let Some(corpus) = published_corpus(&published) {
        drop(cleanup);
        let _ = std::fs::remove_dir_all(&partial);
        return corpus;
    }
    let displaced = root.join(format!("displaced-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&displaced);
    if published.exists() {
        std::fs::rename(&published, &displaced).expect("move previous benchmark corpus aside");
    }
    match std::fs::rename(&partial, &published) {
        Ok(()) => {
            let _ = std::fs::remove_dir_all(&displaced);
        }
        Err(error) => {
            let _ = std::fs::rename(&displaced, &published);
            if let Some(corpus) = published_corpus(&published) {
                return corpus;
            }
            panic!("publish benchmark corpus: {error}");
        }
    }
    published_corpus(&published).expect("published benchmark corpus")
}

struct RemoveOnDrop<'a>(&'a Path);

impl Drop for RemoveOnDrop<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0);
    }
}

fn published_corpus(root: &Path) -> Option<Corpus> {
    let stamp = std::fs::read(root.join("READY")).ok()?;
    if stamp != VERSION.as_bytes() {
        return None;
    }
    let corpus = Corpus {
        small: root.join("small"),
        typical: root.join("typical"),
        large: root.join("large"),
    };
    if corpus.small.is_dir() && corpus.typical.is_dir() && corpus.large.is_dir() {
        Some(corpus)
    } else {
        None
    }
}

fn write_corpus(root: &Path) {
    let small = root.join("small");
    let typical = root.join("typical");
    let large = root.join("large");
    let mut pixels = vec![0u8; STILL_WIDTH * STILL_HEIGHT * 4];
    let color = cg_color_space_create_device_rgb();
    assert!(!color.is_null(), "device RGB color space");
    write_stills(&small, SMALL_COUNT, 2, &mut pixels, color);
    write_stills(&typical, TYPICAL_COUNT, 3, &mut pixels, color);
    std::fs::write(typical.join("clip.mp4"), include_bytes!("../../../tests/fixtures/carousel.mp4")).expect("write benchmark movie");
    let template = encode_template(root, color);
    cg_color_space_release(color);
    write_large(&large, &template);
    let small_items = view_core::list_browser(&small).expect("list small corpus");
    assert_eq!(small_items.len(), SMALL_COUNT, "small corpus file count");
    assert_eq!(small_items[0].kind, view_core::MediaKind::Image, "small corpus starts with an image");
    let typical_items = view_core::list_browser(&typical).expect("list typical corpus");
    assert_eq!(typical_items.iter().filter(|item| item.kind == view_core::MediaKind::Image).count(), TYPICAL_COUNT);
    assert_eq!(typical_items.iter().filter(|item| item.kind == view_core::MediaKind::Movie).count(), 1);
    assert!(
        typical_items.windows(2).any(|pair| pair[0].kind == view_core::MediaKind::Image && pair[1].kind == view_core::MediaKind::Image),
        "typical corpus needs two adjacent images"
    );
    let large_items = view_core::scan_recursive(&large).expect("scan large corpus");
    assert_eq!(large_items.len(), LARGE_GROUPS * LARGE_PER_GROUP, "large corpus file count");
    std::fs::write(root.join("READY"), VERSION).expect("stamp benchmark corpus");
}

fn write_stills(dir: &Path, count: usize, width: usize, pixels: &mut [u8], color: *mut std::ffi::c_void) {
    std::fs::create_dir_all(dir).expect("create still directory");
    for index in 0..count {
        fill_still(pixels, 0xC0FFEE ^ (index as u64).wrapping_mul(0x9E37_79B9));
        let name = format!("still-{index:0width$}.jpg");
        write_jpeg(&dir.join(name), pixels, STILL_WIDTH, STILL_HEIGHT, color, STILL_QUALITY);
    }
}

fn encode_template(root: &Path, color: *mut std::ffi::c_void) -> Vec<u8> {
    let scratch = root.join("template.jpg");
    let mut pixels = vec![0u8; 16 * 16 * 4];
    fill_still(&mut pixels, 1);
    write_jpeg(&scratch, &pixels, 16, 16, color, 0.5);
    let bytes = std::fs::read(&scratch).expect("read template jpeg");
    std::fs::remove_file(&scratch).expect("remove template jpeg");
    assert!(bytes.starts_with(&[0xFF, 0xD8]), "template jpeg starts with SOI");
    bytes
}

fn write_large(dir: &Path, template: &[u8]) {
    std::fs::create_dir_all(dir).expect("create large corpus");
    let mut id = 0u32;
    for group in 0..LARGE_GROUPS {
        let group_dir = dir.join(format!("group-{group:02}"));
        std::fs::create_dir_all(&group_dir).expect("create large corpus group");
        for index in 0..LARGE_PER_GROUP {
            std::fs::write(group_dir.join(format!("still-{index:03}.jpg")), with_comment(template, id)).expect("write large corpus file");
            id += 1;
        }
    }
}

fn with_comment(template: &[u8], id: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(template.len() + 6);
    out.extend_from_slice(&[0xFF, 0xD8, 0xFF, 0xFE, 0x00, 0x06]);
    out.extend_from_slice(&id.to_be_bytes());
    out.extend_from_slice(&template[2..]);
    out
}

fn fill_still(pixels: &mut [u8], seed: u64) {
    let mut state = seed | 1;
    for px in pixels.chunks_exact_mut(4) {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        px[0] = (state >> 33) as u8;
        px[1] = (state >> 41) as u8;
        px[2] = (state >> 49) as u8;
        px[3] = 255;
    }
}

fn write_jpeg(path: &Path, pixels: &[u8], width: usize, height: usize, color: *mut std::ffi::c_void, quality: f32) {
    assert_eq!(pixels.len(), width * height * 4, "jpeg buffer is tightly packed");
    // SAFETY: `pixels` outlives the provider, which is released before this
    // function returns. The dictionary callback pointers are the process-wide
    // CoreFoundation statics. Each created object is released on this path.
    unsafe {
        let provider = CGDataProviderCreateWithData(std::ptr::null_mut(), pixels.as_ptr(), pixels.len(), None);
        assert!(!provider.is_null(), "image data provider");
        let image = CGImageCreate(width, height, 8, 32, width * 4, color, NONE_SKIP_LAST, provider, std::ptr::null(), false, 0);
        assert!(!image.is_null(), "benchmark image");
        let url = cf_url(path);
        let kind = cf_string("public.jpeg");
        let key = kCGImageDestinationLossyCompressionQuality;
        let number = super::super::CFNumberCreate(std::ptr::null(), 5, (&quality as *const f32).cast());
        assert!(!number.is_null(), "jpeg quality");
        let dict = CFDictionaryCreate(
            std::ptr::null(),
            &key,
            &number,
            1,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        );
        assert!(!dict.is_null(), "jpeg properties");
        let dest = CGImageDestinationCreateWithURL(url, kind, 1, std::ptr::null());
        assert!(!dest.is_null(), "jpeg destination");
        assert!(CGImageDestinationAddImage(dest, image, dict), "add jpeg image");
        assert!(CGImageDestinationFinalize(dest), "finalize jpeg");
        CFRelease(dest.cast());
        CFRelease(dict);
        CFRelease(number);
        CFRelease(kind);
        CFRelease(url);
        CGImageRelease(image);
        CGDataProviderRelease(provider);
    }
}

fn cf_url(path: &Path) -> *const std::ffi::c_void {
    let bytes = path.as_os_str().as_encoded_bytes();
    assert!(!bytes.contains(&0), "benchmark path contains NUL");
    let url = cf_url_create_from_file_system_representation(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize, false);
    assert!(!url.is_null(), "benchmark file url");
    url
}

fn cf_string(value: &str) -> *const std::ffi::c_void {
    let c = std::ffi::CString::new(value).expect("cf string");
    let string = cf_string_create_with_c_string(std::ptr::null(), c.as_ptr(), 0x0800_0100);
    assert!(!string.is_null(), "cf string");
    string
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const std::ffi::c_void);
    fn CFStringCreateWithCString(allocator: *const std::ffi::c_void, c_string: *const std::os::raw::c_char, encoding: u32) -> *const std::ffi::c_void;
    fn CFURLCreateFromFileSystemRepresentation(allocator: *const std::ffi::c_void, buffer: *const u8, len: isize, is_directory: bool) -> *const std::ffi::c_void;
    fn CFDictionaryCreate(
        allocator: *const std::ffi::c_void,
        keys: *const *const std::ffi::c_void,
        values: *const *const std::ffi::c_void,
        count: isize,
        key_callbacks: *const u8,
        value_callbacks: *const u8,
    ) -> *const std::ffi::c_void;
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGColorSpaceCreateDeviceRGB() -> *mut std::ffi::c_void;
    fn CGColorSpaceRelease(color: *mut std::ffi::c_void);
    fn CGDataProviderCreateWithData(
        info: *mut std::ffi::c_void,
        data: *const u8,
        size: usize,
        release: Option<unsafe extern "C" fn(*mut std::ffi::c_void, *const u8, usize)>,
    ) -> *mut std::ffi::c_void;
    fn CGDataProviderRelease(provider: *mut std::ffi::c_void);
    fn CGImageCreate(
        width: usize,
        height: usize,
        bits_per_component: usize,
        bits_per_pixel: usize,
        bytes_per_row: usize,
        space: *mut std::ffi::c_void,
        bitmap_info: u32,
        provider: *mut std::ffi::c_void,
        decode: *const f64,
        should_interpolate: bool,
        intent: i32,
    ) -> *mut std::ffi::c_void;
    fn CGImageRelease(image: *mut std::ffi::c_void);
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    static kCGImageDestinationLossyCompressionQuality: *const std::ffi::c_void;
    fn CGImageDestinationCreateWithURL(url: *const std::ffi::c_void, kind: *const std::ffi::c_void, count: usize, options: *const std::ffi::c_void) -> *mut std::ffi::c_void;
    fn CGImageDestinationAddImage(dest: *mut std::ffi::c_void, image: *mut std::ffi::c_void, properties: *const std::ffi::c_void) -> bool;
    fn CGImageDestinationFinalize(dest: *mut std::ffi::c_void) -> bool;
}

fn cf_string_create_with_c_string(allocator: *const std::ffi::c_void, c_string: *const std::os::raw::c_char, encoding: u32) -> *const std::ffi::c_void {
    unsafe { CFStringCreateWithCString(allocator, c_string, encoding) }
}

fn cf_url_create_from_file_system_representation(allocator: *const std::ffi::c_void, buffer: *const u8, len: isize, is_directory: bool) -> *const std::ffi::c_void {
    unsafe { CFURLCreateFromFileSystemRepresentation(allocator, buffer, len, is_directory) }
}

fn cg_color_space_create_device_rgb() -> *mut std::ffi::c_void {
    unsafe { CGColorSpaceCreateDeviceRGB() }
}

fn cg_color_space_release(color: *mut std::ffi::c_void) {
    unsafe { CGColorSpaceRelease(color) }
}


