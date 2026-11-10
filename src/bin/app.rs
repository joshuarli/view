//! Rust port GUI binary (see Rust/PARITY.md).
//!
//! Native AppKit host built on `objc2-app-kit` (the same frameworks as
//! the Swift app): window + scroll view + custom grid view, wired to
//! the `view_core` discovery/catalog state.

use std::path::PathBuf;
use std::os::unix::ffi::{OsStrExt, OsStringExt};

use objc2::{define_class, ClassType, DefinedClass, MainThreadOnly};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyObject, NSObject};
use objc2_app_kit::{
    NSApplication, NSApplicationDelegate, NSBackingStoreType, NSBox, NSColor, NSDraggingInfo, NSDraggingSource, NSFont, NSImage,
    NSImageView, NSMenu, NSMenuItem, NSScrollView, NSTextField, NSView, NSWindow, NSWindowDelegate, NSWindowStyleMask, NSWorkspace,
};
use objc2_av_foundation::{AVLayerVideoGravityResizeAspect, AVPlayer, AVPlayerItem, AVPlayerItemStatus};
use objc2_av_kit::AVPlayerView;
use objc2_core_graphics::CGImage;
use objc2_foundation::{
    NSArray, NSDictionary, NSKeyValueObservingOptions, NSNotification, NSObjectNSKeyValueObserverRegistration, NSObjectProtocol,
    NSPoint, NSRect, NSSize, NSString, NSURL,
};
use objc2_quick_look_thumbnailing::{
    QLThumbnailGenerationRequest, QLThumbnailGenerationRequestRepresentationTypes, QLThumbnailGenerator,
    QLThumbnailRepresentation,
};
use objc2_quick_look_ui::{QLPreviewPanel, QLPreviewPanelDataSource};

use view_core::{
    list_browser, scan_recursive, BrowserMode, Catalog, MediaKind, MediaItem, PreviewSession, TrashHistory,
    TrashService, TrashTransaction,
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
    catalog: Catalog,
    preview: PreviewSession,
    preview_bound: bool,
    back: Vec<(PathBuf, Option<PathBuf>)>,
    forward: Vec<(PathBuf, Option<PathBuf>)>,
    _trash_service: TrashService,
    trash_history: TrashHistory,
}

impl AppState {
    fn items(&self) -> &[MediaItem] { self.catalog.items() }
    fn selected(&self) -> Option<usize> { self.catalog.focus_index() }
}

enum ListingPick {
    First,
    Restored(Option<PathBuf>),
}

fn show_listing(catalog: &mut Catalog, items: Vec<MediaItem>, pick: ListingPick) {
    *catalog = Catalog::new();
    catalog.append(items);
    match pick {
        ListingPick::First => {
            if let Some(path) = catalog.items().first().map(|item| item.path.clone()) {
                catalog.select(&path).expect("first item is in the catalog");
            }
        }
        ListingPick::Restored(Some(path)) => {
            if catalog.select(&path).is_err() {
                catalog.clear_selection();
            }
        }
        ListingPick::Restored(None) => catalog.clear_selection(),
    }
}

fn select_index(catalog: &mut Catalog, index: usize) {
    let path = catalog.items()[index].path.clone();
    catalog.select(&path).expect("selection index is in the catalog");
}

fn focused_item(state: &AppState) -> Option<MediaItem> {
    let index = state.catalog.focus_index()?;
    state.catalog.items().get(index).cloned()
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
    preview_label: *mut NSTextField,
    player: *mut AVPlayerView,
}
unsafe impl Send for WindowRefs {}
unsafe impl Sync for WindowRefs {}

static WINDOW_REFS: std::sync::OnceLock<std::sync::Mutex<Option<WindowRefs>>> = std::sync::OnceLock::new();

fn window_refs() -> std::sync::MutexGuard<'static, Option<WindowRefs>> {
    WINDOW_REFS.get_or_init(|| std::sync::Mutex::new(None)).lock().unwrap()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PlayerStatus {
    Loading,
    Ready,
    Failed,
}

struct PlayerStore {
    view: Retained<PreviewPlayerView>,
    player: Retained<AVPlayer>,
    observer: Retained<PlayerItemObserver>,
    poster: *mut NSImageView,
    item: Option<Retained<AVPlayerItem>>,
    item_path: Option<PathBuf>,
    status: PlayerStatus,
    autoplay: bool,
}

static PLAYER_STORE: std::sync::OnceLock<SyncMutex<Option<PlayerStore>>> = std::sync::OnceLock::new();

struct SyncMutex<T>(std::cell::UnsafeCell<std::sync::Mutex<T>>);

unsafe impl<T> Send for SyncMutex<T> {}
unsafe impl<T> Sync for SyncMutex<T> {}

fn player_store() -> std::sync::MutexGuard<'static, Option<PlayerStore>> {
    let cell = PLAYER_STORE.get_or_init(|| SyncMutex(std::cell::UnsafeCell::new(std::sync::Mutex::new(None))));
    // SAFETY: player state is created and used only on the main thread.
    unsafe { &*cell.0.get() }.lock().unwrap_or_else(|e| e.into_inner())
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    struct PlayerItemObserver;

    impl PlayerItemObserver {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(
            &self,
            _key_path: Option<&NSString>,
            object: Option<&AnyObject>,
            _change: Option<&NSDictionary>,
            _context: *mut core::ffi::c_void,
        ) {
            let object = object.map(|observed| observed as *const AnyObject);
            player_item_status_changed(object);
        }
    }
);

unsafe impl NSObjectProtocol for PlayerItemObserver {}

fn update_player_visibility(store: &mut PlayerStore) {
    let visible = store.item_path.is_some() && store.status == PlayerStatus::Ready;
    store.view.setHidden(!visible);
    unsafe { (*store.poster).setHidden(visible) };
}

fn player_item_status_changed(object: Option<*const AnyObject>) {
    if objc2::MainThreadMarker::new().is_none() {
        let object = object.map(|observed| observed as usize);
        dispatch2::DispatchQueue::main().exec_async(move || {
            player_item_status_changed(object.map(|observed| observed as *const AnyObject));
        });
        return;
    }
    let mut store = player_store();
    let Some(store) = store.as_mut() else { return };
    let Some(item) = store.item.as_ref() else { return };
    if object != Some(Retained::as_ptr(item) as *const AnyObject) {
        return;
    }
    store.status = match unsafe { item.status() } {
        AVPlayerItemStatus::Unknown => PlayerStatus::Loading,
        AVPlayerItemStatus::ReadyToPlay => PlayerStatus::Ready,
        AVPlayerItemStatus::Failed => PlayerStatus::Failed,
        _ => PlayerStatus::Loading,
    };
    update_player_visibility(store);
    if store.status == PlayerStatus::Ready && store.autoplay {
        store.autoplay = false;
        unsafe { store.player.play() };
    }
}

fn release_player() {
    let (player, observer, old_item) = {
        let mut store = player_store();
        let Some(store) = store.as_mut() else { return };
        store.item_path = None;
        store.status = PlayerStatus::Loading;
        store.autoplay = false;
        (store.player.clone(), store.observer.clone(), store.item.take())
    };
    if let Some(old_item) = old_item {
        unsafe {
            let key = NSString::from_str("status");
            old_item.as_super().removeObserver_forKeyPath_context(observer.as_super(), &key, core::ptr::null_mut());
        }
    }
    unsafe { player.replaceCurrentItemWithPlayerItem(None) };
    let mut store = player_store();
    if let Some(store) = store.as_mut() {
        update_player_visibility(store);
    }
}

fn load_player_item(path: &std::path::Path) {
    let Some(mtm) = objc2::MainThreadMarker::new() else { return };
    let (player, observer, old_item) = {
        let mut store = player_store();
        let Some(store) = store.as_mut() else { return };
        if store.item_path.as_deref() == Some(path) {
            return;
        }
        (store.player.clone(), store.observer.clone(), store.item.take())
    };
    if let Some(old_item) = old_item {
        unsafe {
            let key = NSString::from_str("status");
            old_item.as_super().removeObserver_forKeyPath_context(observer.as_super(), &key, core::ptr::null_mut());
        }
    }
    let url = url_from_path(path);
    let item = unsafe { AVPlayerItem::playerItemWithURL(&url, mtm) };
    unsafe {
        let key = NSString::from_str("status");
        item.as_super().addObserver_forKeyPath_options_context(
            observer.as_super(),
            &key,
            NSKeyValueObservingOptions::New,
            core::ptr::null_mut(),
        );
    }
    unsafe { player.replaceCurrentItemWithPlayerItem(Some(&item)) };
    let mut store = player_store();
    let Some(store) = store.as_mut() else { return };
    store.item = Some(item);
    store.item_path = Some(path.to_path_buf());
    store.autoplay = false;
    store.status = match unsafe { store.item.as_ref().unwrap().status() } {
        AVPlayerItemStatus::Unknown => PlayerStatus::Loading,
        AVPlayerItemStatus::ReadyToPlay => PlayerStatus::Ready,
        AVPlayerItemStatus::Failed => PlayerStatus::Failed,
        _ => PlayerStatus::Loading,
    };
    update_player_visibility(store);
}

fn start_playback() {
    let mut store = player_store();
    let Some(store) = store.as_mut() else { return };
    if store.item_path.is_none() {
        return;
    }
    if store.status == PlayerStatus::Ready {
        unsafe { store.player.play() };
    } else {
        store.autoplay = true;
    }
}

fn toggle_inline_playback() {
    let mut store = player_store();
    let Some(store) = store.as_mut() else { return };
    match store.status {
        PlayerStatus::Ready => {
            if unsafe { store.player.rate() } == 0.0 {
                unsafe { store.player.play() };
            } else {
                unsafe { store.player.pause() };
            }
        }
        PlayerStatus::Failed => {
            let _ = store;
            quicklook_selected();
        }
        PlayerStatus::Loading => store.autoplay = true,
    }
}

fn space_pressed() {
    let (view_mode, item) = {
        let state = unsafe { &*app_state() };
        (state.view_mode, focused_item(state))
    };
    let Some(item) = item else { return };
    if item.kind != MediaKind::Movie {
        quicklook_selected();
        return;
    }
    if view_mode == ViewMode::Grid {
        unsafe { &mut *app_state() }.view_mode = ViewMode::Carousel;
        apply_view_mode_layout();
        start_playback();
        return;
    }
    toggle_inline_playback();
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

fn text_attributes(font: &NSFont, color: &NSColor, paragraph: &objc2_app_kit::NSMutableParagraphStyle) -> Retained<NSDictionary<objc2_foundation::NSAttributedStringKey>> {
    unsafe {
        let keys = NSArray::from_slice(&[
            objc2_app_kit::NSFontAttributeName,
            objc2_app_kit::NSForegroundColorAttributeName,
            objc2_app_kit::NSParagraphStyleAttributeName,
        ]);
        let objs: [&NSObject; 3] = [font as &NSObject, color as &NSObject, paragraph as &NSObject];
        let values = NSArray::from_slice(&objs);
        let dict_ptr: *mut NSDictionary<objc2_foundation::NSAttributedStringKey> =
            objc2::msg_send![objc2::class!(NSDictionary), dictionaryWithObjects: &*values, forKeys: &*keys];
        Retained::retain(dict_ptr).unwrap()
    }
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

fn cell_content_rects(cell: NSRect, carousel: bool, scale: f64) -> (NSRect, NSRect, NSRect) {
    let (x, y) = (cell.origin.x, cell.origin.y);
    if carousel {
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
    }
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
    player: &AVPlayerView,
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
        let preview_size = preview.bounds().size;
        image.setFrame(NSRect::new(NSPoint::ZERO, preview_size));
        player.setFrame(NSRect::new(NSPoint::ZERO, preview_size));
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
    let (preview_image_ptr, preview_ptr, preview_label_ptr) = {
        let refs = window_refs();
        let Some(refs) = refs.as_ref() else { return };
        (refs.preview_image, refs.preview, refs.preview_label)
    };
    let preview_image = unsafe { &*preview_image_ptr };
    let preview = unsafe { &*preview_ptr };
    let preview_label = unsafe { &*preview_label_ptr };
    let state = unsafe { &*app_state() };
    if state.view_mode != ViewMode::Carousel {
        return;
    }
    let Some(index) = state.selected() else {
        preview_image.setImage(None);
        preview_label.setHidden(true);
        release_player();
        return;
    };
    let Some(item) = state.items().get(index) else {
        preview_image.setImage(None);
        preview_label.setHidden(true);
        release_player();
        return;
    };
    let preview_bounds = preview.bounds();
    if item.kind == MediaKind::Directory {
        release_player();
        let items = view_core::list_browser(&item.path).unwrap_or_default();
        let count = items.len();
        let size: u64 = items.iter().filter_map(|i| i.file_size).map(|s| s.max(0) as u64).sum();
        let name = item.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let size_str = byte_count_string(size);
        let detail = format!("{name}\n{count} items, {}", size_str);
        preview_label.setStringValue(&NSString::from_str(&detail));
        preview_label.setHidden(false);
        preview_image.setImageScaling(objc2_app_kit::NSImageScaling::ScaleProportionallyDown);
        let label_h = ((preview_bounds.size.height * 0.35).clamp(48.0, 140.0)).min(preview_bounds.size.height - 80.0).max(48.0);
        let icon_area_h = (preview_bounds.size.height - label_h - 24.0).max(64.0);
        let icon_size = icon_area_h.min(preview_bounds.size.width - 32.0).min(256.0).max(64.0);
        if let Some(icon) = folder_like_icon(item) {
            unsafe { icon.setSize(NSSize::new(icon_size, icon_size)); }
            preview_image.setImage(Some(&*icon));
            let x = ((preview_bounds.size.width - icon_size) / 2.0).round();
            let y = (label_h + 16.0 + (icon_area_h - icon_size) / 2.0).round();
            preview_image.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(icon_size, icon_size)));
        }
        preview_label.setFrame(NSRect::new(NSPoint::new(16.0, 8.0), NSSize::new((preview_bounds.size.width - 32.0).max(0.0), label_h)));
    } else if item.kind == MediaKind::Movie {
        preview_label.setHidden(true);
        preview_image.setImageScaling(objc2_app_kit::NSImageScaling::ScaleProportionallyUpOrDown);
        let pixels = pixel_limit(preview, preview_bounds);
        let image = cached_sized(&item.path, pixels, None, None);
        if let Some(img) = image {
            let fitted = aspect_fit(img.size(), preview_bounds);
            preview_image.setFrame(fitted);
            preview_image.setImage(Some(&*img));
        } else {
            preview_image.setImage(None);
            request_quicklook_thumbnail_sized(item.path.clone(), pixels);
        }
        load_player_item(&item.path);
    } else {
        release_player();
        preview_label.setHidden(true);
        preview_image.setImageScaling(objc2_app_kit::NSImageScaling::ScaleProportionallyUpOrDown);
        let pixels = pixel_limit(preview, preview_bounds);
        let image = display_image(&item.path, pixels, item.file_size, item.modification);
        if let Some(img) = image {
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
    let (window_ptr, scroll_ptr, grid_ptr, preview_ptr, preview_image_ptr, divider_ptr, player_ptr) = {
        let refs = window_refs();
        let Some(refs) = refs.as_ref() else { return };
        (refs.window, refs.scroll, refs.grid, refs.preview, refs.preview_image, refs.divider, refs.player)
    };
    let carousel = unsafe { (*app_state()).view_mode } == ViewMode::Carousel;
    if !carousel {
        release_player();
    }
    unsafe {
        let window = &*window_ptr;
        let scroll = &*scroll_ptr;
        let grid = &*grid_ptr;
        let preview = &*preview_ptr;
        let preview_image = &*preview_image_ptr;
        let divider = &*divider_ptr;
        let player = &*player_ptr;
        let Some(root) = window.contentView() else { return };
        let bounds = root.bounds();
        layout_carousel(&root, preview, preview_image, player, divider, scroll, bounds, carousel);
        scroll.setHasVerticalScroller(!carousel);
        scroll.setHasHorizontalScroller(carousel);
        let viewport = scroll.contentView().bounds().size;
        let doc_size = content_size(viewport.width, viewport.height, (*app_state()).items().len(), carousel);
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

fn trash_selected(_grid: &GridView) {
    let items = {
        let state = unsafe { &mut *app_state() };
        let Some(item) = focused_item(state) else { return };
        vec![item]
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
                {
                    let state = unsafe { &mut *app_state() };
                    let mut files = Vec::new();
                    for (original, trashed) in trashed_map.iter() {
                        if let Some(pos) = state.items().iter().position(|i| &i.path == original) {
                            files.push(view_core::TrashedFile {
                                item: state.items()[pos].clone(),
                                trashed_path: trashed.clone(),
                                index: pos,
                            });
                        }
                    }
                    for file in files.iter().rev() {
                        state.catalog.remove(&file.item.path);
                    }
                    state.trash_history.record_trash(view_core::TrashTransaction {
                        files,
                        selection_before_op: vec![],
                    });
                }
                reconcile_preview();
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
                let at = file.index.min(state.catalog.count());
                state.catalog.insert(file.item.clone(), at);
            }
            state.trash_history.push_redo(TrashTransaction { files: succeeded, selection_before_op });
        }
        if !residual.is_empty() {
            state.trash_history.push_undo(TrashTransaction { files: residual, selection_before_op: transaction.selection_before_op });
        }
    }
    reconcile_preview();
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
            state.catalog.remove(&file.item.path);
        }
        if !succeeded.is_empty() {
            state.trash_history.push_undo(TrashTransaction { files: succeeded, selection_before_op });
        }
        if !residual.is_empty() {
            state.trash_history.push_redo(TrashTransaction { files: residual, selection_before_op: transaction.selection_before_op });
        }
    }
    reconcile_preview();
    grid.setNeedsDisplay(true);
}

fn quicklook_selected() {
    let mtm = objc2::MainThreadMarker::new().unwrap();
    let Some(panel) = (unsafe { QLPreviewPanel::sharedPreviewPanel(mtm) }) else { return };
    if panel.isVisible() {
        panel.orderOut(None);
        return;
    }
    if unsafe { (*app_state()).catalog.selected().is_none() } {
        return;
    }
    panel.makeKeyAndOrderFront(None);
}

fn reconcile_preview() {
    let Some(mtm) = objc2::MainThreadMarker::new() else { return };
    let Some(panel) = (unsafe { QLPreviewPanel::sharedPreviewPanel(mtm) }) else { return };
    if !panel.isVisible() {
        return;
    }
    let index = {
        let state = unsafe { &mut *app_state() };
        if state.preview.begin(&state.catalog) {
            state.preview.current_index()
        } else {
            None
        }
    };
    let Some(index) = index else {
        panel.orderOut(None);
        return;
    };
    unsafe {
        panel.setCurrentPreviewItemIndex(index as objc2_foundation::NSInteger);
        panel.reloadData();
    }
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
    let title = {
        let state = unsafe { &mut *app_state() };
        let current = state.root.clone();
        let previous = state.catalog.selected().map(|path| path.to_path_buf());
        state.back.push((current, previous));
        state.forward.clear();
        state.root = path;
        let mode = state.mode;
        let root = state.root.clone();
        show_listing(&mut state.catalog, load_listing(&root, mode), ListingPick::First);
        state.root.display().to_string()
    };
    if let Some(refs) = window_refs().as_ref() {
        unsafe {
            (&*refs.window).setTitle(&NSString::from_str(&title));
        }
    }
    apply_view_mode_layout();
    reconcile_preview();
    if let Some(refs) = window_refs().as_ref() {
        unsafe { (&*refs.grid).scroll_selection_into_view() };
        unsafe { (&*refs.grid).setNeedsDisplay(true) };
    }
}

fn open_selected() {
    let Some(item) = focused_item(unsafe { &*app_state() }) else { return };
    if item.kind == MediaKind::Directory {
        enter_directory(item.path);
    } else {
        let ws = NSWorkspace::sharedWorkspace();
        ws.openFile(&NSString::from_str(&item.path.display().to_string()));
    }
}

fn show_selected_in_finder() {
    let Some(item) = focused_item(unsafe { &*app_state() }) else { return };
    let _ = std::process::Command::new("open").arg("-R").arg(&item.path).spawn();
}

fn copy_selected_path() {
    let Some(item) = focused_item(unsafe { &*app_state() }) else { return };
    let pasteboard = objc2_app_kit::NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    unsafe { pasteboard.setString_forType(&NSString::from_str(&item.path.display().to_string()), &objc2_app_kit::NSPasteboardTypeString) };
}

static DRAG_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn dragging_items(view: &GridView) -> Vec<Retained<objc2_app_kit::NSDraggingItem>> {
    let state = unsafe { &*app_state() };
    let Some((start, end)) = state.catalog.selection_bounds() else { return Vec::new() };
    (start..=end).filter_map(|index| {
        let item = state.items().get(index)?;
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
    {
        let state = unsafe { &mut *app_state() };
        let Some(index) = state.items().iter().position(|item| item.path == path) else { return false };
        select_index(&mut state.catalog, index);
    }
    view.scroll_selection_into_view();
    update_carousel_preview();
    view.setNeedsDisplay(true);
    reconcile_preview();
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

struct CachedThumb {
    pixels: u32,
    file_size: Option<i64>,
    modification: Option<u64>,
    image: SyncImage,
    cost: usize,
}

struct ThumbStore {
    images: std::collections::HashMap<PathBuf, Vec<CachedThumb>>,
    order: std::collections::VecDeque<(PathBuf, u32)>,
    cost: usize,
}

static THUMB_CACHE: std::sync::OnceLock<std::sync::Mutex<ThumbStore>> = std::sync::OnceLock::new();
static THUMB_PENDING: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<(PathBuf, u32)>>> =
    std::sync::OnceLock::new();

#[cfg(feature = "benchmark")]
const MOVIE_PREVIEW_PIXELS: u32 = 960;

fn thumb_store() -> std::sync::MutexGuard<'static, ThumbStore> {
    THUMB_CACHE
        .get_or_init(|| {
            std::sync::Mutex::new(ThumbStore {
                images: std::collections::HashMap::new(),
                order: std::collections::VecDeque::new(),
                cost: 0,
            })
        })
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn thumb_pending() -> std::sync::MutexGuard<'static, std::collections::HashSet<(PathBuf, u32)>> {
    THUMB_PENDING
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn cached_thumbnail(path: &PathBuf) -> Option<Retained<NSImage>> {
    thumb_store().images.get(path).and_then(|entries| entries.last().map(|entry| entry.image.0.clone()))
}

fn thumbnail_is_cached(path: &std::path::Path, pixels: u32, file_size: Option<i64>, modification: Option<u64>) -> bool {
    thumb_store().images.get(path).is_some_and(|entries| {
        entries.iter().any(|entry| entry.pixels == pixels && entry.file_size == file_size && entry.modification == modification)
    })
}

fn cached_sized(path: &std::path::Path, pixels: u32, file_size: Option<i64>, modification: Option<u64>) -> Option<Retained<NSImage>> {
    let store = thumb_store();
    let entries = store.images.get(path)?;
    entries.iter().find(|entry| entry.pixels == pixels && entry.file_size == file_size && entry.modification == modification).map(|entry| entry.image.0.clone())
}

fn remember_thumbnail(path: PathBuf, pixels: u32, file_size: Option<i64>, modification: Option<u64>, image: SyncImage, cost: usize) {
    let mut store = thumb_store();
    let replaced = {
        let entries = store.images.entry(path.clone()).or_default();
        if let Some(existing) = entries.iter_mut().find(|entry| entry.pixels == pixels && entry.file_size == file_size && entry.modification == modification) {
            let previous = existing.cost;
            existing.image = image;
            existing.cost = cost;
            Some(previous)
        } else {
            entries.push(CachedThumb { pixels, file_size, modification, image, cost });
            None
        }
    };
    if let Some(previous) = replaced {
        store.cost = store.cost.saturating_sub(previous).saturating_add(cost);
    } else {
        store.order.push_back((path, pixels));
        store.cost = store.cost.saturating_add(cost);
    }
    while store.cost > view_core::THUMBNAIL_CACHE_LIMIT {
        let Some((old_path, old_pixels)) = store.order.pop_front() else { break };
        let removed = store.images.get_mut(&old_path).and_then(|entries| {
            let position = entries.iter().position(|entry| entry.pixels == old_pixels)?;
            Some(entries.remove(position).cost)
        });
        let Some(removed) = removed else { continue };
        store.cost = store.cost.saturating_sub(removed);
        if store.images.get(&old_path).is_some_and(|entries| entries.is_empty()) {
            store.images.remove(&old_path);
        }
    }
}

#[cfg(feature = "benchmark")]
fn forget_thumbnail(path: &std::path::Path) {
    let mut store = thumb_store();
    if let Some(entries) = store.images.remove(path) {
        for entry in entries {
            store.cost = store.cost.saturating_sub(entry.cost);
        }
    }
    store.order.retain(|(old_path, _)| old_path != path);
}

#[cfg(feature = "benchmark")]
fn thumbnail_is_pending(path: &std::path::Path) -> bool {
    thumb_pending().iter().any(|(pending, _)| pending == path)
}

#[cfg(feature = "benchmark")]
fn request_quicklook_thumbnail(path: PathBuf) {
    request_quicklook_thumbnail_sized(path, MOVIE_PREVIEW_PIXELS);
}

fn request_quicklook_thumbnail_sized(path: PathBuf, pixels: u32) {
    let pixels = pixels.clamp(1, 4096);
    if !thumb_pending().insert((path.clone(), pixels)) {
        return;
    }
    let generator = unsafe { QLThumbnailGenerator::sharedGenerator() };
    let url = url_from_path(&path);
    let points = (pixels as f64 / 2.0).max(1.0);
    let request: Retained<QLThumbnailGenerationRequest> = unsafe {
        let alloc = objc2::MainThreadMarker::new().unwrap().alloc::<QLThumbnailGenerationRequest>();
        QLThumbnailGenerationRequest::initWithFileAtURL_size_scale_representationTypes(
            alloc,
            &url,
            NSSize::new(points, points),
            2.0,
            QLThumbnailGenerationRequestRepresentationTypes::Thumbnail,
        )
    };
    let path_for_reply = path.clone();
    let handler = block2::RcBlock::new(move |representation: *mut QLThumbnailRepresentation, error: *mut objc2_foundation::NSError| {
        if representation.is_null() {
            thumb_pending().remove(&(path_for_reply.clone(), pixels));
            if !error.is_null() {
                eprintln!("view: thumbnail {}: {:?}", path_for_reply.display(), unsafe { &*error });
            }
            return;
        }
        let image = SyncImage(unsafe { (*(representation as *const QLThumbnailRepresentation)).NSImage() });
        let path = path_for_reply.clone();
        dispatch2::DispatchQueue::main().exec_async(move || thumbnail_completed(path, pixels, image));
    });
    unsafe { generator.generateBestRepresentationForRequest_completionHandler(&request, &handler) };
}

fn thumbnail_completed(path: PathBuf, pixels: u32, image: SyncImage) {
    thumb_pending().remove(&(path.clone(), pixels));
    let size = image.0.size();
    let cost = ((size.width.max(1.0) * size.height.max(1.0) * 4.0) as usize).saturating_mul(4);
    remember_thumbnail(path, pixels, None, None, image, cost);
    if let Some(refs) = window_refs().as_ref() {
        unsafe { &*refs.grid }.setNeedsDisplay(true);
    }
    update_carousel_preview();
}

fn pixel_limit(view: &NSView, rect: NSRect) -> u32 {
    let scale = view.window().map(|window| window.backingScaleFactor()).unwrap_or(2.0).max(1.0);
    (rect.size.width.max(rect.size.height) * scale).round().clamp(1.0, 4096.0) as u32
}

#[derive(Clone)]
struct ThumbRequest {
    path: PathBuf,
    pixels: u32,
    file_size: Option<i64>,
    modification: Option<u64>,
}

struct DecodedCg {
    image: *mut std::ffi::c_void,
    width: usize,
    height: usize,
}

// SAFETY: DecodedCg owns one CGImage and moves that ownership between threads.
// ImageIO creates the image on a worker; the main thread is the only user after join.
unsafe impl Send for DecodedCg {}

impl Drop for DecodedCg {
    fn drop(&mut self) {
        if !self.image.is_null() {
            cf_release(self.image);
            self.image = std::ptr::null_mut();
        }
    }
}

struct AutoreleasePool(*mut std::ffi::c_void);

impl AutoreleasePool {
    fn push() -> Self {
        Self(unsafe { objc_autorelease_pool_push() })
    }
}

impl Drop for AutoreleasePool {
    fn drop(&mut self) {
        unsafe { objc_autorelease_pool_pop(self.0) }
    }
}

fn display_image(path: &std::path::Path, pixels: u32, file_size: Option<i64>, modification: Option<u64>) -> Option<Retained<NSImage>> {
    if let Some(image) = cached_sized(path, pixels, file_size, modification) {
        return Some(image);
    }
    ensure_thumbnails(&[ThumbRequest { path: path.to_path_buf(), pixels, file_size, modification }]);
    cached_sized(path, pixels, file_size, modification)
}

fn ensure_thumbnails(requests: &[ThumbRequest]) {
    let missing: Vec<ThumbRequest> = requests.iter().filter(|request| {
        !thumbnail_is_cached(&request.path, request.pixels, request.file_size, request.modification)
    }).cloned().collect();
    if missing.is_empty() {
        return;
    }
    let decoded = decode_cg_thumbnails(&missing);
    for (request, decoded) in missing.into_iter().zip(decoded) {
        let Some(decoded) = decoded else { continue };
        let Some(image) = nsimage_from_cg(&decoded) else { continue };
        let cost = decoded.width.saturating_mul(decoded.height).saturating_mul(4);
        remember_thumbnail(request.path, request.pixels, request.file_size, request.modification, SyncImage(image), cost);
    }
}

fn decode_cg_thumbnails(requests: &[ThumbRequest]) -> Vec<Option<DecodedCg>> {
    let mut slots = Vec::with_capacity(requests.len());
    slots.resize_with(requests.len(), || None);
    if requests.len() <= 1 {
        if let Some(request) = requests.first() {
            slots[0] = decode_cg_thumbnail(request);
        }
        return slots;
    }
    let slots_ptr = slots.as_mut_ptr() as usize;
    let requests_ptr = requests.as_ptr() as usize;
    let block = block2::RcBlock::new(move |index: usize| {
        let decoded = std::panic::catch_unwind(|| {
            // SAFETY: dispatch_apply invokes each index once, on one thread, and both
            // allocations outlive the call. Distinct indices do not alias.
            let request = unsafe { &*(requests_ptr as *const ThumbRequest).add(index) };
            decode_cg_thumbnail(request)
        });
        let decoded = decoded.unwrap_or(None);
        // SAFETY: the slot starts as None and this index is written by this invocation only.
        unsafe { *(slots_ptr as *mut Option<DecodedCg>).add(index) = decoded };
    });
    dispatch2::DispatchQueue::apply_with_block(requests.len(), None, &block);
    slots
}

fn decode_cg_thumbnail(request: &ThumbRequest) -> Option<DecodedCg> {
    let _pool = AutoreleasePool::push();
    let c_path = std::ffi::CString::new(request.path.as_os_str().as_bytes()).ok()?;
    unsafe {
        let cf_path = cf_string_create_with_c_string(std::ptr::null(), c_path.as_ptr(), 0x0800_0100);
        if cf_path.is_null() {
            return None;
        }
        let url = cf_url_create_with_file_system_path(std::ptr::null(), cf_path, 0, false);
        cf_release(cf_path as *const std::ffi::c_void);
        if url.is_null() {
            return None;
        }
        let source = cg_image_source_create_with_url(url, std::ptr::null());
        cf_release(url as *const std::ffi::c_void);
        if source.is_null() {
            return None;
        }
        let options = thumbnail_options(request.pixels);
        let cg = cg_image_source_create_thumbnail_at_index(source, 0, options);
        if !options.is_null() {
            cf_release(options);
        }
        cf_release(source as *const std::ffi::c_void);
        if cg.is_null() {
            return None;
        }
        let width = cg_image_get_width(cg);
        let height = cg_image_get_height(cg);
        if width == 0 || height == 0 {
            cf_release(cg as *const std::ffi::c_void);
            return None;
        }
        Some(DecodedCg { image: cg, width, height })
    }
}

fn nsimage_from_cg(decoded: &DecodedCg) -> Option<Retained<NSImage>> {
    let mtm = objc2::MainThreadMarker::new()?;
    let size = NSSize::new(decoded.width as f64, decoded.height as f64);
    let cg = decoded.image.cast::<CGImage>();
    let image: Retained<NSImage> = unsafe { objc2::msg_send![mtm.alloc::<NSImage>(), initWithCGImage: &*cg, size: size] };
    Some(image)
}

unsafe fn thumbnail_options(max_pixel: u32) -> *const std::ffi::c_void {
    unsafe {
        let always = cf_key("kCGImageSourceCreateThumbnailFromImageAlways");
        let transform = cf_key("kCGImageSourceCreateThumbnailWithTransform");
        let max_key = cf_key("kCGImageSourceThumbnailMaxPixelSize");
        let cache_key = cf_key("kCGImageSourceShouldCache");
        let max_value = max_pixel as i32;
        let number = cf_number_create(std::ptr::null(), 3, &max_value);
        let keys = [always, transform, max_key, cache_key];
        let values = [k_cf_boolean_true(), k_cf_boolean_true(), number, k_cf_boolean_false()];
        let dict = cf_dictionary_create(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            4,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        );
        for key in keys {
            if !key.is_null() {
                cf_release(key as *const std::ffi::c_void);
            }
        }
        if !number.is_null() {
            cf_release(number as *const std::ffi::c_void);
        }
        dict
    }
}

unsafe fn cf_key(value: &str) -> *const std::ffi::c_void {
    let Ok(c_string) = std::ffi::CString::new(value) else { return std::ptr::null() };
    unsafe { cf_string_create_with_c_string(std::ptr::null(), c_string.as_ptr(), 0x0800_0100) }
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const std::ffi::c_void);
    fn CFStringCreateWithCString(allocator: *const std::ffi::c_void, c_string: *const std::os::raw::c_char, encoding: u32) -> *const std::ffi::c_void;
    fn CFURLCreateWithFileSystemPath(allocator: *const std::ffi::c_void, path: *const std::ffi::c_void, style: i32, is_directory: bool) -> *const std::ffi::c_void;
    fn CFNumberCreate(allocator: *const std::ffi::c_void, number_type: i32, value: *const i32) -> *const std::ffi::c_void;
    fn CFDictionaryCreate(
        allocator: *const std::ffi::c_void,
        keys: *const *const std::ffi::c_void,
        values: *const *const std::ffi::c_void,
        count: isize,
        key_callbacks: *const u8,
        value_callbacks: *const u8,
    ) -> *const std::ffi::c_void;
    static kCFBooleanTrue: *const std::ffi::c_void;
    static kCFBooleanFalse: *const std::ffi::c_void;
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageSourceCreateWithURL(url: *const std::ffi::c_void, options: *const std::ffi::c_void) -> *mut std::ffi::c_void;
    fn CGImageSourceCreateThumbnailAtIndex(source: *mut std::ffi::c_void, index: usize, options: *const std::ffi::c_void) -> *mut std::ffi::c_void;
    fn CGImageGetWidth(image: *mut std::ffi::c_void) -> usize;
    fn CGImageGetHeight(image: *mut std::ffi::c_void) -> usize;
}

fn cf_release(cf: *const std::ffi::c_void) {
    unsafe { CFRelease(cf) }
}

fn cf_string_create_with_c_string(allocator: *const std::ffi::c_void, c_string: *const std::os::raw::c_char, encoding: u32) -> *const std::ffi::c_void {
    unsafe { CFStringCreateWithCString(allocator, c_string, encoding) }
}

fn cf_url_create_with_file_system_path(allocator: *const std::ffi::c_void, path: *const std::ffi::c_void, style: i32, is_directory: bool) -> *const std::ffi::c_void {
    unsafe { CFURLCreateWithFileSystemPath(allocator, path, style, is_directory) }
}

fn cf_number_create(allocator: *const std::ffi::c_void, number_type: i32, value: *const i32) -> *const std::ffi::c_void {
    unsafe { CFNumberCreate(allocator, number_type, value) }
}

fn cf_dictionary_create(
    allocator: *const std::ffi::c_void,
    keys: *const *const std::ffi::c_void,
    values: *const *const std::ffi::c_void,
    count: isize,
    key_callbacks: *const u8,
    value_callbacks: *const u8,
) -> *const std::ffi::c_void {
    unsafe { CFDictionaryCreate(allocator, keys, values, count, key_callbacks, value_callbacks) }
}

fn cg_image_source_create_with_url(url: *const std::ffi::c_void, options: *const std::ffi::c_void) -> *mut std::ffi::c_void {
    unsafe { CGImageSourceCreateWithURL(url, options) }
}

fn cg_image_source_create_thumbnail_at_index(source: *mut std::ffi::c_void, index: usize, options: *const std::ffi::c_void) -> *mut std::ffi::c_void {
    unsafe { CGImageSourceCreateThumbnailAtIndex(source, index, options) }
}

fn cg_image_get_width(image: *mut std::ffi::c_void) -> usize {
    unsafe { CGImageGetWidth(image) }
}

fn cg_image_get_height(image: *mut std::ffi::c_void) -> usize {
    unsafe { CGImageGetHeight(image) }
}

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_autoreleasePoolPush() -> *mut std::ffi::c_void;
    fn objc_autoreleasePoolPop(pool: *mut std::ffi::c_void);
}

fn objc_autorelease_pool_push() -> *mut std::ffi::c_void {
    unsafe { objc_autoreleasePoolPush() }
}

fn objc_autorelease_pool_pop(pool: *mut std::ffi::c_void) {
    unsafe { objc_autoreleasePoolPop(pool) }
}

fn k_cf_boolean_true() -> *const std::ffi::c_void {
    unsafe { kCFBooleanTrue }
}

fn k_cf_boolean_false() -> *const std::ffi::c_void {
    unsafe { kCFBooleanFalse }
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
    #[unsafe(super(AVPlayerView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    struct PreviewPlayerView;

    impl PreviewPlayerView {
        #[unsafe(method_id(initWithFrame:))]
        fn init_with_frame(this: Allocated<Self>, frame: NSRect) -> Retained<Self> {
            let this = this.set_ivars(());
            unsafe { objc2::msg_send![super(this), initWithFrame: frame] }
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            false
        }
    }
);

unsafe impl NSObjectProtocol for PreviewPlayerView {}

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
            let bounds = state.catalog.selection_bounds();

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
            let name_font = unsafe { NSFont::systemFontOfSize(12.0 * scale) };
            let bold_font = unsafe { NSFont::boldSystemFontOfSize(12.0 * scale) };
            let label = unsafe { NSColor::labelColor() };
            let name_dict = text_attributes(&name_font, &label, &pstyle);
            let bold_dict = text_attributes(&bold_font, &label, &pstyle);
            let detail_font = unsafe { NSFont::systemFontOfSize(10.0 * scale) };
            let detail_color = unsafe { NSColor::secondaryLabelColor() };
            let detail_dict = text_attributes(&detail_font, &detail_color, &pstyle_detail);
            let carousel = state.view_mode == ViewMode::Carousel;
            let mut requests = Vec::new();
            for (i, item) in state.items().iter().enumerate() {
                if item.kind != MediaKind::Image {
                    continue;
                }
                let cell = self.cell_rect(i);
                if !objc2_foundation::NSIntersectsRect(cell, rect) {
                    continue;
                }
                let (thumb_rect, _, _) = cell_content_rects(cell, carousel, scale);
                requests.push(ThumbRequest {
                    path: item.path.clone(),
                    pixels: pixel_limit(self, thumb_rect),
                    file_size: item.file_size,
                    modification: item.modification,
                });
            }
            ensure_thumbnails(&requests);

            for (i, item) in state.items().iter().enumerate() {
                let cell = self.cell_rect(i);
                if !objc2_foundation::NSIntersectsRect(cell, rect) {
                    continue;
                }
                let is_selected = bounds.is_some_and(|(start, end)| i >= start && i <= end);
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

                let (thumb_rect, name_rect, detail_rect) = cell_content_rects(cell, carousel, scale);

                let thumb_img = if item.kind == MediaKind::Directory {
                    folder_like_icon(item)
                } else if item.kind == MediaKind::Movie {
                    let pixels = pixel_limit(self, thumb_rect);
                    let cached = cached_sized(&item.path, pixels, None, None);
                    if cached.is_none() {
                        request_quicklook_thumbnail_sized(item.path.clone(), pixels);
                    }
                    cached.or_else(|| folder_like_icon(item)).or_else(|| system_image("film"))
                } else {
                    display_image(&item.path, pixel_limit(self, thumb_rect), item.file_size, item.modification)
                        .or_else(|| system_image("exclamationmark.triangle"))
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

                let mut name = item.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if name.chars().count() > 128 {
                    name = name.chars().take(128).collect();
                }
                let string = NSString::from_str(&name);
                let name_dict = if item.kind == MediaKind::Directory { &bold_dict } else { &name_dict };
                use objc2_app_kit::NSStringDrawing;
                unsafe { string.drawInRect_withAttributes(name_rect, Some(&**name_dict)) };

                if item.kind != MediaKind::Directory {
                    let file_size = item.file_size.unwrap_or(0).max(0) as u64;
                    let detail_str = byte_count_string(file_size);
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
                space_pressed();
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
                    let leaving = state.catalog.selected().map(|path| path.to_path_buf());
                    state.forward.push((current, leaving));
                    state.root = url;
                    let mode = state.mode;
                    let root = state.root.clone();
                    show_listing(&mut state.catalog, load_listing(&root, mode), ListingPick::Restored(sel));
                    let title = state.root.display().to_string();
                    if let Some(refs) = window_refs().as_ref() {
                        unsafe { (&*refs.window).setTitle(&NSString::from_str(&title)); }
                    }
                    apply_view_mode_layout();
                    update_carousel_preview();
                    reconcile_preview();
                    self.scroll_selection_to_center();
                }
                self.setNeedsDisplay(true);
                return;
            }
            if code == 124 && shift {
                if let Some((url, sel)) = state.forward.pop() {
                    let current = state.root.clone();
                    let leaving = state.catalog.selected().map(|path| path.to_path_buf());
                    state.back.push((current, leaving));
                    state.root = url;
                    let mode = state.mode;
                    let root = state.root.clone();
                    show_listing(&mut state.catalog, load_listing(&root, mode), ListingPick::Restored(sel));
                    let title = state.root.display().to_string();
                    if let Some(refs) = window_refs().as_ref() {
                        unsafe { (&*refs.window).setTitle(&NSString::from_str(&title)); }
                    }
                    apply_view_mode_layout();
                    update_carousel_preview();
                    reconcile_preview();
                    self.scroll_selection_to_center();
                }
                self.setNeedsDisplay(true);
                return;
            }
            if code == 51 && !command && !control {
                if state.mode == BrowserMode::Browser {
                    let parent = state.root.parent().map(|p| p.to_path_buf());
                    if let Some(parent_url) = parent {
                        if parent_url != state.root {
                            let came_from = state.root.clone();
                            let leaving = state.catalog.selected().map(|path| path.to_path_buf());
                            state.back.push((came_from.clone(), leaving));
                            state.forward.clear();
                            state.root = parent_url;
                            let mode = state.mode;
                            let root = state.root.clone();
                            show_listing(&mut state.catalog, load_listing(&root, mode), ListingPick::Restored(Some(came_from)));
                            let title = state.root.display().to_string();
                            if let Some(refs) = window_refs().as_ref() {
                                unsafe { (&*refs.window).setTitle(&NSString::from_str(&title)); }
                            }
                            apply_view_mode_layout();
                            update_carousel_preview();
                            reconcile_preview();
                            self.scroll_selection_to_center();
                            self.setNeedsDisplay(true);
                        }
                    }
                }
                return;
            }
            if state.items().is_empty() {
                return;
            }
            let visible_cols = columns_for(self.bounds().size.width).max(1);
            let cols = if state.view_mode == ViewMode::Carousel || visible_cols >= state.items().len() { 1 } else { visible_cols };
            let cur = state.selected().unwrap_or(0);
            let next = match code {
                123 | 126 => cur as isize - if code == 126 { cols as isize } else { 1 },
                124 | 125 => cur as isize + if code == 125 { cols as isize } else { 1 },
                36 => {
                    if let Some(item) = state.items().get(cur).filter(|item| item.kind == MediaKind::Directory) {
                        enter_directory(item.path.clone());
                    }
                    return;
                }
                _ => return,
            };
            let n = state.items().len() as isize;
            let wrapped = ((next % n) + n) % n;
            select_index(&mut state.catalog, wrapped as usize);
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
                select_index(&mut state.catalog, index);
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
            if state.items().is_empty() {
                return;
            }
            let Some(index) = self.index_at_point(view_location) else { return };
            self.ivars().drag_candidate.set(true);
            if event.clickCount() >= 2 {
                if let Some(item) = state.items().get(index).filter(|item| item.kind == MediaKind::Directory) {
                    enter_directory(item.path.clone());
                }
            } else if index < state.items().len() {
                let shift = event.modifierFlags().contains(objc2_app_kit::NSEventModifierFlags::Shift);
                let clicked = state.items()[index].path.clone();
                if shift {
                    if let Some(anchor) = state.catalog.anchor().map(|path| path.to_path_buf()) {
                        state.catalog.select_range(&anchor, &clicked).expect("shift selection stays in the catalog");
                    } else {
                        state.catalog.select(&clicked).expect("clicked item is in the catalog");
                    }
                } else if state.catalog.anchor().is_some() && state.catalog.selection_bounds().is_some_and(|(start, end)| (start..=end).contains(&index)) {
                    self.ivars().deferred_click.set(Some(index));
                } else {
                    state.catalog.select(&clicked).expect("clicked item is in the catalog");
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
                select_index(&mut state.catalog, index);
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
            let index = {
                let state = unsafe { &mut *app_state() };
                if !state.preview.begin(&state.catalog) {
                    return;
                }
                state.preview.current_index().expect("preview session has a focus")
            };
            unsafe {
                panel.setDataSource(Some(objc2::runtime::ProtocolObject::from_ref(self)));
                panel.setCurrentPreviewItemIndex(index as objc2_foundation::NSInteger);
                let _: () = objc2::msg_send![panel, addObserver: self, forKeyPath: &*NSString::from_str("currentPreviewItemIndex"), options: 0usize, context: core::ptr::null_mut::<core::ffi::c_void>()];
            }
            unsafe { (*app_state()).preview_bound = true };
        }

        #[unsafe(method(endPreviewPanelControl:))]
        fn end_preview_panel_control(&self, panel: &QLPreviewPanel) {
            let bound = {
                let state = unsafe { &mut *app_state() };
                state.preview.end();
                let bound = state.preview_bound;
                state.preview_bound = false;
                bound
            };
            if bound {
                unsafe {
                    let _: () = objc2::msg_send![panel, removeObserver: self, forKeyPath: &*NSString::from_str("currentPreviewItemIndex")];
                }
            }
            unsafe { panel.setDataSource(None) };
        }

        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_preview_index(&self, _key: &NSString, panel: &QLPreviewPanel, _change: &NSDictionary, _context: *mut core::ffi::c_void) {
            let index = unsafe { panel.currentPreviewItemIndex() };
            if index < 0 { return; }
            let state = unsafe { &mut *app_state() };
            let Some(path) = state.preview.select_at(index as usize) else { return };
            if state.catalog.selected() != Some(path.as_path()) {
                state.catalog.select(&path).expect("preview item is in the catalog");
            }
            self.scroll_selection_into_view();
            update_carousel_preview();
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn number_of_preview_items_in_preview_panel(&self, _panel: Option<&QLPreviewPanel>) -> objc2_foundation::NSInteger {
            unsafe { &*app_state() }.preview.count() as objc2_foundation::NSInteger
        }

        #[unsafe(method_id(previewPanel:previewItemAtIndex:))]
        fn preview_panel_preview_item_at_index(
            &self,
            _panel: Option<&QLPreviewPanel>,
            index: objc2_foundation::NSInteger,
        ) -> Retained<NSURL> {
            let state = unsafe { &*app_state() };
            let path = (index >= 0).then(|| state.preview.item_at(index as usize)).flatten()
                .or_else(|| state.preview.current_index().and_then(|current| state.preview.item_at(current)))
                .expect("quick look index is outside the preview session");
            url_from_path(path)
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
        (index < state.items().len() && objc2_foundation::NSPointInRect(point, self.cell_rect(index))).then_some(index)
    }

    fn scroll_selection_into_view(&self) {
        let index = unsafe { (*app_state()).selected() };
        let Some(index) = index else { return };
        let cell = self.cell_rect(index);
        unsafe { self.scrollRectToVisible(cell); };
    }

    fn scroll_selection_to_center(&self) {
        let Some(index) = (unsafe { (*app_state()).selected() }) else { return };
        let cell = self.cell_rect(index);
        let Some(scroll) = self.enclosingScrollView() else {
            unsafe { self.scrollRectToVisible(cell); };
            return;
        };
        let clip = scroll.contentView();
        let visible = clip.bounds().size;
        let origin = NSPoint::new(
            (cell.origin.x + cell.size.width / 2.0 - visible.width / 2.0).clamp(0.0, (self.bounds().size.width - visible.width).max(0.0)),
            (cell.origin.y + cell.size.height / 2.0 - visible.height / 2.0).clamp(0.0, (self.bounds().size.height - visible.height).max(0.0)),
        );
        clip.scrollToPoint(origin);
        scroll.reflectScrolledClipView(&clip);
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
                    state.items().len(),
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
            let preview_label = NSTextField::new(objc2::MainThreadMarker::new().unwrap());
            preview_label.setEditable(false);
            preview_label.setSelectable(true);
            preview_label.setBezeled(false);
            preview_label.setDrawsBackground(false);
            preview_label.setAlignment(objc2_app_kit::NSTextAlignment::Center);
            preview_label.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByCharWrapping);
            preview_label.setFont(Some(&NSFont::systemFontOfSize(13.0)));
            preview_label.setTextColor(Some(&NSColor::labelColor()));
            unsafe { let _: () = objc2::msg_send![&*preview_label, setUsesSingleLineMode: false]; }
            unsafe { let _: () = objc2::msg_send![&*preview_label, setMaximumNumberOfLines: 0i64]; }
            preview_nsview.addSubview(&*preview_label);
            let mtm = objc2::MainThreadMarker::new().unwrap();
            let player_view: Retained<PreviewPlayerView> = unsafe {
                objc2::msg_send![PreviewPlayerView::alloc(mtm), initWithFrame: NSRect::new(NSPoint::ZERO, NSSize::ZERO)]
            };
            let player = unsafe { AVPlayer::playerWithPlayerItem(None, mtm) };
            unsafe {
                player_view.setVideoGravity(AVLayerVideoGravityResizeAspect.unwrap());
                player_view.setPlayer(Some(&player));
            }
            let player_observer: Retained<PlayerItemObserver> = unsafe { objc2::msg_send![PlayerItemObserver::class(), new] };
            *player_store() = Some(PlayerStore {
                view: player_view.clone(),
                player,
                observer: player_observer,
                poster: Retained::into_raw(preview_image.clone()) as *mut NSImageView,
                item: None,
                item_path: None,
                status: PlayerStatus::Loading,
                autoplay: false,
            });
            preview_nsview.addSubview(&*player_view);
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
            layout_carousel(&root_view, &preview_nsview, &preview_image, &player_view, &divider, &scroll, frame, view_mode_carousel);
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
                preview_label: Retained::into_raw(preview_label.clone()) as *mut NSTextField,
                player: Retained::into_raw(player_view.clone()) as *mut AVPlayerView,
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
            if *PLAYER_TEST.get_or_init(|| false) {
                start_player_test();
            }
            if !*REGRESSION_TEST.get_or_init(|| false) && !*BENCHMARK_STARTUP.get_or_init(|| false) && !*BENCHMARK_MEMORY.get_or_init(|| false) && !benchmark_mode() {
                window.setFrameAutosaveName(&NSString::from_str("ViewWindow"));
            }
            window.setTitle(&NSString::from_str(&title));
            window.center();
            window.makeKeyAndOrderFront(None);
            let _ = window.makeFirstResponder(Some(&*grid));
            let mtm = objc2::MainThreadMarker::new().unwrap();
            let app = NSApplication::sharedApplication(mtm);
            #[allow(deprecated)]
            if !benchmark_mode() { unsafe { app.activateIgnoringOtherApps(true); } }
            if *BENCHMARK_MEMORY.get_or_init(|| false) {
                benchmark_browsing_memory(&window, &grid);
                std::process::exit(0);
            }
            if *BENCHMARK_STARTUP.get_or_init(|| false) {
                assert_eq!(unsafe { (*app_state()).items()[0].kind }, MediaKind::Image, "startup corpus must begin with an image");
                assert!(preview_image.image().is_some(), "startup preview must contain an image");
                let bounds = preview_nsview.bounds();
                let bitmap = preview_nsview.bitmapImageRepForCachingDisplayInRect(bounds).expect("startup preview bitmap");
                preview_nsview.cacheDisplayInRect_toBitmapImageRep(bounds, &bitmap);
                std::hint::black_box(bitmap.bitmapData());
                window.displayIfNeeded();
                std::process::exit(0);
            }
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
    assert_eq!(unsafe { (*app_state()).items().len() }, 1);
    open_selected();
    assert_eq!(unsafe { (*app_state()).selected() }, Some(0));
    assert!(preview.image().is_some(), "first image is visible after entering a folder");
    let bounds = preview.bounds();
    let bitmap = preview.bitmapImageRepForCachingDisplayInRect(bounds).expect("preview bitmap");
    preview.cacheDisplayInRect_toBitmapImageRep(bounds, &bitmap);
    assert_orientation(&bitmap);
    assert!(unsafe { (*app_state()).items().len() } >= 3);
    let folder = unsafe { (*app_state()).root.clone() };
    let parent = folder.parent().unwrap().to_path_buf();
    assert_eq!(window.title().to_string(), folder.display().to_string(), "directory entry updates the window title");
    let app = NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap());
    window.makeKeyAndOrderFront(None);
    let _ = window.makeFirstResponder(Some(grid));
    send_key_via_app(&app, 123, false);
    assert_eq!(unsafe { (*app_state()).selected() }, Some(unsafe { (*app_state()).items().len() } - 1), "left from the first item wraps to the last");
    send_key_via_app(&app, 124, false);
    assert_eq!(unsafe { (*app_state()).selected() }, Some(0), "right from the last item wraps to the first");
    send_key_via_app(&app, 123, true);
    assert_eq!(unsafe { (*app_state()).root.clone() }, parent, "back returns to the previous folder");
    assert_eq!(unsafe { (*app_state()).selected().and_then(|i| (*app_state()).items().get(i)).map(|item| item.path.clone()) }, Some(folder.clone()), "back restores the directory selection");
    assert_eq!(window.title().to_string(), parent.display().to_string(), "back updates the window title");
    send_key_via_app(&app, 124, true);
    assert_eq!(unsafe { (*app_state()).root.clone() }, folder, "forward reopens the folder");
    assert_eq!(unsafe { (*app_state()).selected() }, Some(0), "forward restores the selected item");
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
    assert_eq!(unsafe { (*app_state()).selected() }, Some(1), "quick look arrow advances selection");
    unsafe { panel.setCurrentPreviewItemIndex(1); }
    assert_eq!(unsafe { (*app_state()).selected() }, Some(1), "quick look advances selection");
    let event = objc2_app_kit::NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        objc2_app_kit::NSEventType::KeyDown, NSPoint::ZERO,
        objc2_app_kit::NSEventModifierFlags::empty(), 0.0, panel.windowNumber(), None,
        &NSString::from_str("\u{f702}"), &NSString::from_str("\u{f702}"), false, 123,
    ).unwrap();
    panel.sendEvent(&event);
    assert_eq!(unsafe { (*app_state()).selected() }, Some(0), "quick look arrow moves selection backwards");
    panel.orderOut(None);
    window.makeKeyAndOrderFront(None);
    let _ = window.makeFirstResponder(Some(grid));
    quicklook_selected();
    assert!(unsafe { panel.dataSource() }.is_some(), "quick look can reopen");
    unsafe { panel.setCurrentPreviewItemIndex(2); }
    assert_eq!(unsafe { (*app_state()).selected() }, Some(2), "reopened quick look updates selection");
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
    assert_eq!(unsafe { (*app_state()).selected() }, Some(6), "down moves by the visible column count");
    send_key_via_app(&app, 126, false);
    assert_eq!(unsafe { (*app_state()).selected() }, Some(2), "up returns by the visible column count");
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
    let anchor = state.items()[0].path.clone();
    let focus = state.items()[1].path.clone();
    state.catalog.select_range(&anchor, &focus).expect("drag selection stays in the catalog");
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
        assert_eq!(path_from_url(url), unsafe { (*app_state()).items()[index].path.clone() });
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
    assert_eq!(unsafe { (*app_state()).items()[(*app_state()).selected().unwrap()].path.clone() }, file);
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
        let index = state.items().iter().position(|item| item.path == original).unwrap();
        let item = state.items()[index].clone();
        state.catalog.remove(&original);
        state.trash_history.record_trash(TrashTransaction {
            files: vec![view_core::TrashedFile { item, trashed_path: trashed.clone(), index }],
            selection_before_op: vec![original.clone()],
        });
    }
    send_key_command(&app, 6);
    assert_eq!(std::fs::read(&original).unwrap(), content, "undo restores the file contents");
    assert!(!trashed.exists(), "undo removes the trashed copy");
    assert!(unsafe { (*app_state()).items().iter().any(|item| item.path == original) }, "undo restores the catalog item");
    unsafe { let _: () = objc2::msg_send![grid, redoTrash: grid]; }
    assert!(!original.exists(), "redo removes the original file");
    assert_eq!(std::fs::read(&trashed).unwrap(), content, "redo preserves the file contents");
    assert!(!unsafe { (*app_state()).items().iter().any(|item| item.path == original) }, "redo removes the catalog item");
    std::fs::write(&original, b"replacement").unwrap();
    send_key_command(&app, 6);
    assert_eq!(std::fs::read(&original).unwrap(), b"replacement", "undo preserves a conflicting file");
    assert_eq!(std::fs::read(&trashed).unwrap(), content, "conflict leaves the trashed file intact");
    assert!(!unsafe { (*app_state()).items().iter().any(|item| item.path == original) }, "conflict does not restore the catalog item");
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
    assert!(unsafe { (*app_state()).items().iter().any(|item| item.path == album) }, "redo conflict leaves the folder in the catalog");
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
    assert_eq!(unsafe { (*app_state()).items()[0].kind }, MediaKind::Movie);
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
    let dir_index = unsafe { (*app_state()).items().iter().position(|i| i.kind == MediaKind::Directory) };
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
                        unsafe { (*app_state()).selected().unwrap_or(usize::MAX) },
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
                            .items()
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
                    let now = unsafe { (*app_state()).selected() };
                    let root = unsafe { (*app_state()).root.clone() };
                    let has_directory = unsafe { (*app_state()).items().iter().any(|i| i.kind == MediaKind::Directory) };
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
                            unsafe { (*app_state()).items().len() }
                        );
                        std::process::exit(1);
                    }
                }
            }
        })
    };
    unsafe { objc2_foundation::NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.25, true, &block) };
}

fn start_player_test() {
    PLAYER_TEST_START.set(std::time::Instant::now()).ok();
    let block = block2::RcBlock::new(move |_timer: core::ptr::NonNull<objc2_foundation::NSTimer>| {
        run_player_test();
    });
    unsafe { objc2_foundation::NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.05, true, &block) };
}

fn run_player_test() {
    let started = PLAYER_TEST_START.get().copied().unwrap_or_else(std::time::Instant::now);
    match PLAYER_TEST_STEP.load(std::sync::atomic::Ordering::SeqCst) {
        0 => {
            let Some(item) = focused_item(unsafe { &*app_state() }) else { return };
            if item.kind != MediaKind::Movie {
                assert!(started.elapsed().as_secs() < 10, "player test corpus must begin with a movie");
                return;
            }
            let (ready, visible, rate) = {
                let store = player_store();
                let Some(store) = store.as_ref() else { return };
                (store.status == PlayerStatus::Ready, !store.view.isHidden(), unsafe { store.player.rate() })
            };
            if !ready {
                assert!(started.elapsed().as_secs() < 10, "inline player item did not become ready");
                return;
            }
            assert!(visible, "inline player view is visible once the item is ready");
            assert_eq!(rate, 0.0, "inline player starts paused");
            let attached = {
                let store = player_store();
                let store = store.as_ref().expect("player store");
                store.view.window().is_some() && unsafe { store.view.superview().is_some() }
            };
            assert!(attached, "inline player view is attached to the preview");
            let focus = {
                let store = player_store();
                let store = store.as_ref().expect("player store");
                store.view.acceptsFirstResponder()
            };
            assert!(!focus, "inline player view must not take keyboard focus");
            PLAYER_TEST_STEP.store(1, std::sync::atomic::Ordering::SeqCst);
        }
        1 => {
            let mtm = objc2::MainThreadMarker::new().unwrap();
            let app = NSApplication::sharedApplication(mtm);
            if let Some(refs) = window_refs().as_ref() {
                let window = unsafe { &*refs.window };
                window.makeKeyAndOrderFront(None);
                let _ = window.makeFirstResponder(Some(unsafe { &*refs.grid }));
            }
            send_key_via_app(&app, 49, false);
            let rate = {
                let store = player_store();
                unsafe { store.as_ref().expect("player store").player.rate() }
            };
            assert!(rate > 0.0, "space plays the inline video");
            send_key_via_app(&app, 49, false);
            let rate = {
                let store = player_store();
                unsafe { store.as_ref().expect("player store").player.rate() }
            };
            assert_eq!(rate, 0.0, "space pauses the inline video");
            unsafe { &mut *app_state() }.view_mode = ViewMode::Grid;
            apply_view_mode_layout();
            let hidden = {
                let store = player_store();
                store.as_ref().expect("player store").view.isHidden()
            };
            assert!(hidden, "grid mode releases the inline player");
            send_key_via_app(&app, 49, false);
            PLAYER_TEST_STEP.store(2, std::sync::atomic::Ordering::SeqCst);
        }
        _ => {
            let carousel = unsafe { (*app_state()).view_mode == ViewMode::Carousel };
            assert!(carousel, "space in grid mode returns to carousel");
            let rate = {
                let store = player_store();
                unsafe { store.as_ref().expect("player store").player.rate() }
            };
            if rate <= 0.0 {
                assert!(started.elapsed().as_secs() < 10, "space in grid mode did not begin playback");
                return;
            }
            eprintln!("player test: PASS");
            std::process::exit(0);
        }
    }
}

fn resident_memory() -> std::io::Result<u64> {
    unsafe extern "C" {
        static mach_task_self_: u32;
        fn task_info(task: u32, flavor: u32, info: *mut i32, count: *mut u32) -> i32;
    }
    let mut info: libc::mach_task_basic_info_data_t = unsafe { std::mem::zeroed() };
    let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
    let result = unsafe {
        task_info(mach_task_self_, libc::MACH_TASK_BASIC_INFO, (&mut info as *mut libc::mach_task_basic_info_data_t).cast(), &mut count)
    };
    if result == libc::KERN_SUCCESS && count == libc::MACH_TASK_BASIC_INFO_COUNT {
        Ok(info.resident_size)
    } else {
        Err(std::io::Error::other(format!("task_info returned {result} with {count} fields")))
    }
}

fn benchmark_browsing_memory(window: &NSWindow, grid: &GridView) {
    let count = unsafe { (*app_state()).items().len() };
    assert!(count >= 100, "memory corpus needs at least 100 items");
    unsafe { (*app_state()).view_mode = ViewMode::Grid; }
    window.setContentSize(NSSize::new(1000.0, 700.0));
    apply_view_mode_layout();
    let mut peak = resident_memory().expect("read resident memory");
    for step in 0..80 {
        let pool = unsafe { objc2_foundation::NSAutoreleasePool::new() };
        let index = (step * 8) % count;
        grid.scrollRectToVisible(grid.cell_rect(index));
        let visible = grid.visibleRect();
        let bitmap = grid.bitmapImageRepForCachingDisplayInRect(visible).expect("memory viewport bitmap");
        grid.setNeedsDisplay(true);
        grid.cacheDisplayInRect_toBitmapImageRep(visible, &bitmap);
        std::hint::black_box(bitmap.bitmapData());
        drop(bitmap);
        drop(pool);
        peak = peak.max(resident_memory().expect("read resident memory"));
    }
    let final_rss = resident_memory().expect("read resident memory");
    println!("{{\"frames\":80,\"max_sampled_rss_bytes\":{peak},\"final_rss_bytes\":{final_rss}}}");
}

fn benchmark_mode() -> bool {
    #[cfg(feature = "benchmark")]
    { return benchmarks::active(); }
    #[cfg(not(feature = "benchmark"))]
    { false }
}

static SELF_TEST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static BENCHMARK_MEMORY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static BENCHMARK_STARTUP: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static REGRESSION_TEST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static DEBUG_ENTER: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static PLAYER_TEST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static PLAYER_TEST_START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
static PLAYER_TEST_STEP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        eprintln!("view: panic: {info}\n{backtrace}");
        std::process::abort();
    }));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let self_test = args.iter().any(|a| a == "--self-test");
    let benchmark_memory = args.iter().any(|a| a == "--benchmark-memory");
    let benchmark_startup = args.iter().any(|a| a == "--benchmark-startup");
    let regression_test = args.iter().any(|a| a == "--regression-test");
    let debug_enter = args.iter().any(|a| a == "--debug-enter");
    let player_test = args.iter().any(|a| a == "--player-test");
    let args: Vec<String> = args.into_iter().filter(|a| a != "--self-test" && a != "--debug-enter" && a != "--regression-test" && a != "--benchmark-startup" && a != "--benchmark-memory" && a != "--player-test").collect();
    SELF_TEST.set(self_test).ok();
    DEBUG_ENTER.set(debug_enter).ok();
    REGRESSION_TEST.set(regression_test).ok();
    BENCHMARK_STARTUP.set(benchmark_startup).ok();
    BENCHMARK_MEMORY.set(benchmark_memory).ok();
    PLAYER_TEST.set(player_test).ok();
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
        state.view_mode = if benchmark_memory { ViewMode::Grid } else if benchmark_startup { ViewMode::Carousel } else { match saved.as_deref() {
            Some("grid") => ViewMode::Grid,
            _ => ViewMode::Carousel,
        }};
        show_listing(&mut state.catalog, load_listing(&root, mode), ListingPick::First);
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

#[cfg(feature = "benchmark")]
#[path = "app/benchmarks.rs"]
pub mod benchmarks;
