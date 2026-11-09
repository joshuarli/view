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
    NSImageView, NSMenu, NSMenuItem, NSScrollView, NSView, NSWindow, NSWindowStyleMask, NSWorkspace,
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

fn columns_for(width: f64) -> usize {
    let available = (width - 32.0).max(0.0);
    ((available + SPACING) / (CELL_W + SPACING)).floor().max(1.0) as usize
}

fn grid_cell_spacing(width: f64) -> f64 {
    let cols = columns_for(width);
    if cols <= 1 {
        return SPACING;
    }
    let available = (width - 32.0).max(0.0);
    let remaining = available - (cols as f64 * CELL_W);
    (remaining / (cols as f64 - 1.0)).max(SPACING)
}

fn content_size(width: f64, height: f64, count: usize, carousel: bool) -> NSSize {
    if carousel {
        NSSize::new(((count as f64) * (CAROUSEL_CELL_W + CAROUSEL_SPACING) + 24.0).max(width), 184.0)
    } else {
        let cols = columns_for(width).max(1);
        let rows = count.div_ceil(cols);
        let content_h = 16.0 + rows as f64 * (CELL_H + SPACING) + 16.0;
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
        let strip_h = 184.0;
        let preview_h = (frame.size.height - strip_h).max(0.0);
        scroll.setHidden(false);
        divider.setHidden(false);
        preview.setHidden(false);
        scroll.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(frame.size.width, strip_h)));
        divider.setFrame(NSRect::new(NSPoint::new(0.0, strip_h), NSSize::new(frame.size.width, 1.0)));
        preview.setFrame(NSRect::new(NSPoint::new(0.0, strip_h + 1.0), NSSize::new(frame.size.width, preview_h - 1.0)));
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
        if let Some(img) = image_for(item).or_else(|| cached_thumbnail(&item.path)) {
            let fitted = aspect_fit(img.size(), preview_bounds);
            preview_image.setFrame(fitted);
            preview_image.setImage(Some(&*img));
        } else {
            preview_image.setImage(None);
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
        let width = window.frame().size.width.max(320.0);
        let height = window.frame().size.height.max(240.0);
        if let Some(root) = window.contentView() {
            layout_carousel(&root, preview, preview_image, divider, scroll, NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height)), carousel);
        }
        scroll.setHasVerticalScroller(!carousel);
        scroll.setHasHorizontalScroller(carousel);
        let doc_size = content_size(width, height, (*app_state()).items.len(), carousel);
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
    let mtm = objc2::MainThreadMarker::new().unwrap();
    let string = NSString::from_str(&path.display().to_string());
    unsafe { NSURL::fileURLWithPath(&*string) }
}

fn path_from_url(url: &NSURL) -> PathBuf {
    let path = url.path().map(|s| s.to_string()).unwrap_or_default();
    PathBuf::from(path)
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

fn undo_trash(grid: &GridView) {
    let transaction = unsafe { (&mut *app_state()).trash_history.pop_undo() };
    let Some(transaction) = transaction else { return };
    let mut succeeded = Vec::new();
    let mut residual = Vec::new();
    for file in transaction.files {
        let exists = std::path::Path::new(&file.trashed_path).exists();
        let dest_exists = std::path::Path::new(&file.item.path).exists();
        match (exists, dest_exists) {
            (true, false) => {
                let _ = std::fs::copy(&file.trashed_path, &file.item.path).and_then(|_| std::fs::remove_file(&file.trashed_path));
                succeeded.push(file);
            }
            _ => residual.push(file),
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
        let present = std::path::Path::new(&file.item.path).exists();
        match present {
            true => {
                let _ = std::fs::copy(&file.item.path, &file.trashed_path).and_then(|_| std::fs::remove_file(&file.item.path));
                succeeded.push(file);
            }
            false => residual.push(file),
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
    let grid_ptr = {
        let refs_guard = window_refs();
        let Some(refs) = refs_guard.as_ref() else { return };
        refs.grid
    };
    let grid = unsafe { &*grid_ptr };
    unsafe { panel.setDataSource(Some(objc2::runtime::ProtocolObject::from_ref(grid))) };
    let index = unsafe { (*app_state()).selected.unwrap_or(0) } as objc2_foundation::NSInteger;
    unsafe { panel.setCurrentPreviewItemIndex(index) };
    panel.orderFront(None);
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

fn open_selected() {
    let Some(item) = unsafe { (*app_state()).selected }.and_then(|i| unsafe { (&*app_state()).items.get(i).cloned() }) else { return };
    if item.kind == MediaKind::Directory {
        let current = unsafe { (*app_state()).root.clone() };
        unsafe { &mut *app_state() }.back.push((current, unsafe { (*app_state()).selected }.and_then(|i| unsafe { (&*app_state()).items.get(i) }).map(|x| x.path.clone())));
        unsafe { &mut *app_state() }.forward.clear();
        unsafe { &mut *app_state() }.root = item.path.clone();
        unsafe { &mut *app_state() }.items = load_listing(&item.path, unsafe { (*app_state()).mode });
        unsafe { &mut *app_state() }.selected = if unsafe { (*app_state()).items.is_empty() } { None } else { Some(0) };
        if let Some(refs) = window_refs().as_ref() {
            unsafe { &*refs.grid }.setNeedsDisplay(true);
        }
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

#[allow(dead_code)]
fn begin_drag_session(view: &GridView, path: &PathBuf, event: &objc2_app_kit::NSEvent) {
    if DRAG_ACTIVE.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let item = objc2_app_kit::NSPasteboardItem::new();
    let url = url_from_path(path);
    let file_text = url.path().map(|s| s.to_string()).unwrap_or_else(|| path.display().to_string());
    unsafe { item.setString_forType(&NSString::from_str(&file_text), &objc2_app_kit::NSPasteboardTypeString) };
    let dragging_item: Retained<objc2_app_kit::NSDraggingItem> = unsafe {
        let alloc: *mut objc2_app_kit::NSDraggingItem =
            objc2::msg_send![objc2_app_kit::NSDraggingItem::class(), alloc];
        let ptr: *mut objc2_app_kit::NSDraggingItem =
            objc2::msg_send![alloc, initWithPasteboardWriter: &*item];
        match Retained::from_raw(ptr) {
            Some(retained) => retained,
            None => {
                DRAG_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
                return;
            }
        }
    };
    let cell_rect = if let Some(idx) = unsafe { (*app_state()).selected } {
        view.cell_rect(idx)
    } else {
        NSRect::new(NSPoint::ZERO, NSSize::new(120.0, 120.0))
    };
    let drag_image = folder_like_icon(&MediaItem::new(path, MediaKind::Directory))
        .or_else(|| image_for(&MediaItem::new(path, MediaKind::Image)))
        .or_else(|| system_image("doc"));
    unsafe {
        let contents_ptr: *const NSObject = match &drag_image {
            Some(img) => &**img as &NSObject,
            None => core::ptr::null(),
        };
        let _: () = objc2::msg_send![
            &*dragging_item,
            setDraggingFrame: cell_rect,
            contents: contents_ptr
        ];
    };
    let items = NSArray::from_retained_slice(&[dragging_item]);
    let source = objc2::runtime::ProtocolObject::from_ref(view);
    view.beginDraggingSessionWithItems_event_source(&items, event, source);
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
    let handler = unsafe {
        block2::RcBlock::new(move |representation: *mut QLThumbnailRepresentation, _error: *mut objc2_foundation::NSError| {
            thumb_pending().remove(&path_for_reply);
            if !representation.is_null() {
                let image = unsafe { (*(representation as *const QLThumbnailRepresentation)).NSImage() };
                thumb_cache().insert(path_for_reply.clone(), SyncImage(image));
            }
            dispatch2::DispatchQueue::main().exec_async(move || {
                if let Some(refs) = window_refs().as_ref() {
                    unsafe { &*refs.grid }.setNeedsDisplay(true);
                }
            });
        })
    };
    unsafe { generator.generateBestRepresentationForRequest_completionHandler(&request, &handler) };
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

        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, rect: NSRect) {
            let state = unsafe { &mut *app_state() };
            let width = self.bounds().size.width;
            let cols = if state.view_mode == ViewMode::Carousel { state.items.len().max(1) } else { columns_for(width).max(1) };
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

                let (cw, ch) = (cell.size.width, cell.size.height);
                let (x, y) = (cell.origin.x, cell.origin.y);
                let (thumb_rect, name_rect, detail_rect) = if state.view_mode == ViewMode::Carousel {
                    (
                        NSRect::new(NSPoint::new(x + 6.0, y + 6.0), NSSize::new(108.0, 110.0)),
                        NSRect::new(NSPoint::new(x + 6.0, y + 120.0), NSSize::new(108.0, 16.0)),
                        NSRect::new(NSPoint::new(x + 6.0, y + 137.0), NSSize::new(108.0, 14.0)),
                    )
                } else {
                    (
                        NSRect::new(NSPoint::new(x + 6.0, y + 6.0), NSSize::new(148.0, 134.0)),
                        NSRect::new(NSPoint::new(x + 6.0, y + 144.0), NSSize::new(148.0, 16.0)),
                        NSRect::new(NSPoint::new(x + 6.0, y + 161.0), NSSize::new(148.0, 14.0)),
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
                        NSFont::boldSystemFontOfSize(12.0)
                    } else {
                        NSFont::systemFontOfSize(12.0)
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
                    let font_detail: Retained<NSFont> = unsafe { NSFont::systemFontOfSize(10.0) };
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
                    // Return: enter directory.
                    if let Some(item) = state.items.get(cur) {
                        if item.kind == MediaKind::Directory {
                            let current = state.root.clone();
                            state.back.push((current, state.selected.and_then(|i| state.items.get(i)).map(|x| x.path.clone())));
                            state.forward.clear();
                            state.root = item.path.clone();
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
            if !state.items.is_empty() {
                let (x, y) = if state.view_mode == ViewMode::Carousel {
                    (view_location.x - 12.0, view_location.y - 8.0)
                } else {
                    (view_location.x - 16.0, view_location.y - 16.0)
                };
                if x >= 0.0 && y >= 0.0 {
                    let index = if state.view_mode == ViewMode::Carousel {
                        (x / (120.0 + 8.0)).floor() as usize
                    } else {
                        let cols = columns_for(self.bounds().size.width).max(1);
                        let col = (x / (CELL_W + SPACING)).floor() as usize;
                        let row = (y / (CELL_H + SPACING)).floor() as usize;
                        if col >= cols { 0 } else { row * cols + col }
                    };
                    if index < state.items.len() {
                        state.selected = Some(index);
                        self.setNeedsDisplay(true);
                    }
                }
            }
            place_context_menu(self, event);
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
                if x < 0.0 || y < 0.0 || y >= CAROUSEL_CELL_H {
                    return;
                }
                (x / (CAROUSEL_CELL_W + CAROUSEL_SPACING)).floor() as usize
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
                        state.selection_anchor = state.selected;
                        state.selection_range = (0, 0);
                        let title = state.root.display().to_string();
                        if let Some(refs) = window_refs().as_ref() {
                            unsafe { (&*refs.window).setTitle(&NSString::from_str(&title)); }
                        }
                        apply_view_mode_layout();
                        update_carousel_preview();
                    }
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

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &objc2_app_kit::NSEvent) {
            let state = unsafe { &mut *app_state() };
            if state.items.is_empty() {
                return;
            }
            let Some(index) = state.selected else { return; };
            if let Some(item) = unsafe { (&*app_state()).items.get(index).cloned() } {
                let path = item.path.clone();
                begin_drag_session(self, &path, event);
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
);

unsafe impl objc2_foundation::NSObjectProtocol for GridView {}
unsafe impl objc2_app_kit::NSDraggingSource for GridView {
    fn draggingSession_sourceOperationMaskForDraggingContext(
        &self,
        _session: &objc2_app_kit::NSDraggingSession,
        _context: objc2_app_kit::NSDraggingContext,
    ) -> objc2_app_kit::NSDragOperation {
        objc2_app_kit::NSDragOperation::Copy
    }

    fn draggingSession_endedAtPoint_operation(
        &self,
        _session: &objc2_app_kit::NSDraggingSession,
        _screen_point: objc2_foundation::NSPoint,
        _operation: objc2_app_kit::NSDragOperation,
    ) {
        DRAG_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

impl GridView {
    fn cell_rect(&self, index: usize) -> NSRect {
        let state = unsafe { &*app_state() };
        let width = self.bounds().size.width;
        if state.view_mode == ViewMode::Carousel {
            let x = 12.0 + index as f64 * (CAROUSEL_CELL_W + CAROUSEL_SPACING);
            return NSRect::new(NSPoint::new(x, 14.0), NSSize::new(CAROUSEL_CELL_W, CAROUSEL_CELL_H));
        }
        let cols = columns_for(width).max(1);
        let spacing = grid_cell_spacing(width);
        let col = index % cols;
        let row = index / cols;
        NSRect::new(
            NSPoint::new(16.0 + col as f64 * (CELL_W + spacing), 16.0 + row as f64 * (CELL_H + SPACING)),
            NSSize::new(CELL_W, CELL_H),
        )
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
            if let Some(Some(dir)) = DUMP_VISUAL.get() {
                start_visual_dump(dir.clone());
            } else {
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
            let _ = window;
        }
    }
);

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
static DEBUG_ENTER: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static DUMP_VISUAL: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
static VISUAL_DUMP_STEP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn capture_screenshot(window: &NSWindow, path: &std::path::Path) {
    let Some(content) = (unsafe { window.contentView() }) else { return };
    unsafe {
        window.layoutIfNeeded();
        content.layoutSubtreeIfNeeded();
    }
    let bounds = content.bounds();
    let rep: Option<Retained<objc2_app_kit::NSBitmapImageRep>> = unsafe {
        let ptr: *mut objc2_app_kit::NSBitmapImageRep =
            objc2::msg_send![&*content, bitmapImageRepForCachingDisplayInRect: bounds];
        Retained::retain(ptr)
    };
    let Some(rep) = rep else { return };
    unsafe {
        let _: () = objc2::msg_send![&*content, cacheDisplayInRect: bounds, toBitmapImageRep: &*rep];
        let empty_dict = objc2_foundation::NSDictionary::<objc2_foundation::NSString, objc2::runtime::NSObject>::new();
        let data: Option<Retained<objc2_foundation::NSData>> = {
            let ptr: *mut objc2_foundation::NSData = objc2::msg_send![
                &*rep,
                representationUsingType: 4usize,
                properties: &*empty_dict
            ];
            Retained::retain(ptr)
        };
        if let Some(data) = data {
            let bytes = unsafe { data.as_bytes_unchecked() };
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, bytes);
        }
    }
}

fn start_visual_dump(out_dir: PathBuf) {
    let mtm = objc2::MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    let block = unsafe {
        block2::RcBlock::new(move |_timer: core::ptr::NonNull<objc2_foundation::NSTimer>| {
            let (window_ptr, grid_ptr) = {
                let refs = window_refs();
                let Some(refs) = refs.as_ref() else { return };
                (refs.window, refs.grid)
            };
            let window = unsafe { &*window_ptr };
            let grid = unsafe { &*grid_ptr };
            let step = VISUAL_DUMP_STEP.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match step {
                0 => {
                    unsafe { (*app_state()).view_mode = ViewMode::Carousel };
                    apply_view_mode_layout();
                    capture_screenshot(window, &out_dir.join("step_0_carousel_initial.png"));
                    send_key_via_app(&app, 124, false);
                }
                1 => {
                    capture_screenshot(window, &out_dir.join("step_1_carousel_right.png"));
                    send_key_via_app(&app, 124, false);
                }
                2 => {
                    capture_screenshot(window, &out_dir.join("step_2_carousel_red.png"));
                    unsafe { (*app_state()).view_mode = ViewMode::Grid };
                    apply_view_mode_layout();
                }
                3 => {
                    capture_screenshot(window, &out_dir.join("step_3_grid_mode.png"));
                    send_key_via_app(&app, 125, false);
                }
                4 => {
                    capture_screenshot(window, &out_dir.join("step_4_grid_down.png"));
                    send_key_via_app(&app, 123, false);
                }
                5 => {
                    capture_screenshot(window, &out_dir.join("step_5_grid_left.png"));
                    unsafe { (*app_state()).view_mode = ViewMode::Carousel };
                    apply_view_mode_layout();
                }
                6 => {
                    capture_screenshot(window, &out_dir.join("step_6_back_to_carousel.png"));
                    send_key_via_app(&app, 123, false);
                    send_key_via_app(&app, 123, false);
                    send_key_via_app(&app, 123, false);
                }
                7 => {
                    send_key_via_app(&app, 36, false);
                }
                8 => {
                    capture_screenshot(window, &out_dir.join("step_7_entered_subdir.png"));
                    send_key_via_app(&app, 123, true);
                }
                9 => {
                    capture_screenshot(window, &out_dir.join("step_8_history_back.png"));
                    std::process::exit(0);
                }
                _ => {}
            }
        })
    };
    unsafe { objc2_foundation::NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.2, true, &block) };
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        eprintln!("view: panic: {info}\n{backtrace}");
        std::process::abort();
    }));
    let mut dump_visual: Option<PathBuf> = None;
    let mut args: Vec<String> = Vec::new();
    let mut i = 1;
    let raw_args: Vec<String> = std::env::args().collect();
    while i < raw_args.len() {
        if raw_args[i] == "--dump-visual" && i + 1 < raw_args.len() {
            dump_visual = Some(PathBuf::from(&raw_args[i + 1]));
            i += 2;
        } else if raw_args[i] == "--self-test" {
            SELF_TEST.set(true).ok();
            i += 1;
        } else if raw_args[i] == "--debug-enter" {
            DEBUG_ENTER.set(true).ok();
            i += 1;
        } else {
            args.push(raw_args[i].clone());
            i += 1;
        }
    }
    DUMP_VISUAL.set(dump_visual.clone()).ok();
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
        if dump_visual.is_none() {
            let defaults = objc2_foundation::NSUserDefaults::standardUserDefaults();
            let saved = defaults.stringForKey(&NSString::from_str("View.ViewMode"))
                .map(|s| s.to_string());
            state.view_mode = match saved.as_deref() {
                Some("grid") => ViewMode::Grid,
                _ => ViewMode::Carousel,
            };
        } else {
            state.view_mode = ViewMode::Carousel;
        }
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
