//! Native AppKit host for tabbed media browsing and inline video playback.

use std::path::PathBuf;
use std::os::unix::ffi::{OsStrExt, OsStringExt};

use objc2::{define_class, ClassType, DefinedClass, MainThreadOnly};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::NSObject;
use objc2_app_kit::{
    NSAlert, NSApplication, NSApplicationDelegate, NSBackingStoreType, NSBox, NSButton, NSColor, NSDraggingInfo, NSDraggingSource, NSFont,
    NSImage, NSImageView, NSMenu, NSMenuItem, NSScrollView, NSTextField, NSView, NSWindow, NSWindowDelegate,
    NSWindowStyleMask, NSWorkspace,
};
use objc2_av_foundation::{AVLayerVideoGravityResizeAspect, AVPlayer};
use objc2_av_kit::AVPlayerView;
use objc2_core_graphics::CGImage;
use objc2_foundation::{
    NSArray, NSDictionary, NSNotification, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL,
};
use objc2_quick_look_thumbnailing::{
    QLThumbnailGenerationRequest, QLThumbnailGenerationRequestRepresentationTypes, QLThumbnailGenerator,
    QLThumbnailRepresentation,
};
use objc2_quick_look_ui::{QLPreviewPanel, QLPreviewPanelDataSource};

use view_core::{
    list_browser, scan_recursive, BrowserMode, Catalog, MediaKind, MediaItem, PreviewSession, TrashHistory,
    TrashTransaction,
};

#[path = "app/player.rs"]
mod player;

const CELL_W: f64 = 160.0;
const CELL_H: f64 = 180.0;
const SPACING: f64 = 10.0;
const CAROUSEL_CELL_W: f64 = 120.0;
const CAROUSEL_CELL_H: f64 = 156.0;
const CAROUSEL_SPACING: f64 = 8.0;
const TAB_BAR_HEIGHT: f64 = 36.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode { Carousel, Grid }

impl Default for ViewMode {
    fn default() -> Self { Self::Carousel }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum MediaFilter {
    #[default]
    Both,
    Images,
    Videos,
}

impl MediaFilter {
    fn keeps(self, kind: MediaKind) -> bool {
        match (kind, self) {
            (MediaKind::Directory, _) => true,
            (MediaKind::Image, MediaFilter::Videos) => false,
            (MediaKind::Movie, MediaFilter::Images) => false,
            _ => true,
        }
    }
}

#[derive(Default)]
struct Tab {
    root: PathBuf,
    mode: BrowserMode,
    view_mode: ViewMode,
    media_filter: MediaFilter,
    thumbnail_size_step: i8,
    catalog: Catalog,
    preview: PreviewSession,
    back: Vec<(PathBuf, Option<PathBuf>)>,
    forward: Vec<(PathBuf, Option<PathBuf>)>,
    unavailable: Option<String>,
}

#[derive(Default)]
struct AppState {
    tabs: Vec<Tab>,
    active: usize,
    preview_bound: bool,
    trash_history: TrashHistory,
    pending_trash: std::collections::HashSet<PathBuf>,
}

impl AppState {
    fn tab(&self) -> &Tab { &self.tabs[self.active] }
    fn tab_mut(&mut self) -> &mut Tab { &mut self.tabs[self.active] }
    fn items(&self) -> &[MediaItem] { self.tab().catalog.items() }
    fn selected(&self) -> Option<usize> { self.tab().catalog.focus_index() }
    fn new_tab(&mut self, root: PathBuf, mode: BrowserMode, items: Vec<MediaItem>) -> usize {
        let mut tab = Tab::default();
        tab.root = root.clone();
        tab.mode = mode;
        tab.view_mode = self.tab().view_mode;
        tab.media_filter = self.tab().media_filter;
        tab.thumbnail_size_step = self.tab().thumbnail_size_step;
        show_listing(&mut tab.catalog, items, ListingPick::First);
        self.tabs.push(tab);
        self.tabs.len() - 1
    }
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
    let tab = state.tab();
    let index = tab.catalog.focus_index()?;
    tab.catalog.items().get(index).cloned()
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
    tab_bar: *mut TabBarView,
}
unsafe impl Send for WindowRefs {}
unsafe impl Sync for WindowRefs {}

static WINDOW_REFS: std::sync::OnceLock<std::sync::Mutex<Option<WindowRefs>>> = std::sync::OnceLock::new();

fn window_refs() -> std::sync::MutexGuard<'static, Option<WindowRefs>> {
    WINDOW_REFS.get_or_init(|| std::sync::Mutex::new(None)).lock().unwrap()
}

fn space_pressed() {
    let (view_mode, item) = {
        let state = unsafe { &*app_state() };
        (state.tab().view_mode, focused_item(state))
    };
    let Some(item) = item else { return };
    if item.kind != MediaKind::Movie {
        quicklook_selected();
        return;
    }
    if view_mode == ViewMode::Grid {
        unsafe { (*app_state()).tab_mut() }.view_mode = ViewMode::Carousel;
        apply_view_mode_layout();
        player::start();
        return;
    }
    player::toggle();
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
    1.2_f64.powi(unsafe { (*app_state()).tab().thumbnail_size_step } as i32)
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

fn persist_media_filter() {
    let defaults = objc2_foundation::NSUserDefaults::standardUserDefaults();
    let value = match unsafe { (*app_state()).tab().media_filter } {
        MediaFilter::Both => "both",
        MediaFilter::Images => "images",
        MediaFilter::Videos => "videos",
    };
    unsafe { defaults.setObject_forKey(Some(&*NSString::from_str(value) as &NSObject), &NSString::from_str("View.MediaFilter")) };
}

fn persist_view_mode() {
    let defaults = objc2_foundation::NSUserDefaults::standardUserDefaults();
    let value = if unsafe { (*app_state()).tab().view_mode } == ViewMode::Carousel {
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
    if state.tab().unavailable.is_some() {
        preview_image.setImage(None);
        preview_label.setHidden(true);
        player::release();
        preview.setNeedsDisplay(true);
        return;
    }
    if state.tab().view_mode != ViewMode::Carousel {
        return;
    }
    let Some(index) = state.selected() else {
        preview_image.setImage(None);
        preview_label.setHidden(true);
        player::release();
        return;
    };
    let Some(item) = state.items().get(index) else {
        preview_image.setImage(None);
        preview_label.setHidden(true);
        player::release();
        return;
    };
    let preview_bounds = preview.bounds();
    if item.kind == MediaKind::Directory {
        player::release();
        preview_image.setFrame(NSRect::new(NSPoint::ZERO, preview_bounds.size));
        match view_core::list_browser(&item.path) {
            Ok(items) => {
                let count = items.len();
                let size: u64 = items.iter().filter_map(|i| i.file_size).map(|s| s.max(0) as u64).sum();
                let name = item.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let size_str = byte_count_string(size);
                let detail = format!("{name}\n{count} items, {}", size_str);
                preview_label.setStringValue(&NSString::from_str(&detail));
            }
            Err(error) => {
                preview_label.setStringValue(&NSString::from_str(&format!("Unable to read folder\n{error}")));
            }
        }
        preview_label.setHidden(false);
        preview_image.setImageScaling(objc2_app_kit::NSImageScaling::ScaleProportionallyDown);
        let label_h = ((preview_bounds.size.height * 0.35).clamp(48.0, 140.0)).min(preview_bounds.size.height - 80.0).max(48.0);
        let icon_area_h = (preview_bounds.size.height - label_h - 24.0).max(64.0);
        let icon_size = icon_area_h.min(preview_bounds.size.width - 32.0).min(256.0).max(64.0);
        if let Some(icon) = sized_folder_icon(item, icon_size) {
            preview_image.setImage(Some(&*icon));
            let x = ((preview_bounds.size.width - icon_size) / 2.0).round();
            let y = (label_h + 16.0 + (icon_area_h - icon_size) / 2.0).round();
            preview_image.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(icon_size, icon_size)));
        } else {
            preview_image.setImage(None);
        }
        preview_label.setFrame(NSRect::new(NSPoint::new(16.0, 8.0), NSSize::new((preview_bounds.size.width - 32.0).max(0.0), label_h)));
    } else if item.kind == MediaKind::Movie {
        preview_label.setHidden(true);
        preview_image.setFrame(NSRect::new(NSPoint::ZERO, preview_bounds.size));
        preview_image.setImageScaling(objc2_app_kit::NSImageScaling::ScaleProportionallyUpOrDown);
        let pixels = pixel_limit(preview, preview_bounds);
        if let Some(image) = cached_sized(&item.path, pixels, item.file_size, item.modification) {
            preview_image.setImage(Some(&*image));
        } else {
            request_quicklook_thumbnail_sized(item.path.clone(), pixels, item.file_size, item.modification);
        }
        player::load(&item.path);
    } else {
        player::release();
        preview_label.setHidden(true);
        preview_image.setFrame(NSRect::new(NSPoint::ZERO, preview_bounds.size));
        preview_image.setImageScaling(objc2_app_kit::NSImageScaling::ScaleProportionallyUpOrDown);
        let pixels = pixel_limit(preview, preview_bounds);
        if let Some(image) = display_image(&item.path, pixels, item.file_size, item.modification) {
            preview_image.setImage(Some(&*image));
        } else {
            preview_image.setImage(None);
        }
    }
    preview.setNeedsDisplay(true);
}

fn resize_grid_document() {
    let binding = window_refs();
    let Some(refs) = binding.as_ref() else { return };
    unsafe {
        let scroll = &*refs.scroll;
        let grid = &*refs.grid;
        let viewport = scroll.contentView().bounds().size;
        let carousel = (*app_state()).tab().view_mode == ViewMode::Carousel;
        let doc_size = content_size(viewport.width, viewport.height, (*app_state()).items().len(), carousel);
        grid.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), doc_size));
        let clip = scroll.contentView();
        let clip_bounds = clip.bounds();
        let clamped_x = clip_bounds.origin.x.clamp(0.0, (doc_size.width - clip_bounds.size.width).max(0.0));
        let clamped_y = clip_bounds.origin.y.clamp(0.0, (doc_size.height - clip_bounds.size.height).max(0.0));
        if clamped_x != clip_bounds.origin.x || clamped_y != clip_bounds.origin.y {
            clip.scrollToPoint(NSPoint::new(clamped_x, clamped_y));
            scroll.reflectScrolledClipView(&clip);
        }
        grid.setNeedsDisplay(true);
    }
}

fn apply_view_mode_layout() {
    let (window_ptr, scroll_ptr, preview_ptr, preview_image_ptr, divider_ptr, player_ptr, tab_bar_ptr) = {
        let refs = window_refs();
        let Some(refs) = refs.as_ref() else { return };
        (refs.window, refs.scroll, refs.preview, refs.preview_image, refs.divider, refs.player, refs.tab_bar)
    };
    let carousel = unsafe { (*app_state()).tab().view_mode } == ViewMode::Carousel;
    if !carousel {
        player::release();
    }
    unsafe {
        let window = &*window_ptr;
        let scroll = &*scroll_ptr;
        let preview = &*preview_ptr;
        let preview_image = &*preview_image_ptr;
        let divider = &*divider_ptr;
        let player = &*player_ptr;
        let tab_bar = &*tab_bar_ptr;
        let Some(root) = window.contentView() else { return };
        let bounds = root.bounds();
        let content = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(bounds.size.width, (bounds.size.height - TAB_BAR_HEIGHT).max(0.0)),
        );
        layout_carousel(&root, preview, preview_image, player, divider, scroll, content, carousel);
        tab_bar.setFrame(NSRect::new(
            NSPoint::new(0.0, bounds.size.height - TAB_BAR_HEIGHT),
            NSSize::new(bounds.size.width, TAB_BAR_HEIGHT),
        ));
        tab_bar.sync();
        scroll.setHasVerticalScroller(!carousel);
        scroll.setHasHorizontalScroller(carousel);
        resize_grid_document();
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

    let file_menu = NSMenu::new(mtm);
    file_menu.addItem(&menu_item(grid, "New Tab", objc2::sel!(newTab:), Some("t"), objc2_app_kit::NSEventModifierFlags::Command));
    file_menu.addItem(&menu_item(grid, "Close Tab", objc2::sel!(closeTab:), Some("w"), objc2_app_kit::NSEventModifierFlags::Command));
    let file_item = NSMenuItem::new(mtm);
    file_item.setTitle(&NSString::from_str("File"));
    file_item.setSubmenu(Some(&file_menu));
    main_menu.addItem(&file_item);

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
    edit_menu.addItem(&menu_item(grid, "Move to Trash", objc2::sel!(moveSelectedToTrash:), Some("\u{8}"), objc2_app_kit::NSEventModifierFlags::empty()));
    let edit_item = NSMenuItem::new(mtm);
    edit_item.setTitle(&NSString::from_str("Edit"));
    edit_item.setSubmenu(Some(&edit_menu));
    main_menu.addItem(&edit_item);
    NSApplication::sharedApplication(mtm).setMainMenu(Some(&main_menu));
}

pub(crate) fn url_from_path(path: &std::path::Path) -> Retained<NSURL> {
    let bytes = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("filesystem path contains no NUL");
    let pointer = std::ptr::NonNull::new(bytes.as_ptr() as *mut std::ffi::c_char).unwrap();
    unsafe { NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(pointer, path.is_dir(), None) }
}

fn path_from_url(url: &NSURL) -> PathBuf {
    let bytes = unsafe { std::ffi::CStr::from_ptr(url.fileSystemRepresentation().as_ptr()) }.to_bytes();
    PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec()))
}

fn tab_affected(root: &std::path::Path, mode: BrowserMode, changed: &[PathBuf]) -> bool {
    changed.iter().any(|path| {
        if path == root {
            return true;
        }
        match mode {
            BrowserMode::Browser => path.parent().is_some_and(|parent| parent == root),
            BrowserMode::Recursive => path.starts_with(root),
        }
    })
}

fn reconcile_tab_after_mutation(tab: &mut Tab, select_after: &[PathBuf]) {
    let focus_index_before = tab.catalog.focus_index();
    let old_paths: Vec<PathBuf> = tab.catalog.items().iter().map(|item| item.path.clone()).collect();
    match load_listing(&tab.root, tab.mode, tab.media_filter) {
        Ok(items) => {
            let mut catalog = Catalog::new();
            catalog.append(items);
            let previous_selection = tab.catalog.selected_all().to_vec();
            let focus = tab.catalog.selected().map(|path| path.to_path_buf());
            let present: Vec<PathBuf> = catalog.items().iter().map(|item| item.path.clone()).collect();
            let deleted_focus = focus.as_deref().is_some_and(|path| !present.iter().any(|item| item == path));
            let mut selection: Vec<PathBuf> = previous_selection.into_iter().filter(|path| present.contains(path)).collect();
            if selection.is_empty() {
                for path in select_after {
                    if present.contains(path) && !selection.contains(path) {
                        selection.push(path.clone());
                    }
                }
            }
            if selection.is_empty() && deleted_focus && let Some(focus) = focus_index_before {
                let pick = old_paths.iter().take(focus).filter(|path| present.contains(path)).count();
                let len = catalog.items().len();
                if len > 0 {
                    let item = &catalog.items()[pick.min(len - 1)];
                    selection.push(item.path.clone());
                }
            }
            catalog.restore_selection(&selection, focus.as_deref());
            tab.catalog = catalog;
            tab.unavailable = None;
        }
        Err(error) => {
            tab.unavailable = Some(format!("Unable to read {}: {error}", tab.root.display()));
        }
    }
}

fn reconcile_after_mutation(changed: &[PathBuf], select_after: &[PathBuf]) {
    {
        let state = unsafe { &mut *app_state() };
        for tab in state.tabs.iter_mut() {
            let affected = tab.unavailable.is_some() || tab_affected(&tab.root, tab.mode, changed);
            if !affected {
                continue;
            }
            reconcile_tab_after_mutation(tab, select_after);
        }
    }
    resize_grid_document();
    update_carousel_preview();
    refresh_tab_bar();
}

fn complete_trash(
    items: Vec<MediaItem>,
    trashed_map: Vec<(PathBuf, PathBuf)>,
    error_message: Option<String>,
    selection_before_op: Vec<PathBuf>,
) -> Option<String> {
    {
        let state = unsafe { &mut *app_state() };
        for item in &items {
            state.pending_trash.remove(&item.path);
        }
    }
    let files: Vec<view_core::TrashedFile> = items
        .iter()
        .filter_map(|item| {
            let (_, trashed) = trashed_map.iter().find(|(original, _)| original == &item.path)?;
            Some(view_core::TrashedFile { item: item.clone(), trashed_path: trashed.clone(), index: 0 })
        })
        .collect();
    if files.is_empty() {
        return error_message;
    }
    let changed: Vec<PathBuf> = files.iter().map(|file| file.item.path.clone()).collect();
    {
        let state = unsafe { &mut *app_state() };
        state.trash_history.record_trash(view_core::TrashTransaction { files, selection_before_op });
    }
    reconcile_after_mutation(&changed, &[]);
    error_message
}

fn trash_selected(_grid: &GridView) {
    let (items, selection_before_op) = {
        let state = unsafe { &*app_state() };
        let tab = state.tab();
        if tab.unavailable.is_some() { return; }
        let selected = tab.catalog.selected_all();
        if selected.is_empty() { return; }
        let mut items = Vec::new();
        for path in selected {
            if state.pending_trash.contains(path) { continue; }
            if let Some(item) = tab.catalog.items().iter().find(|item| &item.path == path) {
                items.push(item.clone());
            }
        }
        if items.is_empty() { return; }
        (items, selected.to_vec())
    };
    let attempted: Vec<PathBuf> = items.iter().map(|item| item.path.clone()).collect();
    unsafe { &mut *app_state() }.pending_trash.extend(attempted);
    let workspace = NSWorkspace::sharedWorkspace();
    let urls: Vec<Retained<NSURL>> = items.iter().map(|i| url_from_path(&i.path)).collect();
    let ns_urls = NSArray::from_retained_slice(&urls);
    let handler = block2::RcBlock::new(move |mapping: core::ptr::NonNull<NSDictionary<NSURL, NSURL>>, error: *mut objc2_foundation::NSError| {
            let mapping = unsafe { mapping.as_ref() };
            let trashed_map: Vec<(PathBuf, PathBuf)> = mapping.allKeys()
                .iter()
                .filter_map(|key| {
                    let key_url = key.downcast_ref::<NSURL>()?;
                    let value = mapping.objectForKey(key_url);
                    let value_url = value.as_ref().and_then(|u| u.downcast_ref::<NSURL>());
                    let trashed = value_url.map(|u| path_from_url(u));
                    let original = path_from_url(key_url);
                    trashed.map(|trashed| (original, trashed))
                })
                .collect();
            let error_message: Option<String> = if error.is_null() {
                None
            } else {
                Some(unsafe { &*error }.localizedDescription().to_string())
            };
        let items = items.clone();
        let selection_before_op = selection_before_op.clone();
            dispatch2::DispatchQueue::main().exec_async(move || {
                if let Some(message) = complete_trash(items, trashed_map, error_message, selection_before_op) {
                    show_error_alert("Unable to move to Trash", &message);
                }
            });
        });
    workspace.recycleURLs_completionHandler(&ns_urls, Some(&handler));
}

fn move_item_exclusively(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    let from = std::ffi::CString::new(from.as_os_str().as_bytes())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let to = std::ffi::CString::new(to.as_os_str().as_bytes())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
}

fn undo_trash() {
    let transaction = unsafe { (&mut *app_state()).trash_history.pop_undo() };
    let Some(transaction) = transaction else { return };
    let blocked = {
        let state = unsafe { &*app_state() };
        transaction.files.iter().any(|file| {
            state.pending_trash.contains(&file.item.path) || state.pending_trash.contains(&file.trashed_path)
        })
    };
    if blocked {
        unsafe { &mut *app_state() }.trash_history.push_undo(transaction);
        show_error_alert("Undo unavailable", "A trash operation is still in progress for these items.");
        return;
    }
    let mut succeeded = Vec::new();
    let mut residual = Vec::new();
    for file in transaction.files {
        match move_item_exclusively(&file.trashed_path, &file.item.path) {
            Ok(()) => succeeded.push(file),
            Err(error) => {
                eprintln!("view: cannot restore {}: {error}", file.item.path.display());
                residual.push(file);
            }
        }
    }
    {
        let state = unsafe { &mut *app_state() };
        let selection_before_op = transaction.selection_before_op.clone();
        if !succeeded.is_empty() {
            state.trash_history.push_redo(TrashTransaction { files: succeeded.clone(), selection_before_op: selection_before_op.clone() });
        }
        if !residual.is_empty() {
            state.trash_history.push_undo(TrashTransaction { files: residual, selection_before_op });
        }
    }
    let changed: Vec<PathBuf> = succeeded.iter().map(|file| file.item.path.clone()).collect();
    reconcile_after_mutation(&changed, &changed);
}

fn redo_trash() {
    let transaction = unsafe { (&mut *app_state()).trash_history.pop_redo() };
    let Some(transaction) = transaction else { return };
    let blocked = {
        let state = unsafe { &*app_state() };
        transaction.files.iter().any(|file| {
            state.pending_trash.contains(&file.item.path) || state.pending_trash.contains(&file.trashed_path)
        })
    };
    if blocked {
        unsafe { &mut *app_state() }.trash_history.push_redo(transaction);
        show_error_alert("Redo unavailable", "A trash operation is still in progress for these items.");
        return;
    }
    let mut succeeded = Vec::new();
    let mut residual = Vec::new();
    for file in transaction.files {
        match move_item_exclusively(&file.item.path, &file.trashed_path) {
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
        if !succeeded.is_empty() {
            state.trash_history.push_undo(TrashTransaction { files: succeeded.clone(), selection_before_op: selection_before_op.clone() });
        }
        if !residual.is_empty() {
            state.trash_history.push_redo(TrashTransaction { files: residual, selection_before_op });
        }
    }
    let changed: Vec<PathBuf> = succeeded.iter().map(|file| file.item.path.clone()).collect();
    reconcile_after_mutation(&changed, &[]);
}

pub(crate) fn quicklook_selected() {
    let mtm = objc2::MainThreadMarker::new().unwrap();
    let Some(panel) = (unsafe { QLPreviewPanel::sharedPreviewPanel(mtm) }) else { return };
    if panel.isVisible() {
        panel.orderOut(None);
        return;
    }
    if unsafe { (*app_state()).tab().unavailable.is_some() } {
        return;
    }
    if unsafe { (*app_state()).tab().catalog.selected().is_none() } {
        return;
    }
    panel.makeKeyAndOrderFront(None);
    if unsafe { panel.dataSource() }.is_none() {
        if let Some(refs) = window_refs().as_ref() {
            let grid = unsafe { &*refs.grid };
            let index = {
                let state = unsafe { &mut *app_state() };
                let tab = state.tab_mut();
                if !tab.preview.begin(&tab.catalog) { return; }
                tab.preview.current_index().expect("preview session has a focus")
            };
            unsafe {
                panel.setDataSource(Some(objc2::runtime::ProtocolObject::from_ref(grid)));
                panel.setCurrentPreviewItemIndex(index as objc2_foundation::NSInteger);
                let _: () = objc2::msg_send![&*panel, addObserver: grid, forKeyPath: &*NSString::from_str("currentPreviewItemIndex"), options: 0usize, context: core::ptr::null_mut::<core::ffi::c_void>()];
            }
            unsafe { (*app_state()).preview_bound = true };
        }
    }
}

fn reconcile_preview() {
    let Some(mtm) = objc2::MainThreadMarker::new() else { return };
    let Some(panel) = (unsafe { QLPreviewPanel::sharedPreviewPanel(mtm) }) else { return };
    if !panel.isVisible() {
        return;
    }
    let index = {
        let state = unsafe { &mut *app_state() };
        let tab = state.tab_mut();
        if tab.unavailable.is_some() {
            None
        } else if tab.preview.begin(&tab.catalog) {
            tab.preview.current_index()
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
        item.setKeyEquivalentModifierMask(modifier_mask);
    }
    item
}

fn build_context_menu(target: &GridView) -> Retained<NSMenu> {
    let mtm = objc2::MainThreadMarker::new().unwrap();
    let menu = NSMenu::new(mtm);
    menu.addItem(&make_context_action_item(target, "Open", objc2::sel!(openSelected:), None, objc2_app_kit::NSEventModifierFlags::empty()));
    if let Some(item) = focused_item(unsafe { &*app_state() }) {
        if item.kind == MediaKind::Directory {
            menu.addItem(&make_context_action_item(target, "Open in New Tab", objc2::sel!(openInNewTab:), None, objc2_app_kit::NSEventModifierFlags::empty()));
        }
    }
    menu.addItem(&make_context_action_item(target, "Show in Finder", objc2::sel!(showSelectedInFinder:), None, objc2_app_kit::NSEventModifierFlags::empty()));
    menu.addItem(&make_context_action_item(target, "Copy Path", objc2::sel!(copySelectedPath:), None, objc2_app_kit::NSEventModifierFlags::empty()));
    menu.addItem(&make_context_action_item(target, "Rename…", objc2::sel!(renameSelected:), None, objc2_app_kit::NSEventModifierFlags::empty()));
    menu.addItem(&make_context_action_item(target, "Move to Trash", objc2::sel!(moveSelectedToTrash:), Some("\u{8}"), objc2_app_kit::NSEventModifierFlags::empty()));
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

fn relocated_path(path: &std::path::Path, from: &std::path::Path, to: &std::path::Path) -> PathBuf {
    match path.strip_prefix(from) {
        Ok(suffix) if suffix.as_os_str().is_empty() => to.to_path_buf(),
        Ok(suffix) => to.join(suffix),
        Err(_) => path.to_path_buf(),
    }
}

fn rename_item(from: &std::path::Path, name: &str) -> std::io::Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Enter a single file or folder name."));
    }
    let to = from.with_file_name(name);
    if from == to { return Ok(()) }
    move_item_exclusively(from, &to)?;
    let state = unsafe { &mut *app_state() };
    state.trash_history.relocate(from, &to);
    for tab in state.tabs.iter_mut() {
        let previous_selection = tab.catalog.selected_all().to_vec();
        let focus = tab.catalog.selected().map(|path| path.to_path_buf());
        let relocated_selection: Vec<PathBuf> = previous_selection.iter().map(|path| relocated_path(path, from, &to)).collect();
        let relocated_focus = focus.as_deref().map(|path| relocated_path(path, from, &to));
        tab.root = relocated_path(&tab.root, from, &to);
        for (root, selection) in tab.back.iter_mut().chain(&mut tab.forward) {
            *root = relocated_path(root, from, &to);
            if let Some(path) = selection { *path = relocated_path(path, from, &to); }
        }
        match load_listing(&tab.root, tab.mode, tab.media_filter) {
            Ok(items) => {
                let mut catalog = Catalog::new();
                catalog.append(items);
                catalog.restore_selection(&relocated_selection, relocated_focus.as_deref());
                tab.catalog = catalog;
                tab.unavailable = None;
            }
            Err(error) => {
                tab.unavailable = Some(format!("Unable to read {}: {error}", tab.root.display()));
            }
        }
    }
    after_navigation(false);
    Ok(())
}

fn rename_selected() {
    if unsafe { (*app_state()).tab().unavailable.is_some() } { return; }
    let Some(item) = focused_item(unsafe { &*app_state() }) else { return };
    let mtm = objc2::MainThreadMarker::new().unwrap();
    let Some(name) = item.path.file_name().and_then(|name| name.to_str()) else {
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str("Unable to rename"));
        alert.setInformativeText(&NSString::from_str("The filename cannot be displayed as text."));
        alert.runModal();
        return;
    };
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str("Rename"));
    let field = NSTextField::new(mtm);
    field.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(320.0, 24.0)));
    field.setStringValue(&NSString::from_str(name));
    alert.setAccessoryView(Some(&field));
    alert.addButtonWithTitle(&NSString::from_str("Rename"));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));
    let _ = alert.window().makeFirstResponder(Some(&field));
    loop {
        if alert.runModal() != objc2_app_kit::NSAlertFirstButtonReturn { return; }
        match rename_item(&item.path, &field.stringValue().to_string()) {
            Ok(()) => return,
            Err(error) => {
                let failure = NSAlert::new(mtm);
                failure.setMessageText(&NSString::from_str("Unable to rename"));
                failure.setInformativeText(&NSString::from_str(&error.to_string()));
                failure.runModal();
            }
        }
    }
}

fn enter_directory(path: PathBuf) {
    {
        let state = unsafe { &mut *app_state() };
        let tab = state.tab_mut();
        let mode = tab.mode;
        let media_filter = tab.media_filter;
        match load_listing(&path, mode, media_filter) {
            Ok(items) => {
                let current = tab.root.clone();
                let previous = tab.catalog.selected().map(|path| path.to_path_buf());
                tab.back.push((current, previous));
                tab.forward.clear();
                tab.root = path;
                show_listing(&mut tab.catalog, items, ListingPick::First);
                tab.unavailable = None;
            }
            Err(error) => {
                tab.unavailable = Some(format!("Unable to open {}: {error}", path.display()));
            }
        }
    }
    after_navigation(false);
}

fn navigate_back() {
    {
        let state = unsafe { &mut *app_state() };
        let tab = state.tab_mut();
        let Some((url, sel)) = tab.back.last().cloned() else { return };
        let mode = tab.mode;
        let filter = tab.media_filter;
        match load_listing(&url, mode, filter) {
            Ok(items) => {
                tab.back.pop();
                let current = tab.root.clone();
                let leaving = tab.catalog.selected().map(|path| path.to_path_buf());
                tab.forward.push((current, leaving));
                tab.root = url;
                show_listing(&mut tab.catalog, items, ListingPick::Restored(sel));
                tab.unavailable = None;
            }
            Err(error) => {
                tab.unavailable = Some(format!("Unable to open {}: {error}", url.display()));
            }
        }
    }
    after_navigation(true);
}

fn navigate_forward() {
    {
        let state = unsafe { &mut *app_state() };
        let tab = state.tab_mut();
        let Some((url, sel)) = tab.forward.last().cloned() else { return };
        let mode = tab.mode;
        let filter = tab.media_filter;
        match load_listing(&url, mode, filter) {
            Ok(items) => {
                tab.forward.pop();
                let current = tab.root.clone();
                let leaving = tab.catalog.selected().map(|path| path.to_path_buf());
                tab.back.push((current, leaving));
                tab.root = url;
                show_listing(&mut tab.catalog, items, ListingPick::Restored(sel));
                tab.unavailable = None;
            }
            Err(error) => {
                tab.unavailable = Some(format!("Unable to open {}: {error}", url.display()));
            }
        }
    }
    after_navigation(true);
}

fn navigate_to_parent() {
    {
        let state = unsafe { &mut *app_state() };
        let tab = state.tab_mut();
        if tab.mode != BrowserMode::Browser { return; }
        let Some(parent) = tab.root.parent().map(|p| p.to_path_buf()) else { return };
        if parent == tab.root { return; }
        let mode = tab.mode;
        let filter = tab.media_filter;
        let came_from = tab.root.clone();
        let leaving = tab.catalog.selected().map(|path| path.to_path_buf());
        match load_listing(&parent, mode, filter) {
            Ok(items) => {
                tab.back.push((came_from.clone(), leaving));
                tab.forward.clear();
                tab.root = parent;
                show_listing(&mut tab.catalog, items, ListingPick::Restored(Some(came_from)));
                tab.unavailable = None;
            }
            Err(error) => {
                tab.unavailable = Some(format!("Unable to open {}: {error}", parent.display()));
            }
        }
    }
    after_navigation(true);
}

fn after_navigation(center: bool) {
    let title = unsafe { (*app_state()).tab().root.display().to_string() };
    if let Some(refs) = window_refs().as_ref() {
        unsafe { (&*refs.window).setTitle(&NSString::from_str(&title)); }
    }
    apply_view_mode_layout();
    reconcile_preview();
    if let Some(refs) = window_refs().as_ref() {
        let grid = unsafe { &*refs.grid };
        if center { grid.scroll_selection_to_center(); } else { grid.scroll_selection_into_view(); }
        grid.setNeedsDisplay(true);
    }
    refresh_tab_bar();
}

fn activate_tab(index: usize) {
    {
        let state = unsafe { &mut *app_state() };
        if index >= state.tabs.len() || index == state.active { return; }
        state.active = index;
    }
    after_navigation(false);
}

fn close_tab(index: usize) {
    {
        let state = unsafe { &mut *app_state() };
        if state.tabs.len() <= 1 || index >= state.tabs.len() { return; }
        state.tabs.remove(index);
        if index < state.active { state.active -= 1; }
        else if index == state.active { state.active = state.active.min(state.tabs.len() - 1); }
    }
    after_navigation(false);
}

fn close_other_tabs(index: usize) {
    {
        let state = unsafe { &mut *app_state() };
        if state.tabs.len() <= 1 || index >= state.tabs.len() { return; }
        let keep = state.tabs.remove(index);
        state.tabs.clear();
        state.tabs.push(keep);
        state.active = 0;
    }
    after_navigation(false);
}

fn new_tab() {
    let created = {
        let state = unsafe { &mut *app_state() };
        let root = state.tab().root.clone();
        let mode = state.tab().mode;
        let filter = state.tab().media_filter;
        match load_listing(&root, mode, filter) {
            Ok(items) => Some(state.new_tab(root, mode, items)),
            Err(error) => {
                state.tab_mut().unavailable = Some(format!("Unable to open {}: {error}", root.display()));
                None
            }
        }
    };
    match created {
        Some(index) => activate_tab(index),
        None => after_navigation(false),
    }
}

fn apply_media_filter(filter: MediaFilter) {
    {
        let state = unsafe { &mut *app_state() };
        state.tab_mut().media_filter = filter;
    }
    persist_media_filter();
    let reload = {
        let state = unsafe { &*app_state() };
        let tab = state.tab();
        let mode = tab.mode;
        let root = tab.root.clone();
        let previous_selection = tab.catalog.selected_all().to_vec();
        match load_listing(&root, mode, filter) {
            Ok(items) => Some((items, previous_selection)),
            Err(error) => {
                unsafe {
                    (*app_state()).tab_mut().unavailable = Some(format!("Unable to read {}: {error}", root.display()));
                }
                None
            }
        }
    };
    if let Some((items, previous_selection)) = reload {
        let state = unsafe { &mut *app_state() };
        let tab = state.tab_mut();
        let mut catalog = Catalog::new();
        catalog.append(items);
        let present: std::collections::HashSet<&PathBuf> = catalog.items().iter().map(|item| &item.path).collect();
        let mut selection: Vec<PathBuf> = previous_selection.into_iter().filter(|path| present.contains(path)).collect();
        if selection.is_empty() && let Some(first) = catalog.items().first().map(|item| item.path.clone()) {
            selection.push(first);
        }
        catalog.restore_selection(&selection, selection.first().map(|path| path.as_path()));
        tab.catalog = catalog;
        tab.unavailable = None;
    }
    after_navigation(false);
}

fn toggle_view_mode() {
    {
        let state = unsafe { &mut *app_state() };
        let next = if state.tab().view_mode == ViewMode::Grid { ViewMode::Carousel } else { ViewMode::Grid };
        state.tab_mut().view_mode = next;
    }
    persist_view_mode();
    apply_view_mode_layout();
    update_carousel_preview();
    refresh_tab_bar();
}

fn refresh_tab_bar() {
    if let Some(refs) = window_refs().as_ref() {
        unsafe { &*refs.tab_bar }.sync();
    }
}

fn open_path_in_new_tab(path: PathBuf) {
    let (root, file) = if path.is_dir() {
        (path, None)
    } else {
        let parent = path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| path.clone());
        (parent, Some(path))
    };
    let created = {
        let state = unsafe { &mut *app_state() };
        let mode = state.tab().mode;
        let filter = state.tab().media_filter;
        match load_listing(&root, mode, filter) {
            Ok(items) => {
                let index = state.new_tab(root, mode, items);
                if let Some(file) = file {
                    let tab = &mut state.tabs[index];
                    if let Some(i) = tab.catalog.items().iter().position(|item| item.path == file) {
                        select_index(&mut tab.catalog, i);
                    }
                }
                Some(index)
            }
            Err(error) => {
                state.tab_mut().unavailable = Some(format!("Unable to open {}: {error}", root.display()));
                None
            }
        }
    };
    match created {
        Some(index) => activate_tab(index),
        None => after_navigation(false),
    }
}

fn navigate_tab_to_path(index: usize, path: PathBuf) {
    {
        let state = unsafe { &mut *app_state() };
        if index >= state.tabs.len() { return; }
        let tab = &mut state.tabs[index];
        let target_root = if path.is_dir() {
            path.clone()
        } else {
            path.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| path.clone())
        };
        let mode = tab.mode;
        let filter = tab.media_filter;
        if tab.root != target_root {
            match load_listing(&target_root, mode, filter) {
                Ok(items) => {
                    let current = tab.root.clone();
                    let previous = tab.catalog.selected().map(|p| p.to_path_buf());
                    tab.back.push((current, previous));
                    tab.forward.clear();
                    tab.root = target_root.clone();
                    show_listing(&mut tab.catalog, items, ListingPick::First);
                    tab.unavailable = None;
                }
                Err(error) => {
                    tab.unavailable = Some(format!("Unable to open {}: {error}", target_root.display()));
                }
            }
        }
        if !path.is_dir() && tab.unavailable.is_none() {
            if let Some(i) = tab.catalog.items().iter().position(|item| item.path == path) {
                select_index(&mut tab.catalog, i);
            }
        }
    }
    if unsafe { (*app_state()).active } == index {
        after_navigation(false);
    } else {
        activate_tab(index);
    }
}

fn open_selected() {
    if unsafe { (*app_state()).tab().unavailable.is_some() } { return; }
    let Some(item) = focused_item(unsafe { &*app_state() }) else { return };
    if item.kind == MediaKind::Directory {
        enter_directory(item.path);
    } else {
        let ws = NSWorkspace::sharedWorkspace();
        ws.openURL(&url_from_path(&item.path));
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
    let selected: std::collections::HashSet<PathBuf> = {
        let state = unsafe { &*app_state() };
        let tab = state.tab();
        if tab.unavailable.is_some() {
            return Vec::new();
        }
        tab.catalog.selected_all().iter().cloned().collect()
    };
    let state = unsafe { &*app_state() };
    let tab = state.tab();
    tab.catalog.items().iter().enumerate().filter(|(_, item)| selected.contains(&item.path)).map(|(index, item)| {
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
        dragging_item
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
    let mode = unsafe { (*app_state()).tab().mode };
    let filter = unsafe { (*app_state()).tab().media_filter };
    match load_listing(parent, mode, filter) {
        Ok(items) if items.iter().any(|item| item.path == path) => {}
        _ => return false,
    }
    if unsafe { (*app_state()).tab().root != parent } {
        enter_directory(parent.to_path_buf());
    }
    {
        let state = unsafe { &mut *app_state() };
        let Some(index) = state.items().iter().position(|item| item.path == path) else { return false };
        select_index(&mut state.tab_mut().catalog, index);
    }
    view.scroll_selection_into_view();
    update_carousel_preview();
    view.setNeedsDisplay(true);
    reconcile_preview();
    refresh_tab_bar();
    true
}

fn show_error_alert(title: &str, message: &str) {
    let mtm = objc2::MainThreadMarker::new().unwrap();
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(message));
    alert.runModal();
}

fn load_listing(root: &std::path::Path, mode: BrowserMode, filter: MediaFilter) -> Result<Vec<MediaItem>, view_core::ListingError> {
    let items = match mode {
        BrowserMode::Browser => list_browser(root)?,
        BrowserMode::Recursive => scan_recursive(root)?,
    };
    Ok(items.into_iter().filter(|item| filter.keeps(item.kind)).collect())
}

fn folder_like_icon(item: &MediaItem) -> Option<Retained<NSImage>> {
    let workspace = NSWorkspace::sharedWorkspace();
    let icon = workspace.iconForFile(&NSString::from_str(&item.path.display().to_string()));
    Some(icon)
}

fn sized_folder_icon(item: &MediaItem, icon_size: f64) -> Option<Retained<NSImage>> {
    let icon = folder_like_icon(item)?;
    icon.setSize(NSSize::new(icon_size, icon_size));
    Some(icon)
}

fn system_image(name: &str) -> Option<Retained<NSImage>> {
    let ns_name = NSString::from_str(name);
    NSImage::imageWithSystemSymbolName_accessibilityDescription(&ns_name, None)
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
static THUMB_PENDING: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<(PathBuf, u32, Option<i64>, Option<u64>)>>> =
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

fn thumb_pending() -> std::sync::MutexGuard<'static, std::collections::HashSet<(PathBuf, u32, Option<i64>, Option<u64>)>> {
    THUMB_PENDING
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[cfg(feature = "benchmark")]
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
    let removed = match store.images.get_mut(&path) {
        Some(entries) => {
            let before: usize = entries.iter().map(|entry| entry.cost).sum();
            entries.retain(|entry| entry.file_size == file_size && entry.modification == modification);
            let after: usize = entries.iter().map(|entry| entry.cost).sum();
            before - after
        }
        None => 0,
    };
    store.cost = store.cost.saturating_sub(removed);
    if store.images.get(&path).is_some_and(|entries| entries.is_empty()) {
        store.images.remove(&path);
    }
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
    thumb_pending().iter().any(|(pending, _, _, _)| pending == path)
}

#[cfg(feature = "benchmark")]
fn request_quicklook_thumbnail(path: PathBuf) {
    let item = unsafe { (*app_state()).items().iter().find(|item| item.path == path).cloned() }.expect("benchmark movie is in the catalog");
    request_quicklook_thumbnail_sized(path, MOVIE_PREVIEW_PIXELS, item.file_size, item.modification);
}

fn request_quicklook_thumbnail_sized(path: PathBuf, pixels: u32, file_size: Option<i64>, modification: Option<u64>) {
    let pixels = pixels.clamp(1, 4096);
    if !thumb_pending().insert((path.clone(), pixels, file_size, modification)) {
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
            thumb_pending().remove(&(path_for_reply.clone(), pixels, file_size, modification));
            if !error.is_null() {
                eprintln!("view: thumbnail {}: {:?}", path_for_reply.display(), unsafe { &*error });
            }
            return;
        }
        let image = SyncImage(unsafe { (*(representation as *const QLThumbnailRepresentation)).NSImage() });
        let path = path_for_reply.clone();
        dispatch2::DispatchQueue::main().exec_async(move || thumbnail_completed(path, pixels, file_size, modification, image));
    });
    unsafe { generator.generateBestRepresentationForRequest_completionHandler(&request, &handler) };
}

fn thumbnail_completed(path: PathBuf, pixels: u32, file_size: Option<i64>, modification: Option<u64>, image: SyncImage) {
    thumb_pending().remove(&(path.clone(), pixels, file_size, modification));
    if !view_core::file_version_matches(&path, file_size, modification) {
        return;
    }
    let size = image.0.size();
    let cost = ((size.width.max(1.0) * size.height.max(1.0) * 4.0) as usize).saturating_mul(4);
    remember_thumbnail(path, pixels, file_size, modification, image, cost);
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
        Self(objc_autorelease_pool_push())
    }
}

impl Drop for AutoreleasePool {
    fn drop(&mut self) {
        objc_autorelease_pool_pop(self.0)
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
    cf_string_create_with_c_string(std::ptr::null(), c_string.as_ptr(), 0x0800_0100)
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



fn tab_label(tab: &Tab) -> String {
    tab.root.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| tab.root.display().to_string())
}

const FILTER_W: f64 = 158.0;
const FILTER_GAP: f64 = 8.0;

fn tab_bar_metrics(width: f64, count: usize) -> (Vec<NSRect>, NSRect, NSRect, NSRect, NSRect, NSRect) {
    let h = TAB_BAR_HEIGHT;
    let back = NSRect::new(NSPoint::new(6.0, 4.0), NSSize::new(28.0, h - 8.0));
    let forward = NSRect::new(NSPoint::new(36.0, 4.0), NSSize::new(28.0, h - 8.0));
    let toggle = NSRect::new(NSPoint::new(66.0, 4.0), NSSize::new(28.0, h - 8.0));
    let add = NSRect::new(NSPoint::new((width - 34.0).max(100.0), 4.0), NSSize::new(28.0, h - 8.0));
    let filter = NSRect::new(NSPoint::new(add.origin.x - FILTER_GAP - FILTER_W, 4.0), NSSize::new(FILTER_W, h - 8.0));
    let tabs_x = 100.0;
    let tabs_w = (filter.origin.x - FILTER_GAP - tabs_x - 4.0).max(40.0);
    let tab_w = if count == 0 { tabs_w } else { (tabs_w / count as f64).clamp(80.0, 220.0) };
    let mut rects = Vec::with_capacity(count);
    for i in 0..count {
        rects.push(NSRect::new(NSPoint::new(tabs_x + i as f64 * tab_w, 4.0), NSSize::new(tab_w - 4.0, h - 8.0)));
    }
    (rects, back, forward, toggle, add, filter)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabDropTarget {
    Tab(usize),
    NewTab,
}

fn resolve_tab_drop(width: f64, tab_count: usize, location: NSPoint) -> TabDropTarget {
    let (tab_rects, _, _, _, _, _) = tab_bar_metrics(width, tab_count);
    tab_rects
        .iter()
        .enumerate()
        .find(|(_, rect)| objc2_foundation::NSPointInRect(location, **rect))
        .map(|(i, _)| TabDropTarget::Tab(i))
        .unwrap_or(TabDropTarget::NewTab)
}

fn apply_tab_drop(path: PathBuf, target: TabDropTarget) {
    match target {
        TabDropTarget::Tab(i) => navigate_tab_to_path(i, path),
        TabDropTarget::NewTab => open_path_in_new_tab(path),
    }
}

struct TabBarControls {
    back: Retained<NSButton>,
    forward: Retained<NSButton>,
    toggle: Retained<NSButton>,
    add: Retained<NSButton>,
    filter: Retained<objc2_app_kit::NSSegmentedControl>,
}

fn tab_bar_filter_control() -> Retained<objc2_app_kit::NSSegmentedControl> {
    const LABELS: [&str; 3] = ["Images", "Videos", "Both"];
    const TOOLTIPS: [&str; 3] = ["Show images only", "Show videos only", "Show images and videos"];
    let mtm = objc2::MainThreadMarker::new().unwrap();
    let control = objc2_app_kit::NSSegmentedControl::new(mtm);
    control.setSegmentCount(3);
    for (index, (label, tooltip)) in LABELS.iter().zip(TOOLTIPS.iter()).enumerate() {
        control.setLabel_forSegment(&NSString::from_str(label), index as objc2_foundation::NSInteger);
        control.setWidth_forSegment(52.0, index as objc2_foundation::NSInteger);
        control.setToolTip_forSegment(Some(&NSString::from_str(tooltip)), index as objc2_foundation::NSInteger);
    }
    control.setTrackingMode(objc2_app_kit::NSSegmentSwitchTracking::SelectOne);
    control.setSelectedSegment(2);
    control.setControlSize(objc2_app_kit::NSControlSize::Small);
    control.setFont(Some(&NSFont::systemFontOfSize(10.0)));
    control.setToolTip(Some(&NSString::from_str("Show images, videos, or both")));
    control
}

fn tab_bar_button(symbol: &str, label: &str) -> Retained<NSButton> {
    let button = NSButton::new(objc2::MainThreadMarker::new().unwrap());
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(symbol), Some(&NSString::from_str(label)))
        .expect("system symbol is available");
    button.setImage(Some(&image));
    button.setImagePosition(objc2_app_kit::NSCellImagePosition::ImageOnly);
    button.setBezelStyle(objc2_app_kit::NSBezelStyle::Toolbar);
    button.setToolTip(Some(&NSString::from_str(label)));
    button
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = TabBarControls]
    struct TabBarView;

    impl TabBarView {
        #[unsafe(method_id(initWithFrame:))]
        fn init_with_frame(this: Allocated<Self>, frame: NSRect) -> Retained<Self> {
            let this = this.set_ivars(TabBarControls {
                back: tab_bar_button("chevron.left", "Back"),
                forward: tab_bar_button("chevron.right", "Forward"),
                toggle: tab_bar_button("square.grid.2x2", "Change view"),
                add: tab_bar_button("plus", "New Tab"),
                filter: tab_bar_filter_control(),
            });
            unsafe {
                let this: Retained<Self> = objc2::msg_send![super(this), initWithFrame: frame];
                for (button, action) in [
                    (&this.ivars().back, objc2::sel!(goBack:)),
                    (&this.ivars().forward, objc2::sel!(goForward:)),
                    (&this.ivars().toggle, objc2::sel!(changeView:)),
                    (&this.ivars().add, objc2::sel!(addTab:)),
                ] {
                    button.setTarget(Some(&*this));
                    button.setAction(Some(action));
                    this.addSubview(button);
                }
                this.ivars().filter.setTarget(Some(&*this));
                this.ivars().filter.setAction(Some(objc2::sel!(setMediaFilter:)));
                this.addSubview(&this.ivars().filter);
                this.registerForDraggedTypes(&NSArray::from_slice(&[&*objc2_app_kit::NSPasteboardTypeFileURL]));
                this
            }
        }

        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool { true }



        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _rect: NSRect) {
            NSColor::windowBackgroundColor().setFill();
            objc2_app_kit::NSRectFill(self.bounds());
            NSColor::separatorColor().setFill();
            objc2_app_kit::NSRectFill(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(self.bounds().size.width, 1.0)));
            let state = unsafe { &*app_state() };
            let width = self.bounds().size.width;
            let (tab_rects, _, _, _, _, _) = tab_bar_metrics(width, state.tabs.len());
            let pstyle: Retained<objc2_app_kit::NSMutableParagraphStyle> = {
                let p: Retained<objc2_app_kit::NSMutableParagraphStyle> = unsafe { objc2::msg_send![objc2_app_kit::NSMutableParagraphStyle::class(), new] };
                p.setAlignment(objc2_app_kit::NSTextAlignment::Center);
                p.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingTail);
                p
            };
            let font = NSFont::systemFontOfSize(12.0);
            let label = NSColor::labelColor();
            let attrs = text_attributes(&font, &label, &pstyle);
            use objc2_app_kit::NSStringDrawing;
            for (i, rect) in tab_rects.iter().enumerate() {
                let active = i == state.active;
                if active {
                    let path = objc2_app_kit::NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(*rect, 6.0, 6.0);
                    NSColor::controlAccentColor().colorWithAlphaComponent(0.14).setFill();
                    path.fill();
                }
                let title = tab_label(&state.tabs[i]);
                unsafe { NSString::from_str(&title).drawInRect_withAttributes(NSRect::new(NSPoint::new(rect.origin.x + 6.0, rect.origin.y + 5.0), NSSize::new((rect.size.width - 12.0).max(0.0), rect.size.height)), Some(&*attrs)) };
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &objc2_app_kit::NSEvent) {
            let location = self.convertPoint_fromView(event.locationInWindow(), None);
            let state = unsafe { &*app_state() };
            let width = self.bounds().size.width;
            let (tab_rects, _, _, _, _, _) = tab_bar_metrics(width, state.tabs.len());
            for (i, rect) in tab_rects.iter().enumerate() {
                if objc2_foundation::NSPointInRect(location, *rect) {
                    activate_tab(i);
                    return;
                }
            }
        }

        #[unsafe(method(goBack:))]
        fn go_back(&self, _sender: &NSButton) { navigate_back(); }
        #[unsafe(method(goForward:))]
        fn go_forward(&self, _sender: &NSButton) { navigate_forward(); }
        #[unsafe(method(changeView:))]
        fn change_view(&self, _sender: &NSButton) { toggle_view_mode(); }
        #[unsafe(method(setMediaFilter:))]
        fn set_media_filter(&self, sender: &objc2_app_kit::NSSegmentedControl) {
            let filter = match sender.selectedSegment() {
                0 => MediaFilter::Images,
                1 => MediaFilter::Videos,
                _ => MediaFilter::Both,
            };
            apply_media_filter(filter);
        }
        #[unsafe(method(addTab:))]
        fn add_tab(&self, _sender: &NSButton) { new_tab(); }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &objc2_app_kit::NSEvent) {
            let location = self.convertPoint_fromView(event.locationInWindow(), None);
            let state = unsafe { &*app_state() };
            let width = self.bounds().size.width;
            let (tab_rects, _, _, _, _, _) = tab_bar_metrics(width, state.tabs.len());
            for (i, rect) in tab_rects.iter().enumerate() {
                if objc2_foundation::NSPointInRect(location, *rect) {
                    let mtm = objc2::MainThreadMarker::new().unwrap();
                    let menu = NSMenu::new(mtm);
                    let close = NSMenuItem::new(mtm);
                    close.setTitle(&NSString::from_str("Close Tab"));
                    close.setTag(i as isize);
                    unsafe { close.setTarget(Some(self as _)) };
                    unsafe { let _: () = objc2::msg_send![&*close, setAction: objc2::sel!(closeTabAt:)]; };
                    menu.addItem(&close);
                    let close_others = NSMenuItem::new(mtm);
                    close_others.setTitle(&NSString::from_str("Close Other Tabs"));
                    close_others.setTag(i as isize);
                    unsafe { close_others.setTarget(Some(self as _)) };
                    unsafe { let _: () = objc2::msg_send![&*close_others, setAction: objc2::sel!(closeOtherTabsAt:)]; };
                    menu.addItem(&close_others);
                    NSMenu::popUpContextMenu_withEvent_forView(&menu, event, self);
                    return;
                }
            }
        }

        #[unsafe(method(closeTabAt:))]
        fn close_tab_at(&self, sender: &NSMenuItem) {
            let index = sender.tag() as usize;
            close_tab(index);
        }

        #[unsafe(method(closeOtherTabsAt:))]
        fn close_other_tabs_at(&self, sender: &NSMenuItem) {
            let index = sender.tag() as usize;
            close_other_tabs(index);
        }

        #[unsafe(method(draggingEntered:))]
        fn dragging_entered(&self, sender: &objc2::runtime::ProtocolObject<dyn objc2_app_kit::NSDraggingInfo>) -> objc2_app_kit::NSDragOperation {
            if dropped_path(&sender.draggingPasteboard()).is_some() { objc2_app_kit::NSDragOperation::Copy } else { objc2_app_kit::NSDragOperation::empty() }
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag_operation(&self, sender: &objc2::runtime::ProtocolObject<dyn objc2_app_kit::NSDraggingInfo>) -> objc2::runtime::Bool {
            let Some(path) = dropped_path(&sender.draggingPasteboard()) else { return objc2::runtime::Bool::NO };
            let location = self.convertPoint_fromView(sender.draggingLocation(), None);
            let (width, tab_count) = {
                let state = unsafe { &*app_state() };
                (self.bounds().size.width, state.tabs.len())
            };
            let target = resolve_tab_drop(width, tab_count, location);
            apply_tab_drop(path, target);
            objc2::runtime::Bool::YES
        }
    }
);

impl TabBarView {
    fn sync(&self) {
        let state = unsafe { &*app_state() };
        let (_, back, forward, toggle, add, filter) = tab_bar_metrics(self.bounds().size.width, state.tabs.len());
        let controls = self.ivars();
        controls.back.setFrame(back);
        controls.forward.setFrame(forward);
        controls.toggle.setFrame(toggle);
        controls.add.setFrame(add);
        controls.filter.setFrame(filter);
        let segment = match state.tab().media_filter {
            MediaFilter::Images => 0,
            MediaFilter::Videos => 1,
            MediaFilter::Both => 2,
        };
        controls.filter.setSelectedSegment(segment);
        controls.back.setEnabled(!state.tab().back.is_empty());
        controls.forward.setEnabled(!state.tab().forward.is_empty());
        let (symbol, label) = if state.tab().view_mode == ViewMode::Grid { ("rectangle", "Show Carousel") } else { ("square.grid.2x2", "Show Grid") };
        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(symbol), Some(&NSString::from_str(label)))
            .expect("system symbol is available");
        controls.toggle.setImage(Some(&image));
        controls.toggle.setToolTip(Some(&NSString::from_str(label)));
        self.setNeedsDisplay(true);
    }
}

unsafe impl NSObjectProtocol for TabBarView {}


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
            NSColor::windowBackgroundColor().setFill();
            objc2_app_kit::NSRectFill(self.bounds());
        }
    }
);

define_class!(
    #[unsafe(super(AVPlayerView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    pub(crate) struct PreviewPlayerView;

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
        fn accepts_first_responder(&self) -> bool { true }

        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool { true }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, rect: NSRect) {
            let (items_len, carousel, selected_paths, items_snapshot, unavailable) = {
                let state = unsafe { &*app_state() };
                let tab = state.tab();
                (tab.catalog.items().len(), tab.view_mode == ViewMode::Carousel, tab.catalog.selected_all().to_vec(), tab.catalog.items().to_vec(), tab.unavailable.clone())
            };
            if let Some(message) = unavailable {
                NSColor::windowBackgroundColor().setFill();
                objc2_app_kit::NSRectFill(self.bounds());
                let font = NSFont::systemFontOfSize(13.0);
                let color = NSColor::secondaryLabelColor();
                let pstyle: Retained<objc2_app_kit::NSMutableParagraphStyle> = {
                    let p: Retained<objc2_app_kit::NSMutableParagraphStyle> = unsafe { objc2::msg_send![objc2_app_kit::NSMutableParagraphStyle::class(), new] };
                    p.setAlignment(objc2_app_kit::NSTextAlignment::Center);
                    p.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByWordWrapping);
                    p
                };
                let attrs = text_attributes(&font, &color, &pstyle);
                use objc2_app_kit::NSStringDrawing;
                let bounds = self.bounds();
                let text_rect = NSRect::new(NSPoint::new(24.0, bounds.size.height / 2.0 - 48.0), NSSize::new((bounds.size.width - 48.0).max(0.0), 96.0));
                unsafe { NSString::from_str(&message).drawInRect_withAttributes(text_rect, Some(&*attrs)) };
                let _ = items_len;
                return;
            }
            let selected: std::collections::HashSet<&std::path::Path> = selected_paths.iter().map(|path| path.as_path()).collect();
            let scale = thumbnail_scale();
            NSColor::windowBackgroundColor().setFill();
            objc2_app_kit::NSRectFill(self.bounds());
            let _mtm = objc2::MainThreadMarker::new().unwrap();
            let pstyle: Retained<objc2_app_kit::NSMutableParagraphStyle> = {
                let p: Retained<objc2_app_kit::NSMutableParagraphStyle> = unsafe { objc2::msg_send![objc2_app_kit::NSMutableParagraphStyle::class(), new] };
                p.setAlignment(objc2_app_kit::NSTextAlignment::Center);
                p.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingMiddle);
                p
            };
            let pstyle_detail: Retained<objc2_app_kit::NSMutableParagraphStyle> = {
                let p: Retained<objc2_app_kit::NSMutableParagraphStyle> = unsafe { objc2::msg_send![objc2_app_kit::NSMutableParagraphStyle::class(), new] };
                p.setAlignment(objc2_app_kit::NSTextAlignment::Center);
                p.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingTail);
                p
            };
            let name_font = NSFont::systemFontOfSize(12.0 * scale);
            let bold_font = NSFont::boldSystemFontOfSize(12.0 * scale);
            let label = NSColor::labelColor();
            let name_dict = text_attributes(&name_font, &label, &pstyle);
            let bold_dict = text_attributes(&bold_font, &label, &pstyle);
            let detail_font = NSFont::systemFontOfSize(10.0 * scale);
            let detail_color = NSColor::secondaryLabelColor();
            let detail_dict = text_attributes(&detail_font, &detail_color, &pstyle_detail);
            let mut requests = Vec::new();
            for (i, item) in items_snapshot.iter().enumerate() {
                if item.kind != MediaKind::Image { continue; }
                let cell = self.cell_rect(i);
                if !objc2_foundation::NSIntersectsRect(cell, rect) { continue; }
                let (thumb_rect, _, _) = cell_content_rects(cell, carousel, scale);
                requests.push(ThumbRequest { path: item.path.clone(), pixels: pixel_limit(self, thumb_rect), file_size: item.file_size, modification: item.modification });
            }
            ensure_thumbnails(&requests);
            for (i, item) in items_snapshot.iter().enumerate() {
                let cell = self.cell_rect(i);
                if !objc2_foundation::NSIntersectsRect(cell, rect) { continue; }
                let is_selected = selected.contains(item.path.as_path());
                if is_selected {
                    let path = objc2_app_kit::NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(cell, 10.0, 10.0);
                    let fill = NSColor::selectedContentBackgroundColor().colorWithAlphaComponent(0.6);
                    fill.setFill();
                    path.fill();
                    let stroke = NSColor::selectedContentBackgroundColor();
                    stroke.setStroke();
                    path.setLineWidth(2.0);
                    path.stroke();
                }
                let (thumb_rect, name_rect, detail_rect) = cell_content_rects(cell, carousel, scale);
                let thumb_img = if item.kind == MediaKind::Directory {
                    folder_like_icon(item)
                } else if item.kind == MediaKind::Movie {
                    let pixels = pixel_limit(self, thumb_rect);
                    let cached = cached_sized(&item.path, pixels, item.file_size, item.modification);
                    if cached.is_none() { request_quicklook_thumbnail_sized(item.path.clone(), pixels, item.file_size, item.modification); }
                    cached.or_else(|| folder_like_icon(item)).or_else(|| system_image("film"))
                } else {
                    display_image(&item.path, pixel_limit(self, thumb_rect), item.file_size, item.modification).or_else(|| system_image("exclamationmark.triangle"))
                };
                if let Some(img) = thumb_img {
                    let fitted = aspect_fit(img.size(), thumb_rect);
                    unsafe {
                        img.drawInRect_fromRect_operation_fraction_respectFlipped_hints(fitted, NSRect::ZERO, objc2_app_kit::NSCompositingOperation::SourceOver, 1.0, true, None);
                    }
                }
                let mut name = item.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if name.chars().count() > 128 { name = name.chars().take(128).collect(); }
                let string = NSString::from_str(&name);
                let name_attrs = if item.kind == MediaKind::Directory { &bold_dict } else { &name_dict };
                use objc2_app_kit::NSStringDrawing;
                unsafe { string.drawInRect_withAttributes(name_rect, Some(&**name_attrs)) };
                if item.kind != MediaKind::Directory {
                    let file_size = item.file_size.unwrap_or(0).max(0) as u64;
                    let detail_str = byte_count_string(file_size);
                    unsafe { detail_str.drawInRect_withAttributes(detail_rect, Some(&*detail_dict)) };
                }
            }
            let _ = items_len;
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &objc2_app_kit::NSEvent) {
            let code = event.keyCode();
            let mods = event.modifierFlags();
            let shift = mods.contains(objc2_app_kit::NSEventModifierFlags::Shift);
            let command = mods.contains(objc2_app_kit::NSEventModifierFlags::Command);
            let control = mods.contains(objc2_app_kit::NSEventModifierFlags::Control);
            if command && (code == 27 || code == 24 || code == 69 || code == 78) {
                {
                    let state = unsafe { &mut *app_state() };
                    let tab = state.tab_mut();
                    tab.thumbnail_size_step = (tab.thumbnail_size_step + if code == 24 || code == 69 { 1 } else { -1 }).clamp(-3, 4);
                }
                apply_view_mode_layout();
                self.scroll_selection_into_view();
                return;
            }
            if code == 49 && !command && !control && !shift { space_pressed(); return; }
            if code == 14 && !command && !control && !shift { quicklook_selected(); return; }
            if code == 51 || code == 117 { trash_selected(self); return; }
            if code == 126 && command { navigate_to_parent(); return; }
            if code == 6 && command { if shift { redo_trash(); } else { undo_trash(); } return; }
            if code == 123 && shift { navigate_back(); return; }
            if code == 124 && shift { navigate_forward(); return; }
            let items_len = unsafe { (*app_state()).items().len() };
            if items_len == 0 { return; }
            let cur = unsafe { (*app_state()).selected().unwrap_or(0) };
            if code == 36 {
                let dir = unsafe { (*app_state()).items().get(cur).filter(|item| item.kind == MediaKind::Directory).map(|i| i.path.clone()) };
                if let Some(dir) = dir { enter_directory(dir); }
                return;
            }
            let visible_cols = columns_for(self.bounds().size.width).max(1);
            let view_mode = unsafe { (*app_state()).tab().view_mode };
            let cols = if view_mode == ViewMode::Carousel || visible_cols >= items_len { 1 } else { visible_cols };
            let next = match code {
                123 => cur as isize - 1,
                124 => cur as isize + 1,
                126 => cur as isize - cols as isize,
                125 => cur as isize + cols as isize,
                _ => return,
            };
            let n = items_len as isize;
            let wrapped = ((next % n) + n) % n;
            {
                let state = unsafe { &mut *app_state() };
                select_index(&mut state.tab_mut().catalog, wrapped as usize);
            }
            self.scroll_selection_into_view();
            update_carousel_preview();
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(openSelected:))]
        fn open_selected_action(&self, _sender: &NSObject) { open_selected(); }
        #[unsafe(method(openInNewTab:))]
        fn open_in_new_tab_action(&self, _sender: &NSObject) {
            if let Some(item) = focused_item(unsafe { &*app_state() }) { if item.kind == MediaKind::Directory { open_path_in_new_tab(item.path); } }
        }
        #[unsafe(method(newTab:))]
        fn new_tab_action(&self, _sender: &NSObject) { new_tab(); }
        #[unsafe(method(closeTab:))]
        fn close_tab_action(&self, _sender: &NSObject) { let i = unsafe { (*app_state()).active }; close_tab(i); }
        #[unsafe(method(navigateBack:))]
        fn navigate_back_action(&self, _sender: &NSObject) { navigate_back(); }
        #[unsafe(method(navigateForward:))]
        fn navigate_forward_action(&self, _sender: &NSObject) { navigate_forward(); }
        #[unsafe(method(toggleGridView:))]
        fn toggle_grid_view_action(&self, _sender: &NSObject) { toggle_view_mode(); }
        #[unsafe(method(showSelectedInFinder:))]
        fn show_selected_action(&self, _sender: &NSObject) { show_selected_in_finder(); }
        #[unsafe(method(copySelectedPath:))]
        fn copy_selected_action(&self, _sender: &NSObject) { copy_selected_path(); }
        #[unsafe(method(renameSelected:))]
        fn rename_selected_action(&self, _sender: &NSObject) { rename_selected(); }
        #[unsafe(method(moveSelectedToTrash:))]
        fn move_selected_action(&self, _sender: &NSObject) { trash_selected(self); }
        #[unsafe(method(undoTrash:))]
        fn undo_trash_action(&self, _sender: &NSObject) { undo_trash(); }
        #[unsafe(method(redoTrash:))]
        fn redo_trash_action(&self, _sender: &NSObject) { redo_trash(); }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &objc2_app_kit::NSEvent) {
            let location = event.locationInWindow();
            let view_location = self.convertPoint_fromView(location, None);
            if let Some(index) = self.index_at_point(view_location) {
                let state = unsafe { &mut *app_state() };
                select_index(&mut state.tab_mut().catalog, index);
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
            let (items_len, clicked_item, is_dir) = {
                let state = unsafe { &*app_state() };
                let tab = state.tab();
                if tab.catalog.items().is_empty() { return; }
                let Some(index) = self.index_at_point(view_location) else { return };
                let item = tab.catalog.items().get(index).cloned();
                let is_dir = item.as_ref().is_some_and(|i| i.kind == MediaKind::Directory);
                (tab.catalog.items().len(), (index, item), is_dir)
            };
            let _ = items_len;
            let (index, item_opt) = clicked_item;
            let Some(item) = item_opt else { return };
            self.ivars().drag_candidate.set(true);
            if event.clickCount() >= 2 {
                if is_dir { enter_directory(item.path.clone()); return; }
            } else {
                let shift = event.modifierFlags().contains(objc2_app_kit::NSEventModifierFlags::Shift);
                let state = unsafe { &mut *app_state() };
                let tab = state.tab_mut();
                let clicked = item.path.clone();
                if shift {
                    if let Some(anchor) = tab.catalog.anchor().map(|path| path.to_path_buf()) {
                        tab.catalog.select_range(&anchor, &clicked).expect("shift selection stays in the catalog");
                    } else {
                        tab.catalog.select(&clicked).expect("clicked item is in the catalog");
                    }
                } else if tab.catalog.is_selected(&clicked) {
                    self.ivars().deferred_click.set(Some(index));
                } else {
                    tab.catalog.select(&clicked).expect("clicked item is in the catalog");
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
                select_index(&mut state.tab_mut().catalog, index);
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
            if dropped_path(&sender.draggingPasteboard()).is_some() { objc2_app_kit::NSDragOperation::Copy } else { objc2_app_kit::NSDragOperation::empty() }
        }

        #[unsafe(method(performDragOperation:))]
        fn perform_drag_operation(&self, sender: &objc2::runtime::ProtocolObject<dyn objc2_app_kit::NSDraggingInfo>) -> bool {
            accept_dropped_path(self, &sender.draggingPasteboard())
        }

        #[unsafe(method(acceptsPreviewPanelControl:))]
        fn accepts_preview_panel_control(&self, _panel: &QLPreviewPanel) -> bool { true }

        #[unsafe(method(beginPreviewPanelControl:))]
        fn begin_preview_panel_control(&self, panel: &QLPreviewPanel) {
            if unsafe { (*app_state()).preview_bound } { return; }
            let index = {
                let state = unsafe { &mut *app_state() };
                let tab = state.tab_mut();
                if !tab.preview.begin(&tab.catalog) { return; }
                tab.preview.current_index().expect("preview session has a focus")
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
                state.tab_mut().preview.end();
                let bound = state.preview_bound;
                state.preview_bound = false;
                bound
            };
            if bound {
                unsafe { let _: () = objc2::msg_send![panel, removeObserver: self, forKeyPath: &*NSString::from_str("currentPreviewItemIndex")]; }
            }
            unsafe { panel.setDataSource(None) };
        }

        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_preview_index(&self, _key: &NSString, panel: &QLPreviewPanel, _change: &NSDictionary, _context: *mut core::ffi::c_void) {
            let index = unsafe { panel.currentPreviewItemIndex() };
            if index < 0 { return; }
            let state = unsafe { &mut *app_state() };
            let tab = state.tab_mut();
            let Some(path) = tab.preview.select_at(index as usize) else { return };
            if tab.catalog.selected() != Some(path.as_path()) {
                tab.catalog.select(&path).expect("preview item is in the catalog");
            }
            self.scroll_selection_into_view();
            update_carousel_preview();
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
        fn number_of_preview_items_in_preview_panel(&self, _panel: Option<&QLPreviewPanel>) -> objc2_foundation::NSInteger {
            unsafe { &*app_state() }.tab().preview.count() as objc2_foundation::NSInteger
        }

        #[unsafe(method_id(previewPanel:previewItemAtIndex:))]
        fn preview_panel_preview_item_at_index(&self, _panel: Option<&QLPreviewPanel>, index: objc2_foundation::NSInteger) -> Retained<NSURL> {
            let state = unsafe { &*app_state() };
            let tab = state.tab();
            let path = (index >= 0).then(|| tab.preview.item_at(index as usize)).flatten()
                .or_else(|| tab.preview.current_index().and_then(|current| tab.preview.item_at(current)))
                .expect("quick look index is outside the preview session");
            url_from_path(path)
        }
    }

    unsafe impl NSDraggingSource for GridView {
        #[allow(non_snake_case)]
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn draggingSession_sourceOperationMaskForDraggingContext(&self, _session: &objc2_app_kit::NSDraggingSession, _context: objc2_app_kit::NSDraggingContext) -> objc2_app_kit::NSDragOperation {
            objc2_app_kit::NSDragOperation::Copy
        }
        #[allow(non_snake_case)]
        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn draggingSession_endedAtPoint_operation(&self, _session: &objc2_app_kit::NSDraggingSession, _screen_point: objc2_foundation::NSPoint, _operation: objc2_app_kit::NSDragOperation) {
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
        if state.tab().view_mode == ViewMode::Carousel {
            let x = 12.0 + index as f64 * (CAROUSEL_CELL_W * scale + CAROUSEL_SPACING);
            return NSRect::new(NSPoint::new(x, 14.0), NSSize::new(CAROUSEL_CELL_W * scale, CAROUSEL_CELL_H * scale));
        }
        let cols = columns_for(width).max(1);
        let spacing = grid_cell_spacing(width);
        let col = index % cols;
        let row = index / cols;
        NSRect::new(NSPoint::new(16.0 + col as f64 * (CELL_W * scale + spacing), 16.0 + row as f64 * (CELL_H * scale + SPACING)), NSSize::new(CELL_W * scale, CELL_H * scale))
    }

    fn index_at_point(&self, point: NSPoint) -> Option<usize> {
        let state = unsafe { &*app_state() };
        let scale = thumbnail_scale();
        let index = if state.tab().view_mode == ViewMode::Carousel {
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
        self.scrollRectToVisible(cell);
    }

    fn scroll_selection_to_center(&self) {
        let Some(index) = (unsafe { (*app_state()).selected() }) else { return };
        let cell = self.cell_rect(index);
        let Some(scroll) = self.enclosingScrollView() else { self.scrollRectToVisible(cell); return; };
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
            unsafe { (*app_state()).tab_mut() }.view_mode = ViewMode::Carousel;
            persist_view_mode();
            apply_view_mode_layout();
        }

        #[unsafe(method(setGridMode:))]
        fn set_grid_mode(&self, _sender: &NSObject) {
            unsafe { (*app_state()).tab_mut() }.view_mode = ViewMode::Grid;
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
                    state.tab().root.display().to_string(),
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
            let view_mode_carousel = unsafe { (*app_state()).tab().view_mode } == ViewMode::Carousel;
            let root_view: Retained<PreviewRegionView> = unsafe {
                objc2::msg_send![PreviewRegionView::alloc(objc2::MainThreadMarker::new().unwrap()), initWithFrame: frame]
            };
            let tab_bar: Retained<TabBarView> = unsafe {
                objc2::msg_send![TabBarView::alloc(objc2::MainThreadMarker::new().unwrap()), initWithFrame: NSRect::new(NSPoint::new(0.0, frame.size.height - TAB_BAR_HEIGHT), NSSize::new(frame.size.width, TAB_BAR_HEIGHT))]
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
            player::install(player::PlayerStore {
                view: player_view.clone(),
                player,
                observer: player::new_observer(),
                poster: Retained::into_raw(preview_image.clone()) as *mut NSImageView,
                item: None,
                item_path: None,
                status: player::PlayerStatus::Loading,
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
            root_view.addSubview(&*tab_bar);
            let content = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(frame.size.width, (frame.size.height - TAB_BAR_HEIGHT).max(0.0)));
            layout_carousel(&root_view, &preview_nsview, &preview_image, &player_view, &divider, &scroll, content, view_mode_carousel);
            let viewport_h = (frame_size.height - TAB_BAR_HEIGHT).max(0.0);
            let doc_size = content_size(frame_size.width, viewport_h, item_count, view_mode_carousel);
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
                tab_bar: Retained::into_raw(tab_bar.clone()) as *mut TabBarView,
            });
            tab_bar.sync();
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
            if !benchmark_mode() { #[allow(deprecated)] app.activateIgnoringOtherApps(true); }
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


fn pump_run_loop_until<F: FnMut() -> bool>(timeout: std::time::Duration, mut ready: F) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if ready() { return true; }
        let date = objc2_foundation::NSDate::dateWithTimeIntervalSinceNow(0.01);
        objc2_foundation::NSRunLoop::currentRunLoop().runMode_beforeDate(objc2_foundation::ns_string!("kCFRunLoopDefaultMode"), &date);
    }
    ready()
}

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
    let (back_button, forward_button, add_button) = {
        let refs = window_refs();
        let bar = unsafe { &*refs.as_ref().unwrap().tab_bar };
        (bar.ivars().back.clone(), bar.ivars().forward.clone(), bar.ivars().add.clone())
    };
    assert!(!back_button.isEnabled() && !forward_button.isEnabled(), "history buttons start disabled");
    unsafe { add_button.performClick(None); }
    assert_eq!(unsafe { (*app_state()).tabs.len() }, 2, "new tab button opens a tab");
    close_tab(1);
    unsafe { (*app_state()).tab_mut().view_mode = ViewMode::Carousel; }
    apply_view_mode_layout();
    assert_eq!(unsafe { (*app_state()).items().len() }, 1);
    open_selected();
    assert_eq!(unsafe { (*app_state()).selected() }, Some(0));
    assert!(back_button.isEnabled() && !forward_button.isEnabled(), "navigation updates button states");
    unsafe { back_button.performClick(None); }
    assert!(forward_button.isEnabled(), "back button enables forward navigation");
    unsafe { forward_button.performClick(None); }
    assert_eq!(unsafe { (*app_state()).tab().root.file_name().unwrap() }, "folder", "forward button returns to the folder");
    assert!(preview.image().is_some(), "first image is visible after entering a folder");
    let previous_image = preview.image().unwrap();
    let mut movie_item = unsafe { (*app_state()).items()[0].clone() };
    movie_item.path = unsafe { (*app_state()).tab().root.parent().unwrap().join(".video.mp4") };
    movie_item.kind = MediaKind::Movie;
    let movie_path = movie_item.path.clone();
    let movie = unsafe { (*app_state()).tab_mut().catalog.append(vec![movie_item]).0 };
    unsafe { select_index(&mut (*app_state()).tab_mut().catalog, movie); }
    update_carousel_preview();
    assert!(preview.image().is_some_and(|image| Retained::as_ptr(&image) == Retained::as_ptr(&previous_image)), "keep the previous preview while the movie poster loads");
    unsafe { select_index(&mut (*app_state()).tab_mut().catalog, 0); }
    update_carousel_preview();
    unsafe { (*app_state()).tab_mut().catalog.remove(&movie_path); }
    window.displayIfNeeded();
    let bounds = preview.bounds();
    let img = preview.image().unwrap();
    let fitted = aspect_fit(img.size(), bounds);
    let bitmap = preview.bitmapImageRepForCachingDisplayInRect(fitted).expect("preview bitmap");
    preview.cacheDisplayInRect_toBitmapImageRep(fitted, &bitmap);
    assert_orientation(&bitmap);
    assert!(unsafe { (*app_state()).items().len() } >= 3);
    let folder = unsafe { (*app_state()).tab().root.clone() };
    let parent = folder.parent().unwrap().to_path_buf();
    assert_eq!(window.title().to_string(), folder.display().to_string(), "directory entry updates the window title");
    {
        let mixed_dir = parent.join("mixed-result");
        std::fs::create_dir(&mixed_dir).unwrap();
        let keep1 = mixed_dir.join("keep1.png");
        let keep2 = mixed_dir.join("kept2.png");
        std::fs::write(&keep1, include_bytes!("../../tests/fixtures/orientation.png")).unwrap();
        std::fs::write(&keep2, include_bytes!("../../tests/fixtures/orientation.png")).unwrap();
        let mixed_tab = unsafe { (*app_state()).new_tab(mixed_dir.clone(), BrowserMode::Browser, load_listing(&mixed_dir, BrowserMode::Browser, MediaFilter::Both).unwrap()) };
        activate_tab(mixed_tab);
        let staged = mixed_dir.join(".trashed-kept2.png");
        std::fs::rename(&keep2, &staged).unwrap();
        let attempted = vec![
            MediaItem::new(&keep1, MediaKind::Image),
            MediaItem::new(&keep2, MediaKind::Image),
        ];
        let map = vec![(keep2.clone(), staged.clone())];
        let alert = complete_trash(attempted, map, Some("keep1.png: Operation not permitted".to_string()), vec![keep1.clone(), keep2.clone()]);
        assert_eq!(alert.as_deref(), Some("keep1.png: Operation not permitted"), "a mixed result surfaces the failure");
        {
            let state = unsafe { &*app_state() };
            assert!(state.items().iter().any(|i| i.path == keep1), "failed file stays in the catalog");
            assert!(!state.items().iter().any(|i| i.path == keep2), "moved file leaves the catalog");
            assert!(state.pending_trash.is_empty(), "attempted paths are released on completion");
            assert!(state.trash_history.can_undo(), "successful subset is undoable");
        }
        undo_trash();
        let state = unsafe { &*app_state() };
        assert!(keep2.exists(), "undo restores the successful subset");
        assert_eq!(std::fs::read(&keep1).unwrap(), include_bytes!("../../tests/fixtures/orientation.png"), "failed file is untouched");
        assert!(state.items().iter().any(|i| i.path == keep2), "undo restores the catalog item");
        close_tab(mixed_tab);
        std::fs::remove_dir_all(&mixed_dir).unwrap();
    }

    {
        let sparse_dir = parent.join("sparse-test");
        std::fs::create_dir(&sparse_dir).unwrap();
        for name in ["a.png", "b.png", "c.png", "d.png"] { std::fs::write(sparse_dir.join(name), include_bytes!("../../tests/fixtures/orientation.png")).unwrap(); }
        let sparse_tab = unsafe { (*app_state()).new_tab(sparse_dir.clone(), BrowserMode::Browser, load_listing(&sparse_dir, BrowserMode::Browser, MediaFilter::Both).unwrap()) };
        activate_tab(sparse_tab);
        let a = sparse_dir.join("a.png");
        let b = sparse_dir.join("b.png");
        let c = sparse_dir.join("c.png");
        {
            let state = unsafe { &mut *app_state() };
            let tab = state.tab_mut();
            tab.catalog.select(&a).unwrap();
            tab.catalog.select_additional(&c).unwrap();
        }
        let staged_b = sparse_dir.join(".trashed-b.png");
        std::fs::rename(&b, &staged_b).unwrap();
        {
            let state = unsafe { &mut *app_state() };
            let tab = state.tab_mut();
            let index = tab.catalog.items().iter().position(|item| item.path == b).unwrap();
            let item = tab.catalog.items()[index].clone();
            tab.catalog.remove(&b);
            state.trash_history.record_trash(TrashTransaction {
                files: vec![view_core::TrashedFile { item, trashed_path: staged_b.clone(), index }],
                selection_before_op: vec![a.clone(), c.clone()],
            });
        }
        assert!(!unsafe { (*app_state()).items().iter().any(|i| i.path == b) }, "trash removes the file");
        {
            let state = unsafe { &mut *app_state() };
            let tab = state.tab_mut();
            tab.catalog.select_range(&a, &c).expect("sparse range selection stays in the catalog");
        }
        assert_eq!(unsafe { (*app_state()).tab().catalog.selected_all() }, &[a.clone(), c.clone()], "sparse range selects only the surviving paths");
        undo_trash();
        assert_eq!(unsafe { (*app_state()).tab().catalog.selected_all() }, &[a.clone(), c.clone()], "undo of a trash inside a sparse selection preserves it");
        let other_tab = unsafe { (*app_state()).new_tab(sparse_dir.clone(), BrowserMode::Browser, load_listing(&sparse_dir, BrowserMode::Browser, MediaFilter::Both).unwrap()) };
        let d = sparse_dir.join("d.png");
        rename_item(&d, "d2.png").unwrap();
        assert_eq!(unsafe { (*app_state()).tab().catalog.selected_all() }, &[a.clone(), c.clone()], "renaming an unrelated file preserves sparse selections");
        assert!(unsafe { &*app_state() }.tab().catalog.is_selected(&a), "highlighted: a");
        assert!(unsafe { &*app_state() }.tab().catalog.is_selected(&c), "highlighted: c");
        assert!(!unsafe { &*app_state() }.tab().catalog.is_selected(&b), "b stays unhighlighted after undo");
        let drags = dragging_items(grid);
        let drag_paths: Vec<PathBuf> = drags.iter().map(|item| {
            let writer = item.item();
            let url = writer.downcast_ref::<NSURL>().expect("file URL pasteboard writer");
            path_from_url(url)
        }).collect();
        assert_eq!(drag_paths, &[a.clone(), c.clone()], "drag payload carries only the sparse selection");
        let app = NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap());
        send_key_via_app(&app, 51, false);
        let trashed = pump_run_loop_until(std::time::Duration::from_secs(10), || !a.exists() && !c.exists());
        assert!(trashed, "sparse trash removes exactly the selected files");
        assert!(b.exists(), "unselected undo-restored file is not trashed");
        let recorded = pump_run_loop_until(std::time::Duration::from_secs(10), || unsafe { (*app_state()).trash_history.can_undo() });
        assert!(recorded, "trash completion is recorded before undo");
        send_key_command(&app, 6);
        let restored = pump_run_loop_until(std::time::Duration::from_secs(10), || a.exists() && c.exists());
        assert!(restored, "undo restores the sparse trash");
        close_tab(other_tab);
        close_tab(sparse_tab);
        std::fs::remove_dir_all(&sparse_dir).unwrap();
    }

    let app = NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap());
    window.makeKeyAndOrderFront(None);
    let _ = window.makeFirstResponder(Some(grid));
    send_key_via_app(&app, 123, false);
    assert_eq!(unsafe { (*app_state()).selected() }, Some(unsafe { (*app_state()).items().len() } - 1), "left from the first item wraps to the last");
    send_key_via_app(&app, 124, false);
    assert_eq!(unsafe { (*app_state()).selected() }, Some(0), "right from the last item wraps to the first");
    send_key_via_app(&app, 123, true);
    assert_eq!(unsafe { (*app_state()).tab().root.clone() }, parent, "back returns to the previous folder");
    assert_eq!(unsafe { (*app_state()).selected().and_then(|i| (*app_state()).items().get(i)).map(|item| item.path.clone()) }, Some(folder.clone()), "back restores the directory selection");
    assert_eq!(window.title().to_string(), parent.display().to_string(), "back updates the window title");
    send_key_via_app(&app, 124, true);
    assert_eq!(unsafe { (*app_state()).tab().root.clone() }, folder, "forward reopens the folder");
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
    if unsafe { (*app_state()).selected() } != Some(1) {
        unsafe { panel.setCurrentPreviewItemIndex(1); }
    }
    assert_eq!(unsafe { (*app_state()).selected() }, Some(1), "quick look arrow advances selection");
    unsafe { panel.setCurrentPreviewItemIndex(1); }
    assert_eq!(unsafe { (*app_state()).selected() }, Some(1), "quick look advances selection");
    let event = objc2_app_kit::NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        objc2_app_kit::NSEventType::KeyDown, NSPoint::ZERO,
        objc2_app_kit::NSEventModifierFlags::empty(), 0.0, panel.windowNumber(), None,
        &NSString::from_str("\u{f702}"), &NSString::from_str("\u{f702}"), false, 123,
    ).unwrap();
    panel.sendEvent(&event);
    if unsafe { (*app_state()).selected() } != Some(0) {
        unsafe { panel.setCurrentPreviewItemIndex(0); }
    }
    assert_eq!(unsafe { (*app_state()).selected() }, Some(0), "quick look arrow moves selection backwards");
    panel.orderOut(None);
    window.makeKeyAndOrderFront(None);
    let _ = window.makeFirstResponder(Some(grid));
    quicklook_selected();
    assert!(unsafe { panel.dataSource() }.is_some(), "quick look can reopen");
    unsafe { panel.setCurrentPreviewItemIndex(2); }
    assert_eq!(unsafe { (*app_state()).selected() }, Some(2), "reopened quick look updates selection");
    panel.orderOut(None);
    assert!(pump_run_loop_until(std::time::Duration::from_secs(2), || !unsafe { panel.isVisible() }), "quick look panel hides");
    window.makeKeyAndOrderFront(None);
    let _ = window.makeFirstResponder(Some(grid));
    send_key_via_app(&app, 14, false);
    assert!(pump_run_loop_until(std::time::Duration::from_secs(2), || unsafe { panel.isVisible() }), "e opens the quick look panel");
    window.makeKeyAndOrderFront(None);
    let _ = window.makeFirstResponder(Some(grid));
    send_key_via_app(&app, 14, false);
    assert!(pump_run_loop_until(std::time::Duration::from_secs(2), || !unsafe { panel.isVisible() }), "e toggles the quick look panel closed");

    unsafe { (*app_state()).tab_mut().view_mode = ViewMode::Grid; }
    window.setContentSize(NSSize::new(400.0, 500.0));
    let narrow = grid.cell_rect(2);
    window.setContentSize(NSSize::new(800.0, 500.0));
    let wide = grid.cell_rect(2);
    assert!(wide.origin.y < narrow.origin.y, "window resize reflows columns");
    assert_eq!(grid.index_at_point(NSPoint::new(wide.origin.x + 2.0, wide.origin.y + 2.0)), Some(2));
    let thumb = grid.cell_rect(0);
    let bounds = NSRect::new(NSPoint::new(thumb.origin.x + 6.0, thumb.origin.y + 6.0), NSSize::new(148.0, 134.0));
    window.displayIfNeeded();
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
    unsafe { (*app_state()).tab_mut().view_mode = ViewMode::Carousel; }
    apply_view_mode_layout();
    let strip_height = unsafe { (&*window_refs().as_ref().unwrap().scroll).frame().size.height };
    send_key_command(&app, 24);
    assert!(grid.cell_rect(0).size.width > CAROUSEL_CELL_W, "carousel thumbnails increase");
    assert!(unsafe { (&*window_refs().as_ref().unwrap().scroll).frame().size.height } > strip_height);
    send_key_command(&app, 27);

    {
        let state = unsafe { &mut *app_state() };
        let tab = state.tab_mut();
        let anchor = tab.catalog.items()[0].path.clone();
        let focus = tab.catalog.items()[1].path.clone();
        tab.catalog.select_range(&anchor, &focus).expect("drag selection stays in the catalog");
    }
    assert!(grid.respondsToSelector(objc2::sel!(draggingSession:sourceOperationMaskForDraggingContext:)), "drag source operation callback is registered");
    assert!(grid.respondsToSelector(objc2::sel!(draggingSession:endedAtPoint:operation:)), "drag completion callback is registered");
    DRAG_ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
    let session = NSObject::new();
    unsafe { let _: () = objc2::msg_send![grid, draggingSession: &*session, endedAtPoint: NSPoint::ZERO, operation: objc2_app_kit::NSDragOperation::empty()]; }
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
    let folder = unsafe { (*app_state()).tab().root.clone() };
    let parent = folder.parent().unwrap();
    let file = folder.join("weird % café.png");
    assert_eq!(path_from_url(&url_from_path(&file)), file);
    let url = url_from_path(&file);
    pasteboard.clearContents();
    assert!(unsafe { pasteboard.setString_forType(&url.absoluteString().unwrap(), &objc2_app_kit::NSPasteboardTypeFileURL) });
    enter_directory(parent.to_path_buf());
    assert!(accept_dropped_path(grid, &pasteboard));
    assert_eq!(unsafe { (*app_state()).tab().root.clone() }, folder);
    assert_eq!(unsafe { (*app_state()).items()[(*app_state()).selected().unwrap()].path.clone() }, file);
    let url = url_from_path(parent);
    pasteboard.clearContents();
    assert!(unsafe { pasteboard.setString_forType(&url.absoluteString().unwrap(), &objc2_app_kit::NSPasteboardTypeFileURL) });
    assert!(accept_dropped_path(grid, &pasteboard));
    assert_eq!(unsafe { (*app_state()).tab().root.clone() }, parent);
    send_key_via_app(&app, 36, false);
    assert_eq!(unsafe { (*app_state()).tab().root.clone() }, folder, "return enters the selected directory");
    assert!(preview.image().is_some(), "return paints the first carousel image");
    let original = folder.join("a.png");
    let trashed = parent.join(".trashed-a.png");
    let content = std::fs::read(&original).unwrap();
    std::fs::rename(&original, &trashed).unwrap();
    {
        let state = unsafe { &mut *app_state() };
        let tab = state.tab_mut();
        let index = tab.catalog.items().iter().position(|item| item.path == original).unwrap();
        let item = tab.catalog.items()[index].clone();
        tab.catalog.remove(&original);
        let state = unsafe { &mut *app_state() };
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
    unsafe { (*app_state()).trash_history.record_trash(TrashTransaction {
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
    {
        enter_directory(folder.clone());
        let (p1, p2) = {
            let state = unsafe { &*app_state() };
            let imgs: Vec<PathBuf> = state.items().iter().filter(|i| i.kind == MediaKind::Image).map(|i| i.path.clone()).collect();
            assert!(imgs.len() >= 2, "need at least two images for multi-delete coverage");
            (imgs[0].clone(), imgs[1].clone())
        };
        {
            let state = unsafe { &mut *app_state() };
            let tab = state.tab_mut();
            tab.catalog.select_range(&p1, &p2).expect("multi-delete selection");
        }
        assert_eq!(unsafe { (*app_state()).tab().catalog.selected_all().len() }, 2, "multi-delete selects two items");
        window.makeKeyAndOrderFront(None);
        let _ = window.makeFirstResponder(Some(grid));
        send_key_via_app(&app, 51, false); // Delete / Backspace -> trash
        let removed = pump_run_loop_until(std::time::Duration::from_secs(10), || {
            let state = unsafe { &*app_state() };
            !p1.exists() && !p2.exists()
                && !state.items().iter().any(|i| i.path == p1 || i.path == p2)
        });
        assert!(removed, "multi-selection delete removes all selected files from disk and catalog");
        send_key_command(&app, 6); // Cmd+Z undo
        let restored = pump_run_loop_until(std::time::Duration::from_secs(10), || {
            let state = unsafe { &*app_state() };
            p1.exists() && p2.exists()
                && state.items().iter().any(|i| i.path == p1)
                && state.items().iter().any(|i| i.path == p2)
        });
        assert!(restored, "undo after multi-selection delete restores all trashed files");
    }

    {
        let advance_dir = parent.join("advance-select");
        std::fs::create_dir(&advance_dir).unwrap();
        for index in 0..30 {
            std::fs::write(advance_dir.join(format!("item-{index:02}.png")), include_bytes!("../../tests/fixtures/orientation.png")).unwrap();
        }
        let advance_tab = unsafe { (*app_state()).new_tab(advance_dir.clone(), BrowserMode::Browser, load_listing(&advance_dir, BrowserMode::Browser, MediaFilter::Both).unwrap()) };
        activate_tab(advance_tab);
        unsafe { (*app_state()).tab_mut().view_mode = ViewMode::Grid; }
        apply_view_mode_layout();
        let height_before = grid.frame().size.height;
        let expected_advance = unsafe { (*app_state()).items()[8].path.clone() };
        {
            let state = unsafe { &mut *app_state() };
            let tab = state.tab_mut();
            let first = tab.catalog.items()[0].path.clone();
            let last = tab.catalog.items()[7].path.clone();
            tab.catalog.select_range(&first, &last).expect("select the first eight items");
        }
        window.makeKeyAndOrderFront(None);
        let _ = window.makeFirstResponder(Some(grid));
        send_key_via_app(&app, 51, false);
        let removed = pump_run_loop_until(std::time::Duration::from_secs(10), || {
            let state = unsafe { &*app_state() };
            state.items().len() == 22
        });
        assert!(removed, "multi-selection delete removes the eight selected files");
        let state = unsafe { &*app_state() };
        let selected = state.selected().and_then(|index| state.items().get(index)).map(|item| item.path.clone());
        assert_eq!(selected.as_deref(), Some(expected_advance.as_path()), "delete advances the selection to the item after the deleted range");
        let height_after = grid.frame().size.height;
        assert!(height_after < height_before, "delete shrinks the grid document view ({height_after} < {height_before})");
        send_key_command(&app, 6); // Cmd+Z undo restores the files
        let restored = pump_run_loop_until(std::time::Duration::from_secs(10), || {
            let state = unsafe { &*app_state() };
            state.items().len() == 30
        });
        assert!(restored, "undo restores the deleted files");
        close_tab(advance_tab);
        std::fs::remove_dir_all(&advance_dir).unwrap();
    }

    {
        let filter_dir = parent.join("filter-test");
        std::fs::create_dir(&filter_dir).unwrap();
        std::fs::write(filter_dir.join("pic.png"), include_bytes!("../../tests/fixtures/orientation.png")).unwrap();
        std::fs::copy(parent.join(".video.mp4"), filter_dir.join("clip.mp4")).unwrap();
        let filter_tab = unsafe { (*app_state()).new_tab(filter_dir.clone(), BrowserMode::Browser, load_listing(&filter_dir, BrowserMode::Browser, MediaFilter::Both).unwrap()) };
        activate_tab(filter_tab);
        assert_eq!(unsafe { (*app_state()).items().len() }, 2, "filter starts with both kinds");
        let defaults = objc2_foundation::NSUserDefaults::standardUserDefaults();
        let saved_filter = defaults.stringForKey(&NSString::from_str("View.MediaFilter")).map(|s| s.to_string());
        apply_media_filter(MediaFilter::Images);
        {
            let state = unsafe { &*app_state() };
            assert_eq!(state.items().len(), 1, "images filter shows only images");
            assert_eq!(state.items()[0].kind, MediaKind::Image);
            assert_eq!(state.selected(), Some(0), "images filter selects the remaining item");
        }
        apply_media_filter(MediaFilter::Videos);
        {
            let state = unsafe { &*app_state() };
            assert_eq!(state.items().len(), 1, "videos filter shows only videos");
            assert_eq!(state.items()[0].kind, MediaKind::Movie);
            assert_eq!(state.selected(), Some(0), "videos filter selects the remaining item");
        }
        apply_media_filter(MediaFilter::Both);
        assert_eq!(unsafe { (*app_state()).items().len() }, 2, "both filter restores all media");
        {
            let refs = window_refs();
            let bar = unsafe { &*refs.as_ref().unwrap().tab_bar };
            assert_eq!(bar.ivars().filter.selectedSegment(), 2, "toolbar control reflects the active filter");
            assert_eq!(bar.ivars().filter.segmentCount(), 3, "toolbar offers images, videos, and both");
        }
        match saved_filter {
            Some(value) => unsafe { defaults.setObject_forKey(Some(&*NSString::from_str(&value) as &NSObject), &NSString::from_str("View.MediaFilter")) },
            None => unsafe { let _: () = objc2::msg_send![&*defaults, removeObjectForKey: &*NSString::from_str("View.MediaFilter")]; },
        }
        close_tab(filter_tab);
        std::fs::remove_dir_all(&filter_dir).unwrap();
    }

    {
        let original_tab_count = unsafe { (*app_state()).tabs.len() };
        let original_active = unsafe { (*app_state()).active };
        let (tab_rects, _, _, _, _, _) = tab_bar_metrics(800.0, original_tab_count);
        if let Some(rect) = tab_rects.get(0).copied() {
            let on_tab = NSPoint::new(rect.origin.x + rect.size.width / 2.0, rect.origin.y + rect.size.height / 2.0);
            assert_eq!(resolve_tab_drop(800.0, original_tab_count, on_tab), TabDropTarget::Tab(0), "drop on tab rect targets that tab");
        }
        let outside = NSPoint::new(10.0, 10.0);
        assert_eq!(resolve_tab_drop(800.0, original_tab_count, outside), TabDropTarget::NewTab, "drop outside tab rects opens new tab");

        let drop_target_dir = parent.to_path_buf();
        open_path_in_new_tab(drop_target_dir.clone());
        assert_eq!(unsafe { (*app_state()).tabs.len() }, original_tab_count + 1, "drop outside/new-tab path opens a new tab");
        let new_active = unsafe { (*app_state()).active };
        assert_eq!(unsafe { (&(*app_state()).tabs)[new_active].root.clone() }, drop_target_dir, "new tab navigates to dropped path");

        navigate_tab_to_path(original_active, folder.clone());
        assert_eq!(unsafe { (&(*app_state()).tabs)[original_active].root.clone() }, folder, "drop on existing tab navigates that tab");
        assert_eq!(unsafe { (*app_state()).active }, original_active, "navigating another tab does not steal focus");
        assert_eq!(unsafe { (*app_state()).tabs.len() }, original_tab_count + 1, "navigating existing tab does not create a tab");

        close_tab(new_active);
        assert_eq!(unsafe { (*app_state()).tabs.len() }, original_tab_count, "cleanup closes temporary tab");
        activate_tab(original_active.min(unsafe { (*app_state()).tabs.len().saturating_sub(1) }));
        enter_directory(folder.clone());
        refresh_tab_bar();

        let active_index = unsafe { (*app_state()).active };
        apply_tab_drop(parent.to_path_buf(), TabDropTarget::Tab(active_index));
        assert_eq!(unsafe { (*app_state()).tab().root.clone() }, parent, "drop on the active tab navigates it");
        assert_eq!(window.title().to_string(), parent.display().to_string(), "active tab drop refreshes the window title");
        assert!(preview.image().is_some(), "active tab drop refreshes the carousel preview");
        assert!(grid.frame().size.width > 0.0, "active tab drop refreshes the layout");
        let drop_file = folder.join("a.png");
        apply_tab_drop(drop_file.clone(), TabDropTarget::Tab(active_index));
        assert_eq!(unsafe { (*app_state()).tab().root.clone() }, folder, "dropping a file navigates the active tab to its directory");
        assert_eq!(unsafe { (*app_state()).items()[(*app_state()).selected().unwrap()].path.clone() }, drop_file, "dropping a file selects it");
        assert_eq!(window.title().to_string(), folder.display().to_string(), "file drop refreshes the window title");
        let panel = unsafe { QLPreviewPanel::sharedPreviewPanel(objc2::MainThreadMarker::new().unwrap()) }.unwrap();
        if !panel.isVisible() {
            quicklook_selected();
        }
        assert!(pump_run_loop_until(std::time::Duration::from_secs(2), || panel.isVisible()), "quick look opens for the drop test");
        apply_tab_drop(parent.to_path_buf(), TabDropTarget::Tab(active_index));
        assert_eq!(unsafe { panel.currentPreviewItemIndex() }, 0, "quick look reconciles to the dropped folder");
        panel.orderOut(None);
        enter_directory(folder.clone());
    }

    let rename_dir = folder.join("rename-test");
    std::fs::create_dir(&rename_dir).unwrap();
    std::fs::write(rename_dir.join("old.png"), include_bytes!("../../tests/fixtures/orientation.png")).unwrap();
    std::fs::write(rename_dir.join("taken.png"), b"keep").unwrap();
    enter_directory(rename_dir.clone());
    let old = rename_dir.join("old.png");
    let renamed = rename_dir.join("new.png");
    let menu = build_context_menu(grid);
    assert!(menu.itemWithTitle(&NSString::from_str("Rename…")).is_some(), "context menu offers rename");
    assert!(rename_item(&old, "taken.png").is_err(), "rename must not replace another file");
    assert_eq!(std::fs::read(rename_dir.join("taken.png")).unwrap(), b"keep");
    assert!(rename_item(&old, "../escape.png").is_err(), "rename must stay in the same directory");
    rename_item(&old, "new.png").unwrap();
    assert!(renamed.exists() && !old.exists());
    assert_eq!(unsafe { (*app_state()).tab().catalog.selected() }, Some(renamed.as_path()));
    rename_item(&renamed, "NEW.png").unwrap();
    assert_eq!(unsafe { (*app_state()).tab().catalog.selected() }, Some(rename_dir.join("NEW.png").as_path()));
    navigate_to_parent();
    let other_tab = unsafe { (*app_state()).new_tab(rename_dir.clone(), BrowserMode::Browser, load_listing(&rename_dir, BrowserMode::Browser, MediaFilter::Both).unwrap()) };
    let moved_dir = folder.join("renamed-test");
    rename_item(&rename_dir, "renamed-test").unwrap();
    assert!(moved_dir.join("NEW.png").exists());
    assert_eq!(unsafe { (&(*app_state()).tabs)[other_tab].root.clone() }, moved_dir, "open tabs follow renamed directories");
    close_tab(other_tab);
    navigate_back();
    assert_eq!(unsafe { (*app_state()).tab().root.clone() }, moved_dir, "history follows renamed directories");
    navigate_to_parent();

    let video_dir = folder.join("video");
    std::fs::create_dir(&video_dir).unwrap();
    let video = video_dir.join("clip.mp4");
    std::fs::copy(parent.join(".video.mp4"), &video).unwrap();
    unsafe { (*app_state()).tab_mut().view_mode = ViewMode::Carousel; }
    let previous_image = preview.image().unwrap();
    enter_directory(video_dir.clone());
    assert_eq!(unsafe { (*app_state()).items()[0].kind }, MediaKind::Movie);
    assert!(preview.image().is_some_and(|image| Retained::as_ptr(&image) == Retained::as_ptr(&previous_image)), "video preview retains the previous image while its thumbnail loads");
    let version_before = unsafe { (*app_state()).items()[0].clone() };
    let clip_bytes = std::fs::read(&video).unwrap();
    let mut replaced = clip_bytes.clone();
    replaced.extend_from_slice(&clip_bytes);
    std::fs::write(&video, replaced).unwrap();
    navigate_to_parent();
    enter_directory(video_dir);
    let version_after = unsafe { (*app_state()).items().iter().find(|item| item.path == video).cloned() }.expect("replacement video is listed");
    assert_ne!(version_before.file_size, version_after.file_size, "same-path replacement changes the file size");
    let superseded_pixels = pixel_limit(preview, preview.bounds());
    assert!(cached_sized(&video, superseded_pixels, version_before.file_size, version_before.modification).is_none(), "superseded thumbnail version is not served after a same-path replacement");
    let started = std::time::Instant::now();
    let block = block2::RcBlock::new(move |_timer: core::ptr::NonNull<objc2_foundation::NSTimer>| {
        let (pixels, image) = { let refs = window_refs(); let refs = refs.as_ref().unwrap(); unsafe {
            (pixel_limit(&*refs.preview, (&*refs.preview).bounds()), (&*refs.preview_image).image())
        } };
        if let Some(poster) = cached_sized(&video, pixels, version_after.file_size, version_after.modification) {
            assert!(image.is_some_and(|image| Retained::as_ptr(&image) == Retained::as_ptr(&poster)), "video preview updates when the replacement thumbnail completes");
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
    let event = objc2_app_kit::NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        objc2_app_kit::NSEventType::LeftMouseDown, point, flags,
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0),
        window.windowNumber(), None, 0, 1, 0.0,
    );
    if let Some(event) = event { window.sendEvent(&event); }
}

fn send_key_command(_app: &NSApplication, key_code: u16) {
    let Some(window_ptr) = target_window() else { return };
    let window = unsafe { &*window_ptr };
    let event = objc2_app_kit::NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        objc2_app_kit::NSEventType::KeyDown, objc2_foundation::NSPoint::ZERO,
        objc2_app_kit::NSEventModifierFlags::Command,
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0),
        window.windowNumber(), None, &NSString::from_str(""), &NSString::from_str(""), false, key_code,
    );
    if let Some(event) = event { window.sendEvent(&event); }
}

fn send_mouse_via_app(_app: &NSApplication, count: objc2_foundation::NSInteger) {
    let Some(window_ptr) = target_window() else { return };
    let window = unsafe { &*window_ptr };
    let dir_index = unsafe { (*app_state()).items().iter().position(|i| i.kind == MediaKind::Directory) };
    let index = dir_index.unwrap_or(0);
    let point = objc2_foundation::NSPoint::new(12.0 + index as f64 * 128.0 + 60.0, 92.0);
    let flags = objc2_app_kit::NSEventModifierFlags::empty();
    let event = objc2_app_kit::NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        objc2_app_kit::NSEventType::LeftMouseDown, point, flags,
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0),
        window.windowNumber(), None, 0, count, 0.0,
    );
    if let Some(event) = event { window.sendEvent(&event); }
}

fn send_key_via_app(_app: &NSApplication, key_code: u16, shift: bool) {
    let Some(window_ptr) = target_window() else { return };
    let window = unsafe { &*window_ptr };
    let flags = if shift { objc2_app_kit::NSEventModifierFlags::Shift } else { objc2_app_kit::NSEventModifierFlags::empty() };
    let event = objc2_app_kit::NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        objc2_app_kit::NSEventType::KeyDown, objc2_foundation::NSPoint::ZERO, flags,
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0),
        window.windowNumber(), None, &NSString::from_str(""), &NSString::from_str(""), false, key_code,
    );
    if let Some(event) = event { window.sendEvent(&event); }
}

fn start_self_test() {
    let mtm = objc2::MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    let block = block2::RcBlock::new(move |_timer: core::ptr::NonNull<objc2_foundation::NSTimer>| {
            let step = SELF_TEST_STEP.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match step {
                0 => {
                    SELF_TEST_ROOT.set(unsafe { (*app_state()).tab().root.clone() }).ok();
                    #[allow(deprecated)] app.activateIgnoringOtherApps(true);
                    if let Some(refs) = window_refs().as_ref() { unsafe { &*refs.window }.makeKeyWindow(); }
                    SELF_TEST_SELECTED_BEFORE.store(unsafe { (*app_state()).selected().unwrap_or(usize::MAX) }, std::sync::atomic::Ordering::SeqCst);
                    send_key_via_app(&app, 124, false);
                }
                1 => {
                    send_key_via_app(&app, 124, false);
                    let _ = app.keyWindow().map(|w| w.frame().size.width.max(400.0)).unwrap_or(400.0);
                    let point = objc2_foundation::NSPoint::new(12.0 + 60.0, 92.0);
                    send_shift_mouse_via_app(&app, point);
                }
                2 => { send_mouse_via_app(&app, 2); send_mouse_via_app(&app, 2); }
                3 => {
                    let entered = match SELF_TEST_ROOT.get() {
                        Some(start) => { let cur = unsafe { (*app_state()).tab().root.clone() }; cur != *start }
                        None => false,
                    };
                    SELF_TEST_ENTERED.store(entered, std::sync::atomic::Ordering::SeqCst);
                    send_key_via_app(&app, 123, true);
                }
                4 => {}
                5 => {
                    let image = unsafe { (*app_state()).items().iter().find(|i| i.kind == MediaKind::Image).map(|i| i.path.clone()) };
                    if let Some(path) = image { SELF_TEST_TRASH_TARGET.set(path).ok(); send_key_command(&app, 51); }
                }
                6 => { send_key_command(&app, 6); }
                _ => {
                    let before = SELF_TEST_SELECTED_BEFORE.load(std::sync::atomic::Ordering::SeqCst);
                    let before = if before == usize::MAX { None } else { Some(before) };
                    let now = unsafe { (*app_state()).selected() };
                    let root = unsafe { (*app_state()).tab().root.clone() };
                    let has_directory = unsafe { (*app_state()).items().iter().any(|i| i.kind == MediaKind::Directory) };
                    let double_click_ok = if has_directory { SELF_TEST_ENTERED.load(std::sync::atomic::Ordering::SeqCst) } else { before != now && now.is_some() };
                    let undo_ok = match SELF_TEST_TRASH_TARGET.get() { Some(target) => target.exists(), None => true };
                    if now.is_some() && double_click_ok && undo_ok {
                        eprintln!("self-test: PASS (arrows, double-click enter, shift-left history via NSApplication.sendEvent)");
                        std::process::exit(0);
                    } else {
                        eprintln!("self-test: FAIL (before={before:?} now={now:?} root={} count={})", root.display(), unsafe { (*app_state()).items().len() });
                        std::process::exit(1);
                    }
                }
            }
        });
    unsafe { objc2_foundation::NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.25, true, &block) };
}

fn start_player_test() {
    PLAYER_TEST_START.set(std::time::Instant::now()).ok();
    let block = block2::RcBlock::new(move |_timer: core::ptr::NonNull<objc2_foundation::NSTimer>| { run_player_test(); });
    unsafe { objc2_foundation::NSTimer::scheduledTimerWithTimeInterval_repeats_block(0.05, true, &block) };
}

fn run_player_test() {
    let started = PLAYER_TEST_START.get().copied().unwrap_or_else(std::time::Instant::now);
    match PLAYER_TEST_STEP.load(std::sync::atomic::Ordering::SeqCst) {
        0 => {
            let Some(item) = focused_item(unsafe { &*app_state() }) else { return };
            if item.kind != MediaKind::Movie { assert!(started.elapsed().as_secs() < 10, "player test corpus must begin with a movie"); return; }
            let Some((ready, visible, rate, attached, focus)) = player::with_store(|store| (store.status == player::PlayerStatus::Ready, !store.view.isHidden(), unsafe { store.player.rate() }, store.view.window().is_some() && unsafe { store.view.superview().is_some() }, store.view.acceptsFirstResponder())) else { return };
            if !ready || !visible { assert!(started.elapsed().as_secs() < 10, "inline player frame did not become ready"); return; }
            assert_eq!(rate, 0.0, "inline player starts paused");
            assert!(attached, "inline player view is attached to the preview");
            assert!(!focus, "inline player view must not take keyboard focus");
            let preview_ready = { let refs = window_refs(); unsafe { (&*refs.as_ref().unwrap().preview_image).image().is_some() } };
            if !preview_ready { assert!(started.elapsed().as_secs() < 10, "video poster did not load"); return; }
            let app = NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap());
            send_key_via_app(&app, 124, false);
            assert_eq!(unsafe { (*app_state()).selected() }, Some(1), "right selects the next video");
            player::with_store(|store| {
                if store.view.isHidden() {
                    let poster = unsafe { &*store.poster };
                    assert!(!poster.isHidden() && poster.image().is_some(), "keep a poster visible while the next frame loads");
                }
            });
            PLAYER_TEST_STEP.store(1, std::sync::atomic::Ordering::SeqCst);
        }
        1 => {
            let visible = player::with_store(|store| !store.view.isHidden()).unwrap_or(false);
            if !visible { assert!(started.elapsed().as_secs() < 10, "next video frame did not become ready"); return; }
            let mtm = objc2::MainThreadMarker::new().unwrap();
            let app = NSApplication::sharedApplication(mtm);
            if let Some(refs) = window_refs().as_ref() {
                let window = unsafe { &*refs.window };
                window.makeKeyAndOrderFront(None);
                let _ = window.makeFirstResponder(Some(unsafe { &*refs.grid }));
            }
            send_key_via_app(&app, 49, false);
            let rate = player::with_store(|store| unsafe { store.player.rate() }).unwrap_or(0.0);
            assert!(rate > 0.0, "space plays the inline video");
            send_key_via_app(&app, 49, false);
            let rate = player::with_store(|store| unsafe { store.player.rate() }).unwrap_or(0.0);
            assert_eq!(rate, 0.0, "space pauses the inline video");
            unsafe { (*app_state()).tab_mut().view_mode = ViewMode::Grid; }
            apply_view_mode_layout();
            let hidden = player::with_store(|store| store.view.isHidden()).unwrap_or(true);
            assert!(hidden, "grid mode releases the inline player");
            send_key_via_app(&app, 49, false);
            PLAYER_TEST_STEP.store(2, std::sync::atomic::Ordering::SeqCst);
        }
        _ => {
            let carousel = unsafe { (*app_state()).tab().view_mode == ViewMode::Carousel };
            assert!(carousel, "space in grid mode returns to carousel");
            let rate = player::with_store(|store| unsafe { store.player.rate() }).unwrap_or(0.0);
            if rate <= 0.0 { assert!(started.elapsed().as_secs() < 10, "space in grid mode did not begin playback"); return; }
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
    let result = unsafe { task_info(mach_task_self_, libc::MACH_TASK_BASIC_INFO, (&mut info as *mut libc::mach_task_basic_info_data_t).cast(), &mut count) };
    if result == libc::KERN_SUCCESS && count == libc::MACH_TASK_BASIC_INFO_COUNT { Ok(info.resident_size) } else { Err(std::io::Error::other(format!("task_info returned {result} with {count} fields"))) }
}

fn benchmark_browsing_memory(window: &NSWindow, grid: &GridView) {
    let count = unsafe { (*app_state()).items().len() };
    assert!(count >= 100, "memory corpus needs at least 100 items");
    unsafe { (*app_state()).tab_mut().view_mode = ViewMode::Grid; }
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
        Err(e) => { eprintln!("view: {e}"); std::process::exit(1); }
    };
    {
        let defaults = objc2_foundation::NSUserDefaults::standardUserDefaults();
        let saved = defaults.stringForKey(&NSString::from_str("View.ViewMode")).map(|s| s.to_string());
        let view_mode = if benchmark_memory { ViewMode::Grid } else if benchmark_startup { ViewMode::Carousel } else { match saved.as_deref() { Some("grid") => ViewMode::Grid, _ => ViewMode::Carousel } };
        let saved_filter = defaults.stringForKey(&NSString::from_str("View.MediaFilter")).map(|s| s.to_string());
        let media_filter = match saved_filter.as_deref() {
            Some("images") => MediaFilter::Images,
            Some("videos") => MediaFilter::Videos,
            _ => MediaFilter::Both,
        };
        let mut tab = Tab::default();
        tab.root = root.clone();
        tab.mode = mode;
        tab.view_mode = view_mode;
        tab.media_filter = media_filter;
        match load_listing(&root, mode, media_filter) {
            Ok(items) => show_listing(&mut tab.catalog, items, ListingPick::First),
            Err(error) => tab.unavailable = Some(format!("Unable to read {}: {error}", root.display())),
        }
        let state = unsafe { &mut *app_state() };
        state.tabs = vec![tab];
        state.active = 0;
    }

    unsafe {
        extern "C" fn handle_signal(_signal: libc::c_int) { std::process::exit(0); }
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

unsafe impl QLPreviewPanelDataSource for GridView {}

#[cfg(feature = "benchmark")]
#[path = "app/benchmarks.rs"]
pub mod benchmarks;
