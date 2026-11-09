//! Rust port GUI binary (see Rust/PARITY.md).
//!
//! Native AppKit host built on `objc2-app-kit` (the same frameworks as
//! the Swift app): window + scroll view + custom grid view, wired to
//! the `view_core` discovery/catalog state.

use std::path::PathBuf;
use std::os::unix::ffi::{OsStrExt, OsStringExt};

use objc2::{define_class, ClassType, DefinedClass, MainThreadOnly};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::NSObject;
use objc2_app_kit::{
    NSApplication, NSApplicationDelegate, NSBackingStoreType, NSBox, NSColor, NSDraggingInfo, NSDraggingSource, NSFont, NSImage,
    NSImageView, NSMenu, NSMenuItem, NSScrollView, NSView, NSWindow, NSWindowDelegate, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{
    NSArray, NSData, NSDictionary, NSNotification, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL,
};
use objc2_quick_look_thumbnailing::{
    QLThumbnailGenerationRequest, QLThumbnailGenerationRequestRepresentationTypes, QLThumbnailGenerator,
    QLThumbnailRepresentation,
};
use objc2_quick_look_ui::{QLPreviewItem, QLPreviewPanel, QLPreviewPanelDataSource};

use view_core::{
    list_browser, scan_recursive, BrowserMode, MediaKind, MediaItem, TrashHistory, TrashService,
    TrashTransaction,
};

const CELL_W: f64 = 160.0;
const CELL_H: f64 = 180.0;
const SPACING: f64 = 10.0;
const CAROUSEL_CELL_W: f64 = 120.0;
const CAROUSEL_CELL_H: f64 = 156.0;
const CAROUSEL_SPACING: f64 = 8.0;

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
    thumbnail_size_step: i8,
    items: Vec<MediaItem>,
    selected: Option<usize>,
    selection_anchor: Option<usize>,
    selection_range: (usize, usize),
    back: Vec<(PathBuf, Option<PathBuf>)>,
    forward: Vec<(PathBuf, Option<PathBuf>)>,
    _trash_service: TrashService,
    trash_history: TrashHistory,
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

fn aspect_fit(img_size: NSSize, target: NSRect) -> NSRect {
    if img_size.width <= 0.0 || img_size.height <= 0.0 {
        return target;
    }
    let scale_w = target.size.width / img_size.width;
    let scale_h = target.size.height / img_size.height;
    let scale = scale_w.min(scale_h);
    let fitted_w = (img_size.width * scale).round();
    let fitted_h = (img_size.height * scale).round();
    let x = (target.origin.x + (target.size.width - fitted_w) / 2.0).round();
    let y = (target.origin.y + (target.size.height - fitted_h) / 2.0).round();
    NSRect::new(NSPoint::new(x, y), NSSize::new(fitted_w, fitted_h))
}

fn byte_count_string(bytes: u64) -> Retained<NSString> {
    unsafe {
        objc2::msg_send![
            objc2::class!(NSByteCountFormatter),
            stringFromByteCount: bytes as i64,
            countStyle: 0isize
        ]
    }
}

fn thumbnail_scale() -> f64 {
    1.2_f64.powi(unsafe { (*app_state()).thumbnail_size_step } as i32)
}

fn columns_for(width: f64) -> usize {
    let scale = thumbnail_scale();
    let available = (width - 32.0).max(0.0);
    ((available + SPACING) / (CELL_W * scale + SPACING)).floor().max(1.0) as usize
}

fn grid_cell_spacing(width: f64) -> f64 {
    let cols = columns_for(width);
    if cols <= 1 {
        return SPACING;
    }
    let available = (width - 32.0).max(0.0);
    let remaining = available - (cols as f64 * CELL_W * thumbnail_scale());
    (remaining / (cols as f64 - 1.0)).max(SPACING)
}

fn content_size(width: f64, height: f64, count: usize, carousel: bool) -> NSSize {
    let scale = thumbnail_scale();
    if carousel {
        NSSize::new(((count as f64) * (CAROUSEL_CELL_W * scale + CAROUSEL_SPACING) + 24.0).max(width), CAROUSEL_CELL_H * scale + 28.0)
    } else {
        let cols = columns_for(width).max(1);
        let rows = count.div_ceil(cols);
        let content_h = 16.0 + rows as f64 * (CELL_H * scale + SPACING) + 16.0;
        NSSize::new(width, content_h.max(height))
    }
}

fn layout_carousel(
    _root: &NSView,
    preview: &NSView,
    image: &NSImageView,
    divider: &NSBox,
    scroll: &NSScrollView,
    frame: NSRect,
    carousel: bool,
) {
    if carousel {
        let strip_h = (CAROUSEL_CELL_H * thumbnail_scale() + 28.0).min(frame.size.height);
        let preview_h = (frame.size.height - strip_h).max(0.0);
        scroll.setHidden(false);
        divider.setHidden(false);
        preview.setHidden(false);
        scroll.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(frame.size.width, strip_h)));
        divider.setFrame(NSRect::new(NSPoint::new(0.0, strip_h), NSSize::new(frame.size.width, 1.0)));
        preview.setFrame(NSRect::new(NSPoint::new(0.0, strip_h + 1.0), NSSize::new(frame.size.width, (preview_h - 1.0).max(0.0))));
        image.setFrame(NSRect::new(NSPoint::ZERO, preview.bounds().size));
    } else {
        preview.setHidden(true);
        divider.setHidden(true);
        scroll.setHidden(false);
        scroll.setFrame(frame);
    }
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

fn update_carousel_preview() {
    let (preview_image_ptr, preview_ptr) = {
        let refs = window_refs();
        let Some(refs) = refs.as_ref() else { return };
        (refs.preview_image, refs.preview)
    };
    let preview_image = unsafe { &*preview_image_ptr };
    let preview = unsafe { &*preview_ptr };
    let state = unsafe { &*app_state() };
    if state.view_mode != ViewMode::Carousel {
        return;
    }
    let Some(index) = state.selected else {
        preview_image.setImage(None);
        return;
    };
    let Some(item) = state.items.get(index) else {
        preview_image.setImage(None);
        return;
    };
    let preview_bounds = preview.bounds();
    if item.kind == MediaKind::Directory {
        preview_image.setImageScaling(objc2_app_kit::NSImageScaling::ScaleProportionallyDown);
        if let Some(icon) = folder_like_icon(item) {
            unsafe { icon.setSize(NSSize::new(256.0, 256.0)); }
            preview_image.setImage(Some(&*icon));
            let x = ((preview_bounds.size.width - 256.0) / 2.0).round();
            let y = ((preview_bounds.size.height - 256.0) / 2.0).round();
            preview_image.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(256.0, 256.0)));
        }
    } else {
        preview_image.setImageScaling(objc2_app_kit::NSImageScaling::ScaleProportionallyUpOrDown);
        let image = if item.kind == MediaKind::Movie { cached_thumbnail(&item.path) } else { image_for(item) };
        if let Some(img) = image {
            let fitted = aspect_fit(img.size(), preview_bounds);
            preview_image.setFrame(fitted);
            preview_image.setImage(Some(&*img));
        } else {
            preview_image.setImage(None);
            if item.kind == MediaKind::Movie {
                request_quicklook_thumbnail(item.path.clone());
            }
        }
    }
    preview.setNeedsDisplay(true);
}

fn apply_view_mode_layout() {
    let (window_ptr, scroll_ptr, grid_ptr, preview_ptr, preview_image_ptr, divider_ptr) = {
        let refs = window_refs();
        let Some(refs) = refs.as_ref() else { return };
        (refs.window, refs.scroll, refs.grid, refs.preview, refs.preview_image, refs.divider)
    };
    let carousel = unsafe { (*app_state()).view_mode } == ViewMode::Carousel;
    unsafe {
        let window = &*window_ptr;
        let scroll = &*scroll_ptr;
        let grid = &*grid_ptr;
        let preview = &*preview_ptr;
        let preview_image = &*preview_image_ptr;
        let divider = &*divider_ptr;
        let Some(root) = window.contentView() else { return };
        let bounds = root.bounds();
        layout_carousel(&root, preview, preview_image, divider, scroll, bounds, carousel);
        scroll.setHasVerticalScroller(!carousel);
        scroll.setHasHorizontalScroller(carousel);
        let viewport = scroll.contentView().bounds().size;
        let doc_size = content_size(viewport.width, viewport.height, (*app_state()).items.len(), carousel);
        grid.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), doc_size));
        grid.setNeedsDisplay(true);
    }
    update_carousel_preview();
}

fn install_main_menu(_delegate: &AppDelegate, _carousel: bool, grid: &GridView) {
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

    let edit_menu = NSMenu::new(mtm);
    edit_menu.addItem(&menu_item(grid, "Undo", objc2::sel!(undoTrash:), Some("z"), objc2_app_kit::NSEventModifierFlags::Command));
    edit_menu.addItem(&menu_item(grid, "Redo", objc2::sel!(redoTrash:), Some("Z"), objc2_app_kit::NSEventModifierFlags::Command | objc2_app_kit::NSEventModifierFlags::Shift));
    edit_menu.addItem(&menu_item(grid, "Move to Trash", objc2::sel!(moveSelectedToTrash:), Some("\u{8}"), objc2_app_kit::NSEventModifierFlags::Command));
    let edit_item = NSMenuItem::new(mtm);
    edit_item.setTitle(&NSString::from_str("Edit"));
    edit_item.setSubmenu(Some(&edit_menu));
    main_menu.addItem(&edit_item);
    NSApplication::sharedApplication(mtm).setMainMenu(Some(&main_menu));
}

fn url_from_path(path: &std::path::Path) -> Retained<NSURL> {
    let bytes = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("filesystem path contains no NUL");
    let pointer = std::ptr::NonNull::new(bytes.as_ptr() as *mut std::ffi::c_char).unwrap();
    unsafe { NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(pointer, path.is_dir(), None) }
}

fn path_from_url(url: &NSURL) -> PathBuf {
    let bytes = unsafe { std::ffi::CStr::from_ptr(url.fileSystemRepresentation().as_ptr()) }.to_bytes();
    PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec()))
}

fn trash_selected(grid: &GridView) {
    let (items, indexes) = {
        let state = unsafe { &mut *app_state() };
        let Some(index) = state.selected else { return };
        let Some(item) = state.items.get(index) else { return };
        (vec![item.clone()], vec![index])
    };
    let workspace = NSWorkspace::sharedWorkspace();
    let urls: Vec<Retained<NSURL>> = items.iter().map(|i| url_from_path(&i.path)).collect();
    let ns_urls = NSArray::from_retained_slice(&urls);
    let handler = unsafe {
        block2::RcBlock::new(move |mapping: core::ptr::NonNull<NSDictionary<NSURL, NSURL>>, error: *mut objc2_foundation::NSError| {
            let mapping = unsafe { mapping.as_ref() };
            let trashed_map: Vec<(PathBuf, PathBuf)> = mapping.allKeys()
                .iter()
                .filter_map(|key| {
                    let key_url = unsafe { key.downcast_ref::<NSURL>()? };
                    let value = unsafe { mapping.objectForKey(key_url) };
                    let value_url = value.as_ref().map(|u| unsafe { u.downcast_ref::<NSURL>() }).flatten();
                    let trashed = value_url.map(|u| path_from_url(u));
                    let original = path_from_url(key_url);
                    trashed.map(|trashed| (original, trashed))
                })
                .collect();
            let had_error = !error.is_null();
            dispatch2::DispatchQueue::main().exec_async(move || {
                if had_error {
                    return;
                }
                let state = unsafe { &mut *app_state() };
                let mut files = Vec::new();
                for (original, trashed) in trashed_map.iter() {
                    if let Some(pos) = state.items.iter().position(|i| &i.path == original) {
                        files.push(view_core::TrashedFile {
                            item: state.items[pos].clone(),
                            trashed_path: trashed.clone(),
                            index: pos,
                        });
                    }
                }
                for file in files.iter().rev() {
                    if let Some(pos) = state.items.iter().position(|i| i.path == file.item.path) {
                        state.items.remove(pos);
                    }
                }
                state.selected = state.items.first().map(|_| 0);
                state.trash_history.record_trash(view_core::TrashTransaction {
                    files,
                    selection_before_op: vec![],
                });
                if let Some(refs) = window_refs().as_ref() {
                    let grid = unsafe { &*refs.grid };
                    grid.setNeedsDisplay(true);
                }
            });
        })
    };
    workspace.recycleURLs_completionHandler(&ns_urls, Some(&handler));
}

fn move_trash_item(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    let from = std::ffi::CString::new(from.as_os_str().as_bytes())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let to = std::ffi::CString::new(to.as_os_str().as_bytes())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

fn undo_trash(grid: &GridView) {
    let transaction = unsafe { (&mut *app_state()).trash_history.pop_undo() };
    let Some(transaction) = transaction else { return };
    let mut succeeded = Vec::new();
    let mut residual = Vec::new();
    for file in transaction.files {
        match move_trash_item(&file.trashed_path, &file.item.path) {
            Ok(()) => succeeded.push(file),
            Err(error) => {
                eprintln!("view: cannot restore {}: {error}", file.item.path.display());
                residual.push(file);
            }
        }
    }
    {
        let state = unsafe { &mut *app_state() };
        if !succeeded.is_empty() {
            let selection_before_op = transaction.selection_before_op.clone();
            for file in succeeded.iter().rev() {
                let at = file.index.min(state.items.len());
                state.items.insert(at, file.item.clone());
            }
            state.trash_history.push_redo(TrashTransaction { files: succeeded, selection_before_op });
        }
        if !residual.is_empty() {
            state.trash_history.push_undo(TrashTransaction { files: residual, selection_before_op: transaction.selection_before_op });
        }
    }
    grid.setNeedsDisplay(true);
}

fn redo_trash(grid: &GridView) {
    let transaction = unsafe { (&mut *app_state()).trash_history.pop_redo() };
    let Some(transaction) = transaction else { return };
    let mut succeeded = Vec::new();
    let mut residual = Vec::new();
    for file in transaction.files {
        match move_trash_item(&file.item.path, &file.trashed_path) {
            Ok(()) => succeeded.push(file),
            Err(error) => {
                eprintln!("view: cannot redo trash for {}: {error}", file.item.path.display());
                residual.push(file);
            }
        }
    }
    {
        let state = unsafe { &mut *app_state() };
        let selection_before_op = transaction.selection_before_op.clone();
        for file in succeeded.iter().rev() {
            if let Some(pos) = state.items.iter().position(|i| i.path == file.item.path) {
                state.items.remove(pos);
            }
        }
        state.selected = state.items.first().map(|_| 0);
        if !succeeded.is_empty() {
            state.trash_history.push_undo(TrashTransaction { files: succeeded, selection_before_op });
        }
        if !residual.is_empty() {
            state.trash_history.push_redo(TrashTransaction { files: residual, selection_before_op: transaction.selection_before_op });
        }
    }
    grid.setNeedsDisplay(true);
}

fn quicklook_selected() {
    let mtm = objc2::MainThreadMarker::new().unwrap();
    let Some(panel) = (unsafe { QLPreviewPanel::sharedPreviewPanel(mtm) }) else { return };
    if panel.isVisible() {
        panel.orderOut(None);
        return;
    }
    panel.makeKeyAndOrderFront(None);
}

fn make_context_action_item(target: &GridView, title: &str, action: objc2::runtime::Sel, key: Option<&str>, modifier_mask: objc2_app_kit::NSEventModifierFlags) -> Retained<NSMenuItem> {
    let mtm = objc2::MainThreadMarker::new().unwrap();
    let item = NSMenuItem::new(mtm);
    item.setTitle(&NSString::from_str(title));
    unsafe { item.setTarget(Some(target as _)) };
    unsafe { let _: () = objc2::msg_send![&*item, setAction: action]; };
    if let Some(key) = key {
        item.setKeyEquivalent(&NSString::from_str(key));
        unsafe { item.setKeyEquivalentModifierMask(modifier_mask) };
    }
    item
}

fn build_context_menu(target: &GridView) -> Retained<NSMenu> {
    let mtm = objc2::MainThreadMarker::new().unwrap();
    let menu = NSMenu::new(mtm);
    menu.addItem(&make_context_action_item(target, "Open", objc2::sel!(openSelected:), None, objc2_app_kit::NSEventModifierFlags::empty()));
    menu.addItem(&make_context_action_item(target, "Show in Finder", objc2::sel!(showSelectedInFinder:), None, objc2_app_kit::NSEventModifierFlags::empty()));
    menu.addItem(&make_context_action_item(target, "Copy Path", objc2::sel!(copySelectedPath:), None, objc2_app_kit::NSEventModifierFlags::empty()));
    menu.addItem(&make_context_action_item(target, "Move to Trash", objc2::sel!(moveSelectedToTrash:), Some("\u{8}"), objc2_app_kit::NSEventModifierFlags::Command));
    menu.addItem(&make_context_action_item(target, "Undo", objc2::sel!(undoTrash:), Some("z"), objc2_app_kit::NSEventModifierFlags::Command));
    menu.addItem(&make_context_action_item(target, "Redo", objc2::sel!(redoTrash:), Some("Z"), objc2_app_kit::NSEventModifierFlags::Command | objc2_app_kit::NSEventModifierFlags::Shift));
    menu
}

fn menu_item(target: &GridView, title: &str, action: objc2::runtime::Sel, key: Option<&str>, modifier_mask: objc2_app_kit::NSEventModifierFlags) -> Retained<NSMenuItem> {
    make_context_action_item(target, title, action, key, modifier_mask)
}

fn place_context_menu(target: &GridView, event: &objc2_app_kit::NSEvent) {
    let menu = build_context_menu(target);
    NSMenu::popUpContextMenu_withEvent_forView(&menu, event, target);
}

fn enter_directory(path: PathBuf) {
    let state = unsafe { &mut *app_state() };
    let current = state.root.clone();
    let previous = state.selected.and_then(|i| state.items.get(i)).map(|item| item.path.clone());
    state.back.push((current, previous));
    state.forward.clear();
    state.root = path;
    state.items = load_listing(&state.root, state.mode);
    state.selected = state.items.first().map(|_| 0);
    state.selection_anchor = state.selected;
    state.selection_range = (0, 0);
    if let Some(refs) = window_refs().as_ref() {
        unsafe { (&*refs.window).setTitle(&NSString::from_str(&state.root.display().to_string())); }
    }
    apply_view_mode_layout();
}

fn open_selected() {
    let Some(item) = unsafe { (*app_state()).selected }.and_then(|i| unsafe { (&*app_state()).items.get(i).cloned() }) else { return };
    if item.kind == MediaKind::Directory {
        enter_directory(item.path);
    } else {
        let ws = NSWorkspace::sharedWorkspace();
        ws.openFile(&NSString::from_str(&item.path.display().to_string()));
    }
}

fn show_selected_in_finder() {
    let Some(item) = unsafe { (*app_state()).selected }.and_then(|i| unsafe { (&*app_state()).items.get(i).cloned() }) else { return };
    let _ = std::process::Command::new("open").arg("-R").arg(&item.path).spawn();
}

fn copy_selected_path() {
    let Some(item) = unsafe { (*app_state()).selected }.and_then(|i| unsafe { (&*app_state()).items.get(i).cloned() }) else { return };
    let pasteboard = objc2_app_kit::NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    unsafe { pasteboard.setString_forType(&NSString::from_str(&item.path.display().to_string()), &objc2_app_kit::NSPasteboardTypeString) };
}

static DRAG_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn dragging_items(view: &GridView) -> Vec<Retained<objc2_app_kit::NSDraggingItem>> {
    let state = unsafe { &*app_state() };
    let Some(selected) = state.selected else { return Vec::new() };
    let (start, end) = if state.selection_anchor.is_some() {
        state.selection_range
    } else {
        (selected, selected)
    };
    (start..=end).filter_map(|index| {
        let item = state.items.get(index)?;
        let url = url_from_path(&item.path);
        let dragging_item: Retained<objc2_app_kit::NSDraggingItem> = unsafe {
            let alloc: *mut objc2_app_kit::NSDraggingItem = objc2::msg_send![objc2_app_kit::NSDraggingItem::class(), alloc];
            let ptr: *mut objc2_app_kit::NSDraggingItem = objc2::msg_send![alloc, initWithPasteboardWriter: &*url];
            Retained::from_raw(ptr).expect("create dragging item")
        };
        let image = folder_like_icon(item).expect("file icon");
        unsafe {
            let _: () = objc2::msg_send![&*dragging_item, setDraggingFrame: view.cell_rect(index), contents: &*image];
        }
        Some(dragging_item)
    }).collect()
}

fn begin_drag_session(view: &GridView, event: &objc2_app_kit::NSEvent) {
    if DRAG_ACTIVE.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let items = dragging_items(view);
    if items.is_empty() {
        DRAG_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
        return;
    }
    let items = NSArray::from_retained_slice(&items);
    let source = objc2::runtime::ProtocolObject::from_ref(view);
    view.beginDraggingSessionWithItems_event_source(&items, event, source);
}

fn dropped_path(pasteboard: &objc2_app_kit::NSPasteboard) -> Option<PathBuf> {
    let text = unsafe { pasteboard.stringForType(&objc2_app_kit::NSPasteboardTypeFileURL) }?;
    let url = NSURL::URLWithString(&text)?;
    if !url.isFileURL() { return None; }
    let path = path_from_url(&url);
    std::fs::canonicalize(path).ok()
}

fn accept_dropped_path(view: &GridView, pasteboard: &objc2_app_kit::NSPasteboard) -> bool {
    let Some(path) = dropped_path(pasteboard) else { return false };
    if path.is_dir() {
        enter_directory(path);
        return true;
    }
    if !path.is_file() { return false; }
    let Some(parent) = path.parent() else { return false };
    let mode = unsafe { (*app_state()).mode };
    if !load_listing(&parent.to_path_buf(), mode).iter().any(|item| item.path == path) { return false; }
    if unsafe { (*app_state()).root != parent } {
        enter_directory(parent.to_path_buf());
    }
    let state = unsafe { &mut *app_state() };
    let Some(index) = state.items.iter().position(|item| item.path == path) else { return false };
    state.selected = Some(index);
    state.selection_anchor = Some(index);
    state.selection_range = (index, index);
    view.scroll_selection_into_view();
    update_carousel_preview();
    view.setNeedsDisplay(true);
    true
}

fn load_listing(root: &PathBuf, mode: BrowserMode) -> Vec<MediaItem> {
    match mode {
        BrowserMode::Browser => list_browser(root).unwrap_or_default(),
        BrowserMode::Recursive => scan_recursive(root).unwrap_or_default(),
    }
}

fn folder_like_icon(item: &MediaItem) -> Option<Retained<NSImage>> {
    let workspace = NSWorkspace::sharedWorkspace();
    let icon = workspace.iconForFile(&NSString::from_str(&item.path.display().to_string()));
    Some(icon)
}

fn system_image(name: &str) -> Option<Retained<NSImage>> {
    let ns_name = NSString::from_str(name);
    unsafe { NSImage::imageWithSystemSymbolName_accessibilityDescription(&ns_name, None) }
}

struct SyncImage(Retained<NSImage>);
unsafe impl Send for SyncImage {}
unsafe impl Sync for SyncImage {}

static THUMB_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<PathBuf, SyncImage>>,
> = std::sync::OnceLock::new();
static THUMB_PENDING: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<PathBuf>>> =
    std::sync::OnceLock::new();

fn thumb_cache() -> std::sync::MutexGuard<'static, std::collections::HashMap<PathBuf, SyncImage>> {
    THUMB_CACHE
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn thumb_pending() -> std::sync::MutexGuard<'static, std::collections::HashSet<PathBuf>> {
    THUMB_PENDING
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn cached_thumbnail(path: &PathBuf) -> Option<Retained<NSImage>> {
    thumb_cache().get(path).map(|s| s.0.clone())
}

fn request_quicklook_thumbnail(path: PathBuf) {
    if !thumb_pending().insert(path.clone()) {
        return;
    }
    let generator = unsafe { QLThumbnailGenerator::sharedGenerator() };
    let url = url_from_path(&path);
    let request: Retained<QLThumbnailGenerationRequest> = unsafe {
        let alloc = objc2::MainThreadMarker::new().unwrap().alloc::<QLThumbnailGenerationRequest>();
        QLThumbnailGenerationRequest::initWithFileAtURL_size_scale_representationTypes(
            alloc,
            &url,
            NSSize::new(480.0, 480.0),
            2.0,
            QLThumbnailGenerationRequestRepresentationTypes::Thumbnail,
        )
    };
    let path_for_reply = path.clone();
    let handler = block2::RcBlock::new(move |representation: *mut QLThumbnailRepresentation, error: *mut objc2_foundation::NSError| {
        if representation.is_null() {
            thumb_pending().remove(&path_for_reply);
            if !error.is_null() {
                eprintln!("view: thumbnail {}: {:?}", path_for_reply.display(), unsafe { &*error });
            }
            return;
        }
        let image = SyncImage(unsafe { (*(representation as *const QLThumbnailRepresentation)).NSImage() });
        let path = path_for_reply.clone();
        dispatch2::DispatchQueue::main().exec_async(move || thumbnail_completed(path, image));
    });
    unsafe { generator.generateBestRepresentationForRequest_completionHandler(&request, &handler) };
}

fn thumbnail_completed(path: PathBuf, image: SyncImage) {
    thumb_pending().remove(&path);
    thumb_cache().insert(path, image);
    if let Some(refs) = window_refs().as_ref() {
        unsafe { &*refs.grid }.setNeedsDisplay(true);
    }
    update_carousel_preview();
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
    struct PreviewRegionView;

    impl PreviewRegionView {
        #[unsafe(method_id(initWithFrame:))]
        fn init_with_frame(this: Allocated<Self>, frame: NSRect) -> Retained<Self> {
            let this = this.set_ivars(());
            unsafe { objc2::msg_send![super(this), initWithFrame: frame] }
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _rect: NSRect) {
            unsafe {
                NSColor::windowBackgroundColor().setFill();
                objc2_app_kit::NSRectFill(self.bounds());
            }
        }
    }
);

#[derive(Default)]
struct GridInteraction {
    deferred_click: std::cell::Cell<Option<usize>>,
    drag_candidate: std::cell::Cell<bool>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = GridInteraction]
    struct GridView;

    impl GridView {
        #[unsafe(method_id(initWithFrame:))]
        fn init_with_frame(this: Allocated<Self>, frame: NSRect) -> Retained<Self> {
            let this = this.set_ivars(GridInteraction::default());
            unsafe {
                let this: Retained<Self> = objc2::msg_send![super(this), initWithFrame: frame];
                this.setWantsLayer(true);
                this.registerForDraggedTypes(&NSArray::from_slice(&[&*objc2_app_kit::NSPasteboardTypeFileURL]));
                this
            }
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, rect: NSRect) {
            let state = unsafe { &mut *app_state() };
            let scale = thumbnail_scale();
            let selected = state.selected;
            let range_start = state.selection_range.0;
            let range_end = state.selection_range.1;

            unsafe {
                NSColor::windowBackgroundColor().setFill();
                objc2_app_kit::NSRectFill(self.bounds());
            }

            let mtm = objc2::MainThreadMarker::new().unwrap();
            let pstyle: Retained<objc2_app_kit::NSMutableParagraphStyle> = unsafe {
                let p: Retained<objc2_app_kit::NSMutableParagraphStyle> = objc2::msg_send![objc2_app_kit::NSMutableParagraphStyle::class(), new];
                p.setAlignment(objc2_app_kit::NSTextAlignment::Center);
                p.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingMiddle);
                p
            };
            let pstyle_detail: Retained<objc2_app_kit::NSMutableParagraphStyle> = unsafe {
                let p: Retained<objc2_app_kit::NSMutableParagraphStyle> = objc2::msg_send![objc2_app_kit::NSMutableParagraphStyle::class(), new];
                p.setAlignment(objc2_app_kit::NSTextAlignment::Center);
                p.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingTail);
                p
            };

            for (i, item) in state.items.iter().enumerate() {
                let cell = self.cell_rect(i);
                if !objc2_foundation::NSIntersectsRect(cell, rect) {
                    continue;
                }
                let is_selected = selected == Some(i) || (i >= range_start && i <= range_end && state.selection_anchor.is_some());
                if is_selected {
                    let path = unsafe { objc2_app_kit::NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(cell, 10.0, 10.0) };
                    let fill = unsafe { NSColor::selectedContentBackgroundColor().colorWithAlphaComponent(0.6) };
                    fill.setFill();
                    unsafe { path.fill() };
                    let stroke = unsafe { NSColor::selectedContentBackgroundColor() };
                    stroke.setStroke();
                    path.setLineWidth(2.0);
                    unsafe { path.stroke() };
                }

                let (x, y) = (cell.origin.x, cell.origin.y);
                let (thumb_rect, name_rect, detail_rect) = if state.view_mode == ViewMode::Carousel {
                    (
                        NSRect::new(NSPoint::new(x + 6.0 * scale, y + 6.0 * scale), NSSize::new(108.0 * scale, 110.0 * scale)),
                        NSRect::new(NSPoint::new(x + 6.0 * scale, y + 120.0 * scale), NSSize::new(108.0 * scale, 16.0 * scale)),
                        NSRect::new(NSPoint::new(x + 6.0 * scale, y + 137.0 * scale), NSSize::new(108.0 * scale, 14.0 * scale)),
                    )
                } else {
                    (
                        NSRect::new(NSPoint::new(x + 6.0 * scale, y + 6.0 * scale), NSSize::new(148.0 * scale, 134.0 * scale)),
                        NSRect::new(NSPoint::new(x + 6.0 * scale, y + 144.0 * scale), NSSize::new(148.0 * scale, 16.0 * scale)),
                        NSRect::new(NSPoint::new(x + 6.0 * scale, y + 161.0 * scale), NSSize::new(148.0 * scale, 14.0 * scale)),
                    )
                };

                let thumb_img = if item.kind == MediaKind::Directory {
                    folder_like_icon(item)
                } else if item.kind == MediaKind::Movie {
                    let cached = cached_thumbnail(&item.path);
                    if cached.is_none() {
                        request_quicklook_thumbnail(item.path.clone());
                    }
                    cached.or_else(|| folder_like_icon(item)).or_else(|| system_image("film"))
                } else {
                    image_for(item).or_else(|| system_image("exclamationmark.triangle"))
                };

                if let Some(img) = thumb_img {
                    let fitted = aspect_fit(img.size(), thumb_rect);
                    unsafe {
                        img.drawInRect_fromRect_operation_fraction_respectFlipped_hints(
                            fitted,
                            NSRect::ZERO,
                            objc2_app_kit::NSCompositingOperation::SourceOver,
                            1.0,
                            true,
                            None,
                        );
                    }
                }

                let name = item.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let string = NSString::from_str(&name);
                let font: Retained<NSFont> = unsafe {
                    if item.kind == MediaKind::Directory {
                        NSFont::boldSystemFontOfSize(12.0 * scale)
                    } else {
                        NSFont::systemFontOfSize(12.0 * scale)
                    }
                };
                let color = unsafe { NSColor::labelColor() };
                let name_dict: Retained<NSDictionary<objc2_foundation::NSAttributedStringKey>> = unsafe {
                    let keys = NSArray::from_slice(&[
                        objc2_app_kit::NSFontAttributeName,
                        objc2_app_kit::NSForegroundColorAttributeName,
                        objc2_app_kit::NSParagraphStyleAttributeName,
                    ]);
                    let objs: [&NSObject; 3] = [
                        &*font as &NSObject,
                        &*color as &NSObject,
                        &*pstyle as &NSObject,
                    ];
                    let values = NSArray::from_slice(&objs);
                    let dict_ptr: *mut NSDictionary<objc2_foundation::NSAttributedStringKey> =
                        objc2::msg_send![objc2::class!(NSDictionary), dictionaryWithObjects: &*values, forKeys: &*keys];
                    Retained::retain(dict_ptr).unwrap()
                };
                use objc2_app_kit::NSStringDrawing;
                unsafe { string.drawInRect_withAttributes(name_rect, Some(&*name_dict)) };

                if item.kind != MediaKind::Directory {
                    let file_size = std::fs::metadata(&item.path).map(|m| m.len()).unwrap_or(0);
                    let detail_str = byte_count_string(file_size);
                    let font_detail: Retained<NSFont> = unsafe { NSFont::systemFontOfSize(10.0 * scale) };
                    let color_detail = unsafe { NSColor::secondaryLabelColor() };
                    let detail_dict: Retained<NSDictionary<objc2_foundation::NSAttributedStringKey>> = unsafe {
                        let keys = NSArray::from_slice(&[
                            objc2_app_kit::NSFontAttributeName,
                            objc2_app_kit::NSForegroundColorAttributeName,
                            objc2_app_kit::NSParagraphStyleAttributeName,
                        ]);
                        let objs: [&NSObject; 3] = [
                            &*font_detail as &NSObject,
                            &*color_detail as &NSObject,
                            &*pstyle_detail as &NSObject,
                        ];
                        let values = NSArray::from_slice(&objs);
                        let dict_ptr: *mut NSDictionary<objc2_foundation::NSAttributedStringKey> =
                            objc2::msg_send![objc2::class!(NSDictionary), dictionaryWithObjects: &*values, forKeys: &*keys];
                        Retained::retain(dict_ptr).unwrap()
                    };
                    unsafe { detail_str.drawInRect_withAttributes(detail_rect, Some(&*detail_dict)) };
                }
            }
            let _ = mtm;
            let _ = state;
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &objc2_app_kit::NSEvent) {
            let state = unsafe { &mut *app_state() };
            let code = event.keyCode();
            let mods = event.modifierFlags();
            let shift = mods.contains(objc2_app_kit::NSEventModifierFlags::Shift);
            let command = mods.contains(objc2_app_kit::NSEventModifierFlags::Command);
            let control = mods.contains(objc2_app_kit::NSEventModifierFlags::Control);
            if code == 51 && (command || control || (!shift && !command)) {
                if state.mode == BrowserMode::Browser && code == 51 && !command && !control {
                    // Backspace: up one level (handled below via navigateToParent
                    // in the Return branch pattern); fallthrough to parent nav.
                } else {
                    trash_selected(self);
                    return;
                }
            }
            if command && (code == 27 || code == 24 || code == 69 || code == 78) {
                state.thumbnail_size_step = (state.thumbnail_size_step + if code == 24 || code == 69 { 1 } else { -1 }).clamp(-3, 4);
                apply_view_mode_layout();
                self.scroll_selection_into_view();
                return;
            }
            if code == 49 && !command && !control && !shift {
                quicklook_selected();
                return;
            }
            if code == 6 && command {
                if shift {
                    redo_trash(self);
                } else {
                    undo_trash(self);
                }
                return;
            }
            // History (shift arrows) — browser mode only.
            if code == 123 && shift {
                if let Some((url, sel)) = state.back.pop() {
                    let current = state.root.clone();
                    state.forward.push((current, state.selected.and_then(|i| state.items.get(i)).map(|i| i.path.clone())));
                    state.root = url;
                    state.items = load_listing(&state.root, state.mode);
                    state.selected = sel.and_then(|s| state.items.iter().position(|i| i.path == s));
                    let idx = state.selected.unwrap_or(0);
                    state.selection_anchor = state.selected;
                    state.selection_range = (idx, idx);
                    let title = state.root.display().to_string();
                    if let Some(refs) = window_refs().as_ref() {
                        unsafe { (&*refs.window).setTitle(&NSString::from_str(&title)); }
                    }
                    apply_view_mode_layout();
                    update_carousel_preview();
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
                    let idx = state.selected.unwrap_or(0);
                    state.selection_anchor = state.selected;
                    state.selection_range = (idx, idx);
                    let title = state.root.display().to_string();
                    if let Some(refs) = window_refs().as_ref() {
                        unsafe { (&*refs.window).setTitle(&NSString::from_str(&title)); }
                    }
                    apply_view_mode_layout();
                    update_carousel_preview();
                }
                self.setNeedsDisplay(true);
                return;
            }
            if code == 51 && !command && !control {
                if state.mode == BrowserMode::Browser {
                    let parent = state.root.parent().map(|p| p.to_path_buf());
                    if let Some(parent_url) = parent {
                        if parent_url != state.root {
                            let current = state.root.clone();
                            state.back.push((current, state.selected.and_then(|i| state.items.get(i)).map(|x| x.path.clone())));
                            state.forward.clear();
                            state.root = parent_url;
                            state.items = load_listing(&state.root, state.mode);
                            state.selected = if state.items.is_empty() { None } else { Some(0) };
                            state.selection_anchor = state.selected;
                            state.selection_range = (0, 0);
                            let title = state.root.display().to_string();
                            if let Some(refs) = window_refs().as_ref() {
                                unsafe { (&*refs.window).setTitle(&NSString::from_str(&title)); }
                            }
                            apply_view_mode_layout();
                            update_carousel_preview();
                            self.setNeedsDisplay(true);
                        }
                    }
                }
                return;
            }
            if state.items.is_empty() {
                return;
            }
            let visible_cols = columns_for(self.bounds().size.width).max(1);
            let cols = if state.view_mode == ViewMode::Carousel || visible_cols >= state.items.len() { 1 } else { visible_cols };
            let cur = state.selected.unwrap_or(0);
            let next = match code {
                123 | 126 => cur as isize - if code == 126 { cols as isize } else { 1 },
                124 | 125 => cur as isize + if code == 125 { cols as isize } else { 1 },
                36 => {
                    if let Some(item) = state.items.get(cur).filter(|item| item.kind == MediaKind::Directory) {
                        enter_directory(item.path.clone());
                    }
                    return;
                }
                _ => return,
            };
            let n = state.items.len() as isize;
            let wrapped = ((next % n) + n) % n;
            state.selected = Some(wrapped as usize);
            state.selection_anchor = Some(wrapped as usize);
            state.selection_range = (wrapped as usize, wrapped as usize);
            self.scroll_selection_into_view();
            update_carousel_preview();
            self.setNeedsDisplay(true);
            let _ = command;
        }

        #[unsafe(method(openSelected:))]
        fn open_selected_action(&self, _sender: &NSObject) {
            open_selected();
        }

        #[unsafe(method(showSelectedInFinder:))]
        fn show_selected_action(&self, _sender: &NSObject) {
            show_selected_in_finder();
        }

        #[unsafe(method(copySelectedPath:))]
        fn copy_selected_action(&self, _sender: &NSObject) {
            copy_selected_path();
        }

        #[unsafe(method(moveSelectedToTrash:))]
        fn move_selected_action(&self, _sender: &NSObject) {
            trash_selected(self);
        }

        #[unsafe(method(undoTrash:))]
        fn undo_trash_action(&self, _sender: &NSObject) {
            undo_trash(self);
        }

        #[unsafe(method(redoTrash:))]
        fn redo_trash_action(&self, _sender: &NSObject) {
            redo_trash(self);
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &objc2_app_kit::NSEvent) {
            let location = event.locationInWindow();
            let view_location = self.convertPoint_fromView(location, None);
            let state = unsafe { &mut *app_state() };
            if let Some(index) = self.index_at_point(view_location) {
                state.selected = Some(index);
                state.selection_anchor = Some(index);
                state.selection_range = (index, index);
                update_carousel_preview();
                self.setNeedsDisplay(true);
            }
            place_context_menu(self, event);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &objc2_app_kit::NSEvent) {
            self.ivars().drag_candidate.set(false);
            self.ivars().deferred_click.set(None);
            let location = event.locationInWindow();
            let view_location = self.convertPoint_fromView(location, None);
            let state = unsafe { &mut *app_state() };
            if state.items.is_empty() {
                return;
            }
            let Some(index) = self.index_at_point(view_location) else { return };
            self.ivars().drag_candidate.set(true);
            if event.clickCount() >= 2 {
                if let Some(item) = state.items.get(index).filter(|item| item.kind == MediaKind::Directory) {
                    enter_directory(item.path.clone());
                }
            } else if index < state.items.len() {
                let shift = event.modifierFlags().contains(objc2_app_kit::NSEventModifierFlags::Shift);
                if shift {
                    if let Some(anchor) = state.selection_anchor {
                        state.selection_range = (anchor.min(index), anchor.max(index));
                    } else {
                        state.selection_anchor = Some(index);
                        state.selection_range = (index, index);
                    }
                    state.selected = Some(index);
                } else if state.selection_anchor.is_some() && (state.selection_range.0..=state.selection_range.1).contains(&index) {
                    self.ivars().deferred_click.set(Some(index));
                } else {
                    state.selection_anchor = Some(index);
                    state.selection_range = (index, index);
                    state.selected = Some(index);
                }
                update_carousel_preview();
            }
            self.scroll_selection_into_view();
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &objc2_app_kit::NSEvent) {
            self.ivars().drag_candidate.set(false);
            if let Some(index) = self.ivars().deferred_click.take() {
                let state = unsafe { &mut *app_state() };
                state.selected = Some(index);
                state.selection_anchor = Some(index);
                state.selection_range = (index, index);
                update_carousel_preview();
                self.setNeedsDisplay(true);
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &objc2_app_kit::NSEvent) {
            if !self.ivars().drag_candidate.replace(false) { return; }
            self.ivars().deferred_click.set(None);
            begin_drag_session(self, event);
        }

        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, sender: &objc2::runtime::ProtocolObject<dyn objc2_app_kit::NSDraggingInfo>) -> objc2_app_kit::NSDragOperation {
            if dropped_path(&sender.draggingPasteboard()).is_some() {
                objc2_app_kit::NSDragOperation::Copy
            } else {
                objc2_app_kit::NSDragOperation::empty()
            }
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag_operation(&self, sender: &objc2::runtime::ProtocolObject<dyn objc2_app_kit::NSDraggingInfo>) -> bool {
            accept_dropped_path(self, &sender.draggingPasteboard())
        }

        #[unsafe(method(acceptsPreviewPanelControl:))]
        fn accepts_preview_panel_control(&self, _panel: &QLPreviewPanel) -> bool {
            true
        }

        #[unsafe(method(beginPreviewPanelControl:))]
        fn begin_preview_panel_control(&self, panel: &QLPreviewPanel) {
            unsafe {
                panel.setDataSource(Some(objc2::runtime::ProtocolObject::from_ref(self)));
                let index = (*app_state()).selected.unwrap_or(0) as objc2_foundation::NSInteger;
                panel.setCurrentPreviewItemIndex(index);
                let _: () = objc2::msg_send![panel, addObserver: self, forKeyPath: &*NSString::from_str("currentPreviewItemIndex"), options: 0usize, context: core::ptr::null_mut::<core::ffi::c_void>()];
            }
        }

        #[unsafe(method(endPreviewPanelControl:))]
        fn end_preview_panel_control(&self, panel: &QLPreviewPanel) {
            unsafe {
                let _: () = objc2::msg_send![panel, removeObserver: self, forKeyPath: &*NSString::from_str("currentPreviewItemIndex")];
                panel.setDataSource(None);
            }
        }

        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_preview_index(&self, _key: &NSString, panel: &QLPreviewPanel, _change: &NSDictionary, _context: *mut core::ffi::c_void) {
            let index = unsafe { panel.currentPreviewItemIndex() };
            let state = unsafe { &mut *app_state() };
            if index >= 0 && (index as usize) < state.items.len() {
                let index = index as usize;
                state.selected = Some(index);
                state.selection_anchor = Some(index);
                state.selection_range = (index, index);
                self.scroll_selection_into_view();
                update_carousel_preview();
                self.setNeedsDisplay(true);
            }
        }

        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn number_of_preview_items_in_preview_panel(&self, _panel: Option<&QLPreviewPanel>) -> objc2_foundation::NSInteger {
            unsafe { &*app_state() }.items.len() as objc2_foundation::NSInteger
        }

        #[unsafe(method_id(previewPanel:previewItemAtIndex:))]
        fn preview_panel_preview_item_at_index(
            &self,
            _panel: Option<&QLPreviewPanel>,
            index: objc2_foundation::NSInteger,
        ) -> Retained<NSURL> {
            let item = unsafe { &*app_state() }.items.get(index as usize).cloned();
            let item = item.unwrap_or(MediaItem::new("/", MediaKind::Directory));
            url_from_path(&item.path)
        }
    }

    unsafe impl NSDraggingSource for GridView {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn draggingSession_sourceOperationMaskForDraggingContext(
            &self,
            _session: &objc2_app_kit::NSDraggingSession,
            _context: objc2_app_kit::NSDraggingContext,
        ) -> objc2_app_kit::NSDragOperation {
            objc2_app_kit::NSDragOperation::Copy
        }

        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn draggingSession_endedAtPoint_operation(
            &self,
            _session: &objc2_app_kit::NSDraggingSession,
            _screen_point: objc2_foundation::NSPoint,
            _operation: objc2_app_kit::NSDragOperation,
        ) {
            DRAG_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }
);

unsafe impl objc2_foundation::NSObjectProtocol for GridView {}

impl GridView {
    fn cell_rect(&self, index: usize) -> NSRect {
        let state = unsafe { &*app_state() };
        let width = self.bounds().size.width;
        let scale = thumbnail_scale();
        if state.view_mode == ViewMode::Carousel {
            let x = 12.0 + index as f64 * (CAROUSEL_CELL_W * scale + CAROUSEL_SPACING);
            return NSRect::new(NSPoint::new(x, 14.0), NSSize::new(CAROUSEL_CELL_W * scale, CAROUSEL_CELL_H * scale));
        }
        let cols = columns_for(width).max(1);
        let spacing = grid_cell_spacing(width);
        let col = index % cols;
        let row = index / cols;
        NSRect::new(
            NSPoint::new(16.0 + col as f64 * (CELL_W * scale + spacing), 16.0 + row as f64 * (CELL_H * scale + SPACING)),
            NSSize::new(CELL_W * scale, CELL_H * scale),
        )
    }

    fn index_at_point(&self, point: NSPoint) -> Option<usize> {
        let state = unsafe { &*app_state() };
        let scale = thumbnail_scale();
        let index = if state.view_mode == ViewMode::Carousel {
            if point.x < 12.0 { return None; }
            ((point.x - 12.0) / (CAROUSEL_CELL_W * scale + CAROUSEL_SPACING)) as usize
        } else {
            if point.x < 16.0 || point.y < 16.0 { return None; }
            let width = self.bounds().size.width;
            let columns = columns_for(width);
            let column = ((point.x - 16.0) / (CELL_W * scale + grid_cell_spacing(width))) as usize;
            if column >= columns { return None; }
            let row = ((point.y - 16.0) / (CELL_H * scale + SPACING)) as usize;
            row.checked_mul(columns)?.checked_add(column)?
        };
        (index < state.items.len() && objc2_foundation::NSPointInRect(point, self.cell_rect(index))).then_some(index)
    }

    fn scroll_selection_into_view(&self) {
        let index = unsafe { (*app_state()).selected };
        let Some(index) = index else { return };
        let cell = self.cell_rect(index);
        unsafe { self.scrollRectToVisible(cell); };
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

    unsafe impl NSWindowDelegate for AppDelegate {
        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _notification: &NSNotification) {
            apply_view_mode_layout();
        }
    }

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
            let root_view: Retained<PreviewRegionView> = unsafe {
                objc2::msg_send![PreviewRegionView::alloc(objc2::MainThreadMarker::new().unwrap()), initWithFrame: frame]
            };
            let preview_nsview: Retained<PreviewRegionView> = unsafe {
                objc2::msg_send![PreviewRegionView::alloc(objc2::MainThreadMarker::new().unwrap()), initWithFrame: frame]
            };
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
            window.setBackgroundColor(Some(&NSColor::windowBackgroundColor()));
            root_view.setWantsLayer(true);
            preview_nsview.setWantsLayer(true);
            divider.setBoxType(objc2_app_kit::NSBoxType::Separator);
            scroll.setDrawsBackground(true);
            scroll.setBackgroundColor(&NSColor::windowBackgroundColor());
            root_view.addSubview(&*preview_nsview);
            root_view.addSubview(&*divider);
            root_view.addSubview(&*scroll);
            layout_carousel(&root_view, &preview_nsview, &preview_image, &divider, &scroll, frame, view_mode_carousel);
            let doc_size = content_size(frame_size.width, frame_size.height, item_count, view_mode_carousel);
            grid.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), doc_size));
            grid.setNeedsDisplay(true);
            scroll.setDocumentView(Some(&*grid));
            window.setContentView(Some(&*root_view));
            window.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(self)));
            *window_refs() = Some(WindowRefs {
                window: Retained::into_raw(window.clone()) as *mut NSWindow,
                scroll: Retained::into_raw(scroll.clone()) as *mut NSScrollView,
                grid: Retained::into_raw(grid.clone()) as *mut GridView,
                preview: Retained::into_raw(preview_nsview.clone()) as *mut NSView,
                divider: Retained::into_raw(divider.clone()) as *mut NSBox,
                preview_image: Retained::into_raw(preview_image.clone()) as *mut NSImageView,
            });
            update_carousel_preview();
            install_main_menu(self, view_mode_carousel, &grid);
            if *DEBUG_ENTER.get_or_init(|| false) {
                open_selected();
                std::process::exit(0);
            }
            if *SELF_TEST.get_or_init(|| false) {
                start_self_test();
            }
            if !*REGRESSION_TEST.get_or_init(|| false) {
                window.setFrameAutosaveName(&NSString::from_str("ViewWindow"));
            }
            window.setTitle(&NSString::from_str(&title));
            window.center();
            window.makeKeyAndOrderFront(None);
            let _ = window.makeFirstResponder(Some(&*grid));
            let mtm = objc2::MainThreadMarker::new().unwrap();
            let app = NSApplication::sharedApplication(mtm);
            #[allow(deprecated)]
            unsafe { app.activateIgnoringOtherApps(true); }
            if *REGRESSION_TEST.get_or_init(|| false) {
                let window = window.clone();
                let grid = grid.clone();
                let preview_image = preview_image.clone();
                let block = block2::RcBlock::new(move |_timer: core::ptr::NonNull<objc2_foundation::NSTimer>| {
                    run_regression_test(&window, &grid, &preview_image);
                });
                unsafe { objc2_foundation::NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.01, false, &block) };
            }
            let _ = window;
        }
    }
);

fn assert_orientation(bitmap: &objc2_app_kit::NSBitmapImageRep) {
    let pixel = |x: isize, y: isize| bitmap.colorAtX_y(x * bitmap.pixelsWide() as isize / 4, y * bitmap.pixelsHigh() as isize / 4).expect("preview pixel");
    let upper_left = pixel(1, 1);
    let upper_right = pixel(3, 1);
    let lower_left = pixel(1, 3);
    let lower_right = pixel(3, 3);
    assert!(upper_left.redComponent() > upper_left.greenComponent() * 2.0 && upper_left.redComponent() > upper_left.blueComponent() * 2.0, "top left is red: {upper_left:?}");
    assert!(upper_right.greenComponent() > upper_right.redComponent() * 2.0 && upper_right.greenComponent() > upper_right.blueComponent() * 2.0, "top right is green: {upper_right:?}");
    assert!(lower_left.blueComponent() > lower_left.redComponent() * 2.0 && lower_left.blueComponent() > lower_left.greenComponent() * 2.0, "bottom left is blue: {lower_left:?}");
    assert!(lower_right.redComponent() > lower_right.blueComponent() * 2.0 && lower_right.greenComponent() > lower_right.blueComponent() * 2.0, "bottom right is yellow: {lower_right:?}");
}

fn run_regression_test(window: &NSWindow, grid: &GridView, preview: &NSImageView) {
    unsafe { (*app_state()).view_mode = ViewMode::Carousel; }
    apply_view_mode_layout();
    assert_eq!(unsafe { (*app_state()).items.len() }, 1);
    open_selected();
    assert_eq!(unsafe { (*app_state()).selected }, Some(0));
    assert!(preview.image().is_some(), "first image is visible after entering a folder");
    let bounds = preview.bounds();
    let bitmap = preview.bitmapImageRepForCachingDisplayInRect(bounds).expect("preview bitmap");
    preview.cacheDisplayInRect_toBitmapImageRep(bounds, &bitmap);
    assert_orientation(&bitmap);
    assert!(unsafe { (*app_state()).items.len() } >= 3);
    let folder = unsafe { (*app_state()).root.clone() };
    let parent = folder.parent().unwrap().to_path_buf();
    assert_eq!(window.title().to_string(), folder.display().to_string(), "directory entry updates the window title");
    let app = NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap());
    window.makeKeyAndOrderFront(None);
    let _ = window.makeFirstResponder(Some(grid));
    send_key_via_app(&app, 123, false);
    assert_eq!(unsafe { (*app_state()).selected }, Some(unsafe { (*app_state()).items.len() } - 1), "left from the first item wraps to the last");
    send_key_via_app(&app, 124, false);
    assert_eq!(unsafe { (*app_state()).selected }, Some(0), "right from the last item wraps to the first");
    send_key_via_app(&app, 123, true);
    assert_eq!(unsafe { (*app_state()).root.clone() }, parent, "back returns to the previous folder");
    assert_eq!(unsafe { (*app_state()).selected.and_then(|i| (&(*app_state()).items).get(i)).map(|item| item.path.clone()) }, Some(folder.clone()), "back restores the directory selection");
    assert_eq!(window.title().to_string(), parent.display().to_string(), "back updates the window title");
    send_key_via_app(&app, 124, true);
    assert_eq!(unsafe { (*app_state()).root.clone() }, folder, "forward reopens the folder");
    assert_eq!(unsafe { (*app_state()).selected }, Some(0), "forward restores the selected item");
    assert_eq!(window.title().to_string(), folder.display().to_string(), "forward updates the window title");

    window.makeKeyAndOrderFront(None);
    let _ = window.makeFirstResponder(Some(grid));
    quicklook_selected();
    let panel = unsafe { QLPreviewPanel::sharedPreviewPanel(objc2::MainThreadMarker::new().unwrap()) }.unwrap();
    assert!(unsafe { panel.dataSource() }.is_some(), "quick look obtains its data source from the responder; window key={} main={} panel key={} active={}", window.isKeyWindow(), window.isMainWindow(), panel.isKeyWindow(), NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap()).isActive());
    let event = objc2_app_kit::NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        objc2_app_kit::NSEventType::KeyDown, NSPoint::ZERO,
        objc2_app_kit::NSEventModifierFlags::empty(), 0.0, panel.windowNumber(), None,
        &NSString::from_str("\u{f703}"), &NSString::from_str("\u{f703}"), false, 124,
    ).unwrap();
    panel.sendEvent(&event);
    assert_eq!(unsafe { (*app_state()).selected }, Some(1), "quick look arrow advances selection");
    unsafe { panel.setCurrentPreviewItemIndex(1); }
    assert_eq!(unsafe { (*app_state()).selected }, Some(1), "quick look advances selection");
    let event = objc2_app_kit::NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        objc2_app_kit::NSEventType::KeyDown, NSPoint::ZERO,
        objc2_app_kit::NSEventModifierFlags::empty(), 0.0, panel.windowNumber(), None,
        &NSString::from_str("\u{f702}"), &NSString::from_str("\u{f702}"), false, 123,
    ).unwrap();
    panel.sendEvent(&event);
    assert_eq!(unsafe { (*app_state()).selected }, Some(0), "quick look arrow moves selection backwards");
    panel.orderOut(None);
    window.makeKeyAndOrderFront(None);
    let _ = window.makeFirstResponder(Some(grid));
    quicklook_selected();
    assert!(unsafe { panel.dataSource() }.is_some(), "quick look can reopen");
    unsafe { panel.setCurrentPreviewItemIndex(2); }
    assert_eq!(unsafe { (*app_state()).selected }, Some(2), "reopened quick look updates selection");
    panel.orderOut(None);

    unsafe { (*app_state()).view_mode = ViewMode::Grid; }
    window.setContentSize(NSSize::new(400.0, 500.0));
    let narrow = grid.cell_rect(2);
    window.setContentSize(NSSize::new(800.0, 500.0));
    let wide = grid.cell_rect(2);
    assert!(wide.origin.y < narrow.origin.y, "window resize reflows columns");
    assert!((grid.bounds().size.width - window.contentView().unwrap().bounds().size.width).abs() < 1.0);
    assert_eq!(grid.index_at_point(NSPoint::new(wide.origin.x + 2.0, wide.origin.y + 2.0)), Some(2));
    let thumb = grid.cell_rect(0);
    let bounds = NSRect::new(NSPoint::new(thumb.origin.x + 6.0, thumb.origin.y + 6.0), NSSize::new(148.0, 134.0));
    let bitmap = grid.bitmapImageRepForCachingDisplayInRect(bounds).expect("thumbnail bitmap");
    grid.cacheDisplayInRect_toBitmapImageRep(bounds, &bitmap);
    assert_orientation(&bitmap);
    window.makeKeyAndOrderFront(None);
    let _ = window.makeFirstResponder(Some(grid));
    send_key_via_app(&app, 125, false);
    assert_eq!(unsafe { (*app_state()).selected }, Some(6), "down moves by the visible column count");
    send_key_via_app(&app, 126, false);
    assert_eq!(unsafe { (*app_state()).selected }, Some(2), "up returns by the visible column count");
    let row_before_zoom = grid.cell_rect(3).origin.y;
    send_key_command(&app, 24);
    assert!(grid.cell_rect(0).size.width > CELL_W, "command plus increases thumbnails");
    assert!(grid.cell_rect(3).origin.y > row_before_zoom, "thumbnail size reflows the grid");
    send_key_command(&app, 27);
    assert!((grid.cell_rect(0).size.width - CELL_W).abs() < 0.01, "command minus decreases thumbnails");
    unsafe { (*app_state()).view_mode = ViewMode::Carousel; }
    apply_view_mode_layout();
    let strip_height = unsafe { (&*window_refs().as_ref().unwrap().scroll).frame().size.height };
    send_key_command(&app, 24);
    assert!(grid.cell_rect(0).size.width > CAROUSEL_CELL_W, "carousel thumbnails increase");
    assert!(unsafe { (&*window_refs().as_ref().unwrap().scroll).frame().size.height } > strip_height);
    send_key_command(&app, 27);

    let state = unsafe { &mut *app_state() };
    state.selected = Some(1);
    state.selection_anchor = Some(0);
    state.selection_range = (0, 1);
    assert!(grid.respondsToSelector(objc2::sel!(draggingSession:sourceOperationMaskForDraggingContext:)), "drag source operation callback is registered");
    assert!(grid.respondsToSelector(objc2::sel!(draggingSession:endedAtPoint:operation:)), "drag completion callback is registered");
    DRAG_ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
    let session = NSObject::new();
    unsafe {
        let _: () = objc2::msg_send![grid, draggingSession: &*session, endedAtPoint: NSPoint::ZERO, operation: objc2_app_kit::NSDragOperation::empty()];
    }
    assert!(!DRAG_ACTIVE.load(std::sync::atomic::Ordering::SeqCst), "completed drag permits another session");
    let items = dragging_items(grid);
    assert_eq!(items.len(), 2, "drag contains the selection");
    for (index, item) in items.iter().enumerate() {
        let writer = item.item();
        let url = writer.downcast_ref::<NSURL>().expect("file URL pasteboard writer");
        assert_eq!(path_from_url(url), unsafe { (&(*app_state()).items)[index].path.clone() });
    }
    assert!(grid.respondsToSelector(objc2::sel!(draggingEntered:)), "drop negotiation is registered");
    assert!(grid.respondsToSelector(objc2::sel!(performDragOperation:)), "drop handler is registered");
    let pasteboard = objc2_app_kit::NSPasteboard::pasteboardWithUniqueName();
    let folder = unsafe { (*app_state()).root.clone() };
    let parent = folder.parent().unwrap();
    let file = folder.join("weird % café.png");
    assert_eq!(path_from_url(&url_from_path(&file)), file);
    let url = url_from_path(&file);
    pasteboard.clearContents();
    assert!(unsafe { pasteboard.setString_forType(&url.absoluteString().unwrap(), &objc2_app_kit::NSPasteboardTypeFileURL) });
    enter_directory(parent.to_path_buf());
    assert!(accept_dropped_path(grid, &pasteboard));
    assert_eq!(unsafe { (*app_state()).root.clone() }, folder);
    assert_eq!(unsafe { (&(*app_state()).items)[(*app_state()).selected.unwrap()].path.clone() }, file);
    let url = url_from_path(parent);
    pasteboard.clearContents();
    assert!(unsafe { pasteboard.setString_forType(&url.absoluteString().unwrap(), &objc2_app_kit::NSPasteboardTypeFileURL) });
    assert!(accept_dropped_path(grid, &pasteboard));
    assert_eq!(unsafe { (*app_state()).root.clone() }, parent);
    send_key_via_app(&app, 36, false);
    assert_eq!(unsafe { (*app_state()).root.clone() }, folder, "return enters the selected directory");
    assert!(preview.image().is_some(), "return paints the first carousel image");
    let original = folder.join("a.png");
    let trashed = parent.join(".trashed-a.png");
    let content = std::fs::read(&original).unwrap();
    std::fs::rename(&original, &trashed).unwrap();
    {
        let state = unsafe { &mut *app_state() };
        let index = state.items.iter().position(|item| item.path == original).unwrap();
        let item = state.items.remove(index);
        state.selected = Some(0);
        state.trash_history.record_trash(TrashTransaction {
            files: vec![view_core::TrashedFile { item, trashed_path: trashed.clone(), index }],
            selection_before_op: vec![original.clone()],
        });
    }
    send_key_command(&app, 6);
    assert_eq!(std::fs::read(&original).unwrap(), content, "undo restores the file contents");
    assert!(!trashed.exists(), "undo removes the trashed copy");
    assert!(unsafe { (*app_state()).items.iter().any(|item| item.path == original) }, "undo restores the catalog item");
    unsafe { let _: () = objc2::msg_send![grid, redoTrash: grid]; }
    assert!(!original.exists(), "redo removes the original file");
    assert_eq!(std::fs::read(&trashed).unwrap(), content, "redo preserves the file contents");
    assert!(!unsafe { (*app_state()).items.iter().any(|item| item.path == original) }, "redo removes the catalog item");
    std::fs::write(&original, b"replacement").unwrap();
    send_key_command(&app, 6);
    assert_eq!(std::fs::read(&original).unwrap(), b"replacement", "undo preserves a conflicting file");
    assert_eq!(std::fs::read(&trashed).unwrap(), content, "conflict leaves the trashed file intact");
    assert!(!unsafe { (*app_state()).items.iter().any(|item| item.path == original) }, "conflict does not restore the catalog item");
    std::fs::remove_file(&original).unwrap();
    send_key_command(&app, 6);
    assert_eq!(std::fs::read(&original).unwrap(), content, "undo can be retried after resolving a conflict");
    let album = folder.join("album");
    let trashed_album = parent.join(".trashed-album");
    std::fs::create_dir(&album).unwrap();
    std::fs::write(album.join("inside.png"), &content).unwrap();
    std::fs::rename(&album, &trashed_album).unwrap();
    unsafe { (&mut *app_state()).trash_history.record_trash(TrashTransaction {
        files: vec![view_core::TrashedFile { item: MediaItem::new(&album, MediaKind::Directory), trashed_path: trashed_album.clone(), index: 0 }],
        selection_before_op: vec![album.clone()],
    }); }
    send_key_command(&app, 6);
    assert_eq!(std::fs::read(album.join("inside.png")).expect("undo restores folder contents"), content);
    assert!(!trashed_album.exists(), "undo removes the trashed folder");
    std::fs::create_dir(&trashed_album).unwrap();
    std::fs::write(trashed_album.join("replacement"), b"replacement").unwrap();
    unsafe { let _: () = objc2::msg_send![grid, redoTrash: grid]; }
    assert_eq!(std::fs::read(album.join("inside.png")).unwrap(), content, "redo preserves the source on conflict");
    assert_eq!(std::fs::read(trashed_album.join("replacement")).unwrap(), b"replacement", "redo preserves the conflicting destination");
    assert!(unsafe { (*app_state()).items.iter().any(|item| item.path == album) }, "redo conflict leaves the folder in the catalog");
    std::fs::remove_file(trashed_album.join("replacement")).unwrap();
    std::fs::remove_dir(&trashed_album).unwrap();
    unsafe { let _: () = objc2::msg_send![grid, redoTrash: grid]; }
    assert!(!album.exists(), "redo removes the original folder");
    assert_eq!(std::fs::read(trashed_album.join("inside.png")).expect("redo preserves folder contents"), content);
    let video_dir = folder.join("video");
    std::fs::create_dir(&video_dir).unwrap();
    let video = video_dir.join("clip.mp4");
    std::fs::copy(parent.join(".video.mp4"), &video).unwrap();
    unsafe { (*app_state()).view_mode = ViewMode::Carousel; }
    enter_directory(video_dir);
    assert_eq!(unsafe { (&(*app_state()).items)[0].kind }, MediaKind::Movie);
    assert!(preview.image().is_none(), "video preview waits for its thumbnail");
    let started = std::time::Instant::now();
    let block = block2::RcBlock::new(move |_timer: core::ptr::NonNull<objc2_foundation::NSTimer>| {
        if cached_thumbnail(&video).is_some() {
            let image = {
                let refs = window_refs();
                unsafe { (&*refs.as_ref().unwrap().preview_image).image() }
            };
            assert!(image.is_some(), "first video preview updates when thumbnail completes");
            eprintln!("app regression: PASS");
            std::process::exit(0);
        }
        assert!(started.elapsed().as_secs() < 10, "video thumbnail did not complete");
    });
    unsafe { objc2_foundation::NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.05, true, &block) };
}

static SELF_TEST_STEP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static SELF_TEST_SELECTED_BEFORE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);
static SELF_TEST_ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
static SELF_TEST_ENTERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SELF_TEST_TRASH_TARGET: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

fn target_window() -> Option<*mut NSWindow> {
    let refs = window_refs();
    refs.as_ref().map(|r| r.window)
}

#[allow(dead_code)]
fn send_shift_mouse_via_app(_app: &NSApplication, point: objc2_foundation::NSPoint) {
    let Some(window_ptr) = target_window() else { return };
    let window = unsafe { &*window_ptr };
    let flags = objc2_app_kit::NSEventModifierFlags::Shift;
    let event = unsafe {
        objc2_app_kit::NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            objc2_app_kit::NSEventType::LeftMouseDown,
            point,
            flags,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0),
            window.windowNumber(),
            None,
            0,
            1,
            0.0,
        )
    };
    if let Some(event) = event {
        window.sendEvent(&event);
    }
}

fn send_key_command(_app: &NSApplication, key_code: u16) {
    let Some(window_ptr) = target_window() else { return };
    let window = unsafe { &*window_ptr };
    let event = unsafe {
        objc2_app_kit::NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            objc2_app_kit::NSEventType::KeyDown,
            objc2_foundation::NSPoint::ZERO,
            objc2_app_kit::NSEventModifierFlags::Command,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0),
            window.windowNumber(),
            None,
            &NSString::from_str(""),
            &NSString::from_str(""),
            false,
            key_code,
        )
    };
    if let Some(event) = event {
        window.sendEvent(&event);
    }
}

fn send_mouse_via_app(_app: &NSApplication, count: objc2_foundation::NSInteger) {
    let Some(window_ptr) = target_window() else { return };
    let window = unsafe { &*window_ptr };
    let dir_index = unsafe { (*app_state()).items.iter().position(|i| i.kind == MediaKind::Directory) };
    let index = dir_index.unwrap_or(0);
    let point = objc2_foundation::NSPoint::new(12.0 + index as f64 * 128.0 + 60.0, 92.0);
    let flags = objc2_app_kit::NSEventModifierFlags::empty();
    let event = unsafe {
        objc2_app_kit::NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            objc2_app_kit::NSEventType::LeftMouseDown,
            point,
            flags,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0),
            window.windowNumber(),
            None,
            0,
            count,
            0.0,
        )
    };
    if let Some(event) = event {
        window.sendEvent(&event);
    }
}

fn send_key_via_app(_app: &NSApplication, key_code: u16, shift: bool) {
    let Some(window_ptr) = target_window() else { return };
    let window = unsafe { &*window_ptr };
    let flags = if shift {
        objc2_app_kit::NSEventModifierFlags::Shift
    } else {
        objc2_app_kit::NSEventModifierFlags::empty()
    };
    let event = unsafe {
        objc2_app_kit::NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            objc2_app_kit::NSEventType::KeyDown,
            objc2_foundation::NSPoint::ZERO,
            flags,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0),
            window.windowNumber(),
            None,
            &NSString::from_str(""),
            &NSString::from_str(""),
            false,
            key_code,
        )
    };
    if let Some(event) = event {
        window.sendEvent(&event);
    }
}

fn start_self_test() {
    let mtm = objc2::MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    let block = unsafe {
        block2::RcBlock::new(move |_timer: core::ptr::NonNull<objc2_foundation::NSTimer>| {
            let step = SELF_TEST_STEP.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match step {
                0 => {
                    SELF_TEST_ROOT.set(unsafe { (*app_state()).root.clone() }).ok();
                    #[allow(deprecated)] unsafe { app.activateIgnoringOtherApps(true) };
                    if let Some(refs) = window_refs().as_ref() {
                        unsafe { &*refs.window }.makeKeyWindow();
                    }
                    SELF_TEST_SELECTED_BEFORE.store(
                        unsafe { (*app_state()).selected.unwrap_or(usize::MAX) },
                        std::sync::atomic::Ordering::SeqCst,
                    );
                    send_key_via_app(&app, 124, false);
                }
                1 => {
                    send_key_via_app(&app, 124, false);
                    let width = app.keyWindow().map(|w| w.frame().size.width.max(400.0)).unwrap_or(400.0);
                    let _ = width;
                    let point = objc2_foundation::NSPoint::new(12.0 + 60.0, 92.0);
                    send_shift_mouse_via_app(&app, point);
                }
                2 => {
                    send_mouse_via_app(&app, 2);
                    send_mouse_via_app(&app, 2);
                }
                3 => {
                    let entered = match SELF_TEST_ROOT.get() {
                        Some(start) => { let cur = unsafe { &*app_state() }.root.clone(); cur != *start }
                        None => false,
                    };
                    let _ = app;
                    SELF_TEST_ENTERED.store(entered, std::sync::atomic::Ordering::SeqCst);
                    send_key_via_app(&app, 123, true);
                }
                4 => {
                    // History back already sent
                }
                5 => {
                    let image = unsafe {
                        (&*app_state())
                            .items
                            .iter()
                            .find(|i| i.kind == MediaKind::Image)
                            .map(|i| i.path.clone())
                    };
                    if let Some(path) = image {
                        SELF_TEST_TRASH_TARGET.set(path).ok();
                        send_key_command(&app, 51);
                    }
                }
                6 => {
                    send_key_command(&app, 6);
                }
                _ => {
                    let before = SELF_TEST_SELECTED_BEFORE.load(std::sync::atomic::Ordering::SeqCst);
                    let before = if before == usize::MAX { None } else { Some(before) };
                    let now = unsafe { (*app_state()).selected };
                    let root = unsafe { (*app_state()).root.clone() };
                    let has_directory = unsafe { (*app_state()).items.iter().any(|i| i.kind == MediaKind::Directory) };
                    let double_click_ok = if has_directory {
                        SELF_TEST_ENTERED.load(std::sync::atomic::Ordering::SeqCst)
                    } else {
                        before != now && now.is_some()
                    };
                    let trash_ok = true;
                    let undo_ok = match SELF_TEST_TRASH_TARGET.get() {
                        Some(target) => target.exists(),
                        None => true,
                    };
                    if now.is_some() && double_click_ok && trash_ok && undo_ok {
                        eprintln!("self-test: PASS (arrows, double-click enter, shift-left history via NSApplication.sendEvent)");
                        std::process::exit(0);
                    } else {
                        eprintln!(
                            "self-test: FAIL (before={before:?} now={now:?} root={} count={})",
                            root.display(),
                            unsafe { (*app_state()).items.len() }
                        );
                        std::process::exit(1);
                    }
                }
            }
        })
    };
    unsafe { objc2_foundation::NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.25, true, &block) };
}

static SELF_TEST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static REGRESSION_TEST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static DEBUG_ENTER: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        eprintln!("view: panic: {info}\n{backtrace}");
        std::process::abort();
    }));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let self_test = args.iter().any(|a| a == "--self-test");
    let regression_test = args.iter().any(|a| a == "--regression-test");
    let debug_enter = args.iter().any(|a| a == "--debug-enter");
    let args: Vec<String> = args.into_iter().filter(|a| a != "--self-test" && a != "--debug-enter" && a != "--regression-test").collect();
    SELF_TEST.set(self_test).ok();
    DEBUG_ENTER.set(debug_enter).ok();
    REGRESSION_TEST.set(regression_test).ok();
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

    unsafe {
        extern "C" fn handle_signal(_signal: libc::c_int) {
            std::process::exit(0);
        }
        libc::signal(libc::SIGINT, handle_signal as *const () as usize as libc::sighandler_t);
        libc::signal(libc::SIGTERM, handle_signal as *const () as usize as libc::sighandler_t);
    }

    let mtm = objc2::MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(objc2_app_kit::NSApplicationActivationPolicy::Regular);
    let delegate: Retained<AppDelegate> = unsafe { objc2::msg_send![AppDelegate::class(), new] };
    app.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*delegate)));
    app.run();
}

unsafe impl QLPreviewPanelDataSource for GridView {
}
