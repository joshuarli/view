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
    NSApplication, NSApplicationDelegate, NSBackingStoreType, NSBox, NSColor, NSFont, NSImage,
    NSImageView, NSMenu, NSMenuItem, NSScrollView, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSData, NSDictionary, NSNotification, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use view_core::{list_browser, scan_recursive, BrowserMode, MediaKind, MediaItem};

const CELL_W: f64 = 160.0;
const CELL_H: f64 = 180.0;
const SPACING: f64 = 10.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode { Carousel, Grid }

impl Default for ViewMode {
    fn default() -> Self { Self::Carousel }
}

#[derive(Default)]
struct AppState {
    root: PathBuf,
    mode: BrowserMode,
    view_mode: ViewMode,
    items: Vec<MediaItem>,
    selected: Option<usize>,
    back: Vec<(PathBuf, Option<PathBuf>)>,
    forward: Vec<(PathBuf, Option<PathBuf>)>,
}

struct SyncCell(std::cell::UnsafeCell<AppState>);
unsafe impl Sync for SyncCell {}

static STATE: std::sync::OnceLock<SyncCell> = std::sync::OnceLock::new();

struct WindowRefs {
    window: *mut NSWindow,
    scroll: *mut NSScrollView,
    grid: *mut GridView,
    preview: *mut NSView,
    divider: *mut NSBox,
    preview_image: *mut NSImageView,
}
unsafe impl Send for WindowRefs {}
unsafe impl Sync for WindowRefs {}

static WINDOW_REFS: std::sync::OnceLock<std::sync::Mutex<Option<WindowRefs>>> = std::sync::OnceLock::new();

fn window_refs() -> std::sync::MutexGuard<'static, Option<WindowRefs>> {
    WINDOW_REFS.get_or_init(|| std::sync::Mutex::new(None)).lock().unwrap()
}

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

#[allow(dead_code)]
fn layout_carousel(
    root: &NSView,
    preview: &NSView,
    image: &NSImageView,
    divider: &NSBox,
    scroll: &NSScrollView,
    frame: NSRect,
    carousel: bool,
) {
    if carousel {
        let strip_h = 184.0;
        scroll.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(frame.size.width, strip_h)));
        divider.setFrame(NSRect::new(NSPoint::new(0.0, strip_h), NSSize::new(frame.size.width, 1.0)));
        preview.setFrame(NSRect::new(NSPoint::new(0.0, strip_h + 1.0), NSSize::new(frame.size.width, frame.size.height - strip_h - 1.0)));
        image.setFrame(preview.frame());
    } else {
        preview.setHidden(true);
        divider.setHidden(true);
        scroll.setFrame(frame);
    }
    let _ = root;
}

fn persist_view_mode() {
    let defaults = objc2_foundation::NSUserDefaults::standardUserDefaults();
    let value = if unsafe { (*app_state()).view_mode } == ViewMode::Carousel {
        NSString::from_str("carousel")
    } else {
        NSString::from_str("grid")
    };
    unsafe { defaults.setObject_forKey(Some(&*value as &NSObject), &NSString::from_str("View.ViewMode")) };
}

fn apply_view_mode_layout() {
    let refs = window_refs();
    let Some(refs) = refs.as_ref() else { return };
    let carousel = unsafe { (*app_state()).view_mode } == ViewMode::Carousel;
    unsafe {
        let window = &*refs.window;
        let scroll = &*refs.scroll;
        let grid = &*refs.grid;
        let preview = &*refs.preview;
        let preview_image = &*refs.preview_image;
        let divider = &*refs.divider;
        let width = window.frame().size.width.max(320.0);
        let height = window.frame().size.height.max(240.0);
        if let Some(root) = window.contentView() {
            layout_carousel(&root, preview, preview_image, divider, scroll, NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height)), carousel);
        }
        scroll.setHasVerticalScroller(!carousel);
        scroll.setHasHorizontalScroller(carousel);
        let doc_size = content_size(width, (*app_state()).items.len(), carousel);
        grid.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), doc_size));
        grid.setNeedsDisplay(true);
    }
}

fn install_main_menu(_delegate: &AppDelegate, _carousel: bool) {
    let mtm = objc2::MainThreadMarker::new().unwrap();
    let main_menu = NSMenu::new(mtm);
    let app_menu = NSMenu::new(mtm);
    let quit_item = NSMenuItem::new(mtm);
    quit_item.setTitle(&NSString::from_str("Quit View"));
    unsafe { quit_item.setTarget(None) };
    let quit_sel: objc2::runtime::Sel = objc2::sel!(terminate:);
    unsafe { let _: () = objc2::msg_send![&*quit_item, setAction: quit_sel]; };
    quit_item.setKeyEquivalent(&NSString::from_str("q"));
    app_menu.addItem(&quit_item);
    let app_item = NSMenuItem::new(mtm);
    app_item.setTitle(&NSString::from_str("View"));
    app_item.setSubmenu(Some(&app_menu));
    main_menu.addItem(&app_item);

    let view_menu = NSMenu::new(mtm);
    let carousel_item = NSMenuItem::new(mtm);
    carousel_item.setTitle(&NSString::from_str("as Carousel"));
    carousel_item.setState(if _carousel { objc2_app_kit::NSControlStateValueOn } else { objc2_app_kit::NSControlStateValueOff });
    unsafe { carousel_item.setTarget(Some(_delegate as _)) };
    let carousel_action: objc2::runtime::Sel = objc2::sel!(setCarouselMode:);
    unsafe { let _: () = objc2::msg_send![&*carousel_item, setAction: carousel_action]; };
    view_menu.addItem(&carousel_item);
    let grid_item = NSMenuItem::new(mtm);
    grid_item.setTitle(&NSString::from_str("as Grid"));
    grid_item.setState(if _carousel { objc2_app_kit::NSControlStateValueOff } else { objc2_app_kit::NSControlStateValueOn });
    unsafe { grid_item.setTarget(Some(_delegate as _)) };
    let grid_action: objc2::runtime::Sel = objc2::sel!(setGridMode:);
    unsafe { let _: () = objc2::msg_send![&*grid_item, setAction: grid_action]; };
    view_menu.addItem(&grid_item);
    let view_item = NSMenuItem::new(mtm);
    view_item.setTitle(&NSString::from_str("View"));
    view_item.setSubmenu(Some(&view_menu));
    main_menu.addItem(&view_item);
    NSApplication::sharedApplication(mtm).setMainMenu(Some(&main_menu));
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
            let cols = if state.view_mode == ViewMode::Carousel { state.items.len().max(1) } else { columns_for(width).max(1) };
            let selected = state.selected;
            let mtm = objc2::MainThreadMarker::new().unwrap();
            for (i, item) in state.items.iter().enumerate() {
                let (x, y, cw, ch) = if state.view_mode == ViewMode::Carousel {
                    (
                        12.0 + i as f64 * (120.0 + 8.0),
                        8.0,
                        120.0,
                        148.0,
                    )
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
            let cols = if state.view_mode == ViewMode::Carousel { 1 } else { columns_for(self.bounds().size.width).max(1) };
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
            self.scroll_selection_into_view();
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
            let index = if state.view_mode == ViewMode::Carousel {
                let x = view_location.x - 12.0;
                let y = view_location.y - 8.0;
                if x < 0.0 || y < 0.0 || y >= 148.0 {
                    return;
                }
                (x / (120.0 + 8.0)).floor() as usize
            } else {
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
                row * cols + col
            };
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
            self.scroll_selection_into_view();
            self.setNeedsDisplay(true);
        }
    }
);

impl GridView {
    fn cell_rect(&self, index: usize) -> NSRect {
        let state = unsafe { &*app_state() };
        let width = self.bounds().size.width;
        if state.view_mode == ViewMode::Carousel {
            let x = 12.0 + index as f64 * (120.0 + 8.0);
            return NSRect::new(NSPoint::new(x, 8.0), NSSize::new(120.0, 148.0));
        }
        let cols = columns_for(width).max(1);
        let col = index % cols;
        let row = index / cols;
        NSRect::new(
            NSPoint::new(16.0 + col as f64 * (CELL_W + SPACING), 16.0 + row as f64 * (CELL_H + SPACING)),
            NSSize::new(CELL_W, CELL_H),
        )
    }

    fn scroll_selection_into_view(&self) {
        let index = unsafe { (*app_state()).selected };
        let Some(index) = index else { return };
        let Some(superview) = (unsafe { self.superview() }) else { return };
        let Ok(scroll_view) = superview.downcast::<NSScrollView>() else { return };
        let cell = self.cell_rect(index);
        let visible = scroll_view.visibleRect();
        let origin = scroll_view.bounds().origin;
        let epsilon = 0.5;
        let view_mode_carousel = unsafe { (*app_state()).view_mode } == ViewMode::Carousel;
        let target: Option<NSPoint> = if view_mode_carousel {
            if cell.origin.x < visible.origin.x + epsilon {
                Some(NSPoint::new(cell.origin.x, origin.y))
            } else if cell.origin.x + cell.size.width > visible.origin.x + visible.size.width - epsilon {
                Some(NSPoint::new(cell.origin.x + cell.size.width - visible.size.width, origin.y))
            } else { None }
        } else if cell.origin.y < visible.origin.y + epsilon {
            Some(NSPoint::new(origin.x, cell.origin.y))
        } else if cell.origin.y + cell.size.height > visible.origin.y + visible.size.height - epsilon {
            Some(NSPoint::new(origin.x, cell.origin.y + cell.size.height - visible.size.height))
        } else { None };
        if let Some(point) = target {
            let clip = scroll_view.contentView();
            clip.scrollToPoint(point);
            scroll_view.reflectScrolledClipView(&clip);
        }
    }
}

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

        #[unsafe(method(setCarouselMode:))]
        fn set_carousel_mode(&self, _sender: &NSObject) {
            unsafe { &mut *app_state() }.view_mode = ViewMode::Carousel;
            persist_view_mode();
            apply_view_mode_layout();
        }

        #[unsafe(method(setGridMode:))]
        fn set_grid_mode(&self, _sender: &NSObject) {
            unsafe { &mut *app_state() }.view_mode = ViewMode::Grid;
            persist_view_mode();
            apply_view_mode_layout();
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
            let view_mode_carousel = unsafe { (*app_state()).view_mode } == ViewMode::Carousel;
            let root_view = NSView::new(objc2::MainThreadMarker::new().unwrap());
            let preview_nsview = NSView::new(objc2::MainThreadMarker::new().unwrap());
            let preview_image = NSImageView::new(objc2::MainThreadMarker::new().unwrap());
            preview_image.setImageScaling(objc2_app_kit::NSImageScaling::ScaleProportionallyUpOrDown);
            preview_image.setImageAlignment(objc2_app_kit::NSImageAlignment::AlignCenter);
            preview_nsview.addSubview(&*preview_image);
            let divider = NSBox::new(objc2::MainThreadMarker::new().unwrap());
            let scroll = NSScrollView::new(objc2::MainThreadMarker::new().unwrap());
            scroll.setHasVerticalScroller(!view_mode_carousel);
            scroll.setHasHorizontalScroller(view_mode_carousel);
            let grid: Retained<GridView> = unsafe {
                objc2::msg_send![GridView::alloc(objc2::MainThreadMarker::new().unwrap()), initWithFrame: frame]
            };
            root_view.addSubview(&*preview_nsview);
            root_view.addSubview(&*divider);
            root_view.addSubview(&*scroll);
            // Fallback absolute layout (Swift uses constraints); frames are
            // positioned for the current mode and updated on mode switch.
            layout_carousel(&root_view, &preview_nsview, &preview_image, &divider, &scroll, frame, view_mode_carousel);
            let doc_size = content_size(frame_size.width, item_count, view_mode_carousel);
            grid.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), doc_size));
            grid.setNeedsDisplay(true);
            {
                let state = unsafe { &*app_state() };
                if let Some(item) = state.selected.and_then(|i| state.items.get(i)) {
                    if let Some(img) = image_for(item) {
                        preview_image.setImage(Some(&*img));
                    } else if let Some(img) = system_image(match item.kind {
                        MediaKind::Directory => "folder",
                        MediaKind::Image => "photo",
                        MediaKind::Movie => "film",
                    }) {
                        preview_image.setImage(Some(&*img));
                    }
                }
            }
            scroll.setDocumentView(Some(&*grid));
            window.setContentView(Some(&*root_view));
            *window_refs() = Some(WindowRefs {
                window: Retained::into_raw(window.clone()) as *mut NSWindow,
                scroll: Retained::into_raw(scroll.clone()) as *mut NSScrollView,
                grid: Retained::into_raw(grid.clone()) as *mut GridView,
                preview: Retained::into_raw(preview_nsview.clone()) as *mut NSView,
                divider: Retained::into_raw(divider.clone()) as *mut NSBox,
                preview_image: Retained::into_raw(preview_image.clone()) as *mut NSImageView,
            });
            install_main_menu(self, view_mode_carousel);
            window.setTitle(&NSString::from_str(&title));
            window.setFrameAutosaveName(&NSString::from_str("ViewWindow"));
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
        let defaults = objc2_foundation::NSUserDefaults::standardUserDefaults();
        let saved = defaults.stringForKey(&NSString::from_str("View.ViewMode"))
            .map(|s| s.to_string());
        state.view_mode = match saved.as_deref() {
            Some("grid") => ViewMode::Grid,
            _ => ViewMode::Carousel,
        };
        state.items = load_listing(&root, mode);
        state.selected = if state.items.is_empty() { None } else { Some(0) };
    }

    let mtm = objc2::MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    let delegate: Retained<AppDelegate> = unsafe { objc2::msg_send![AppDelegate::class(), new] };
    app.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*delegate)));
    app.run();
}
