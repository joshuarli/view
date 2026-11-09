//! Rust port GUI binary (see Rust/PARITY.md).
//!
//! Native AppKit host built on `objc2-app-kit` (the same frameworks as
//! the Swift app): window + scroll view + custom grid view, wired to
//! the `view_core` discovery/catalog state.

use std::path::PathBuf;

use objc2::{define_class, ClassType, DefinedClass, MainThreadOnly};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::NSObject;
use objc2_app_kit::{
    NSApplication, NSApplicationDelegate, NSBackingStoreType, NSColor, NSFont, NSImage,
    NSScrollView, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSData, NSDictionary, NSNotification, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use view_core::{list_browser, scan_recursive, BrowserMode, MediaKind, MediaItem};

const CELL_W: f64 = 160.0;
const CELL_H: f64 = 180.0;
const SPACING: f64 = 10.0;

#[derive(Default)]
struct AppState {
    root: PathBuf,
    mode: BrowserMode,
    items: Vec<MediaItem>,
    selected: Option<usize>,
    back: Vec<(PathBuf, Option<PathBuf>)>,
    forward: Vec<(PathBuf, Option<PathBuf>)>,
}

struct SyncCell(std::cell::UnsafeCell<AppState>);
unsafe impl Sync for SyncCell {}

static STATE: std::sync::OnceLock<SyncCell> = std::sync::OnceLock::new();

fn app_state() -> *mut AppState {
    let cell = STATE.get_or_init(|| SyncCell(std::cell::UnsafeCell::new(AppState::default())));
    cell.0.get()
}

fn columns_for(width: f64) -> usize {
    let available = (width - 32.0).max(0.0);
    ((available + SPACING) / (CELL_W + SPACING)).floor().max(1.0) as usize
}

#[allow(dead_code)]
fn content_size(width: f64, count: usize, carousel: bool) -> NSSize {
    if carousel {
        NSSize::new(((count as f64) * (120.0 + 8.0) + 24.0).max(width), 184.0)
    } else {
        let cols = columns_for(width).max(1);
        let rows = count.div_ceil(cols);
        NSSize::new(width, rows as f64 * (CELL_H + SPACING) + 32.0)
    }
}

fn load_listing(root: &PathBuf, mode: BrowserMode) -> Vec<MediaItem> {
    match mode {
        BrowserMode::Browser => list_browser(root).unwrap_or_default(),
        BrowserMode::Recursive => scan_recursive(root).unwrap_or_default(),
    }
}

fn system_image(name: &str) -> Option<Retained<NSImage>> {
    let ns_name = NSString::from_str(name);
    unsafe { NSImage::imageWithSystemSymbolName_accessibilityDescription(&ns_name, None) }
}

fn image_for(item: &MediaItem) -> Option<Retained<NSImage>> {
    let data = std::fs::read(&item.path).ok()?;
    if data.is_empty() {
        return None;
    }
    let mtm = objc2::MainThreadMarker::new()?;
    let nsdata = unsafe { NSData::initWithBytes_length(mtm.alloc(), data.as_ptr().cast(), data.len()) };
    unsafe { NSImage::initWithData(mtm.alloc(), &nsdata) }
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    struct GridView;

    impl GridView {
        #[unsafe(method_id(initWithFrame:))]
        fn init_with_frame(this: Allocated<Self>, frame: NSRect) -> Retained<Self> {
            let this = this.set_ivars(());
            unsafe {
                let this: Retained<Self> = objc2::msg_send![super(this), initWithFrame: frame];
                this.setWantsLayer(true);
                this
            }
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _rect: NSRect) {
            let state = unsafe { &mut *app_state() };
            let width = self.bounds().size.width;
            let cols = columns_for(width).max(1);
            let selected = state.selected;
            let mtm = objc2::MainThreadMarker::new().unwrap();
            for (i, item) in state.items.iter().enumerate() {
                let (x, y, cw, ch) = if false {
                    (0.0, 0.0, 0.0, 0.0)
                } else {
                    let col = i % cols;
                    let row = i / cols;
                    (
                        16.0 + col as f64 * (CELL_W + SPACING),
                        16.0 + row as f64 * (CELL_H + SPACING),
                        CELL_W,
                        CELL_H,
                    )
                };
                let cell = NSRect::new(NSPoint::new(x, y), NSSize::new(cw, ch));
                if selected == Some(i) {
                    unsafe {
                        NSColor::selectedContentBackgroundColor().setFill();
                        objc2_app_kit::NSRectFill(cell);
                    }
                }
                let thumb_rect = NSRect::new(NSPoint::new(x + 6.0, y + 6.0), NSSize::new(cw - 12.0, ch - 36.0));
                match image_for(item) {
                    Some(image) => unsafe {
                        image.drawInRect_fromRect_operation_fraction(
                            thumb_rect, NSRect::ZERO, objc2_app_kit::NSCompositingOperation::SourceOver, 1.0
                        );
                    },
                    None => {
                        let name = match item.kind {
                            MediaKind::Directory => "folder",
                            MediaKind::Image => "photo",
                            MediaKind::Movie => "film",
                        };
                        if let Some(image) = system_image(name) {
                            unsafe {
                                image.drawInRect_fromRect_operation_fraction(
                                    thumb_rect, NSRect::ZERO, objc2_app_kit::NSCompositingOperation::SourceOver, 1.0
                                );
                            }
                        }
                    }
                }
                let label_rect = NSRect::new(NSPoint::new(x + 6.0, y + ch - 28.0), NSSize::new(cw - 12.0, 22.0));
                let name = item.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let string = NSString::from_str(&name);
                let font = unsafe { NSFont::systemFontOfSize(12.0) };
                let attrs: Retained<objc2_foundation::NSDictionary<objc2_foundation::NSAttributedStringKey, NSObject>> = unsafe {
                    objc2_foundation::NSDictionary::dictionaryWithObject_forKey(
                        &*font as &NSObject,
                        objc2::runtime::ProtocolObject::from_ref(unsafe { objc2_app_kit::NSFontAttributeName } as &NSString),
                    )
                };
                let _ = mtm;
                unsafe {
                    let _size: NSSize = objc2::msg_send![&*string, drawInRect: label_rect, withAttributes: &*attrs];
                }
            }
            let _ = state;
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &objc2_app_kit::NSEvent) {
            let state = unsafe { &mut *app_state() };
            let code = event.keyCode();
            let mods = event.modifierFlags();
            let shift = mods.contains(objc2_app_kit::NSEventModifierFlags::Shift);
            let command = mods.contains(objc2_app_kit::NSEventModifierFlags::Command);
            // History (shift arrows) — browser mode only.
            if code == 123 && shift {
                if let Some((url, sel)) = state.back.pop() {
                    let current = state.root.clone();
                    state.forward.push((current, state.selected.and_then(|i| state.items.get(i)).map(|i| i.path.clone())));
                    state.root = url;
                    state.items = load_listing(&state.root, state.mode);
                    state.selected = sel.and_then(|s| state.items.iter().position(|i| i.path == s));
                }
                self.setNeedsDisplay(true);
                return;
            }
            if code == 124 && shift {
                if let Some((url, sel)) = state.forward.pop() {
                    let current = state.root.clone();
                    state.back.push((current, state.selected.and_then(|i| state.items.get(i)).map(|i| i.path.clone())));
                    state.root = url;
                    state.items = load_listing(&state.root, state.mode);
                    state.selected = sel.and_then(|s| state.items.iter().position(|i| i.path == s));
                }
                self.setNeedsDisplay(true);
                return;
            }
            if state.items.is_empty() {
                return;
            }
            let cols = columns_for(self.bounds().size.width).max(1);
            let cur = state.selected.unwrap_or(0);
            let next = match code {
                123 | 126 => cur as isize - if code == 126 { cols as isize } else { 1 },
                124 | 125 => cur as isize + if code == 125 { cols as isize } else { 1 },
                36 => {
                    // Return: enter directory.
                    if let Some(item) = state.items.get(cur) {
                        if item.kind == MediaKind::Directory {
                            let current = state.root.clone();
                            state.back.push((current, state.selected.and_then(|i| state.items.get(i)).map(|x| x.path.clone())));
                            state.forward.clear();
                            state.root = item.path.clone();
                            state.items = load_listing(&state.root, state.mode);
                            state.selected = if state.items.is_empty() { None } else { Some(0) };
                            self.setNeedsDisplay(true);
                            return;
                        }
                    }
                    return;
                }
                _ => return,
            };
            let n = state.items.len() as isize;
            let wrapped = ((next % n) + n) % n;
            state.selected = Some(wrapped as usize);
            self.setNeedsDisplay(true);
            let _ = command;
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &objc2_app_kit::NSEvent) {
            let location = event.locationInWindow();
            let view_location = self.convertPoint_fromView(location, None);
            let state = unsafe { &mut *app_state() };
            if state.items.is_empty() {
                return;
            }
            let width = self.bounds().size.width;
            let cols = columns_for(width).max(1);
            let x = view_location.x - 16.0;
            let y = view_location.y - 16.0;
            if x < 0.0 || y < 0.0 {
                return;
            }
            let col = (x / (CELL_W + SPACING)).floor() as usize;
            let row = (y / (CELL_H + SPACING)).floor() as usize;
            if col >= cols { return; }
            let index = row * cols + col;
            if event.clickCount() >= 2 {
                if let Some(item) = state.items.get(index) {
                    if item.kind == MediaKind::Directory {
                        let current = state.root.clone();
                        state.back.push((current, state.selected.and_then(|i| state.items.get(i)).map(|v| v.path.clone())));
                        state.forward.clear();
                        state.root = item.path.clone();
                        state.items = load_listing(&state.root, state.mode);
                        state.selected = if state.items.is_empty() { None } else { Some(0) };
                    }
                }
            } else if index < state.items.len() {
                state.selected = Some(index);
            }
            self.setNeedsDisplay(true);
        }
    }
);

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = AppState]
    struct AppDelegate;

    impl AppDelegate {
        #[unsafe(method_id(init))]
        fn init(this: Allocated<Self>) -> Retained<Self> {
            let this = this.set_ivars(AppState::default());
            unsafe { objc2::msg_send![super(this), init] }
        }
    }

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            let (title, frame_size, item_count) = {
                let state = unsafe { &*app_state() };
                (
                    state.root.display().to_string(),
                    NSSize::new(1100.0, 760.0),
                    state.items.len(),
                )
            };
            let frame = NSRect::new(NSPoint::new(0.0, 0.0), frame_size);
            let style = NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable;
            let window = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(objc2::MainThreadMarker::new().unwrap()),
                    frame,
                    style,
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            let scroll = NSScrollView::new(objc2::MainThreadMarker::new().unwrap());
            scroll.setHasVerticalScroller(true);
            scroll.setHasHorizontalScroller(false);
            let grid: Retained<GridView> = unsafe {
                objc2::msg_send![GridView::alloc(objc2::MainThreadMarker::new().unwrap()), initWithFrame: frame]
            };
            let doc_size = content_size(frame_size.width, item_count, false);
            grid.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), doc_size));
            grid.setNeedsDisplay(true);
            scroll.setDocumentView(Some(&*grid));
            window.setContentView(Some(&*scroll));
            window.setTitle(&NSString::from_str(&title));
            window.center();
            window.makeKeyAndOrderFront(None);
            let _ = window.makeFirstResponder(Some(&*grid));
            let _ = window;
        }
    }
);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let current = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    let (root, mode) = match view_core::resolve_launch(&args, &current.display().to_string()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("view: {e}");
            std::process::exit(1);
        }
    };
    {
        let state = unsafe { &mut *app_state() };
        state.root = root.clone();
        state.mode = mode;
        state.items = load_listing(&root, mode);
        state.selected = if state.items.is_empty() { None } else { Some(0) };
    }

    let mtm = objc2::MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    let delegate: Retained<AppDelegate> = unsafe { objc2::msg_send![AppDelegate::class(), new] };
    app.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*delegate)));
    app.run();
}
