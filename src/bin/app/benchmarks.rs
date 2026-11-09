use super::*;
use rustybench::{black_box, Bencher};
use std::sync::atomic::{AtomicBool, Ordering};

static ACTIVE: AtomicBool = AtomicBool::new(false);

pub(super) fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

fn corpus(variable: &str) -> PathBuf {
    let path = std::env::var_os(variable).unwrap_or_else(|| panic!("{variable} must name a benchmark corpus directory"));
    let path = PathBuf::from(path);
    assert!(path.is_dir(), "{variable} must name an existing directory");
    path
}

fn browser_corpus(variable: &str, min: usize) -> PathBuf {
    let path = corpus(variable);
    let listing = list_browser(&path).expect("list browser corpus");
    assert!(listing.len() >= min, "{variable} must contain at least {min} visible items");
    assert!(listing.iter().any(|item| item.kind == MediaKind::Image), "{variable} must contain images");
    path
}

fn browser_listing(bencher: Bencher, variable: &str, min: usize) {
    let path = browser_corpus(variable, min);
    let expected = list_browser(&path).expect("list browser corpus").len();
    bencher.bench_local(|| {
        let items = list_browser(black_box(&path)).expect("list browser corpus");
        assert_eq!(items.len(), expected, "browser listing changed during measurement");
        black_box(items)
    });
}

#[rustybench::bench(sample_size = 1)]
fn open_small_directory(bencher: Bencher) {
    browser_listing(bencher, "VIEW_BENCH_SMALL", 20);
}

#[rustybench::bench(sample_size = 1)]
fn open_typical_directory(bencher: Bencher) {
    browser_listing(bencher, "VIEW_BENCH_TYPICAL", 100);
}

#[rustybench::bench(sample_size = 1)]
fn recursive_media_scan(bencher: Bencher) {
    let root = corpus("VIEW_BENCH_LARGE");
    let expected = scan_recursive(&root).expect("scan corpus").len();
    assert!(expected >= 1000, "VIEW_BENCH_LARGE must contain at least 1000 media files");
    bencher.bench_local(|| {
        let items = scan_recursive(black_box(&root)).expect("scan corpus");
        assert_eq!(items.len(), expected, "recursive corpus changed during measurement");
        black_box(items)
    });
}

#[rustybench::bench(sample_count = 10, sample_size = 1)]
fn launch_and_paint_first_image(bencher: Bencher) {
    let root = browser_corpus("VIEW_BENCH_SMALL", 20);
    let items = list_browser(&root).expect("list startup corpus");
    assert_eq!(items[0].kind, MediaKind::Image, "VIEW_BENCH_SMALL must begin with an image");
    let app = std::env::var_os("VIEW_BENCH_APP").expect("VIEW_BENCH_APP must name the dist app binary");
    bencher.bench_local(|| {
        let output = std::process::Command::new(black_box(&app))
            .arg("--benchmark-startup").arg(black_box(&root)).output().expect("launch app");
        assert!(output.status.success(), "startup failed: {}", String::from_utf8_lossy(&output.stderr));
        black_box(output.status);
    });
}

fn prepare_window() -> (&'static NSWindow, &'static GridView) {
    ACTIVE.store(true, Ordering::Relaxed);
    let root = browser_corpus("VIEW_BENCH_TYPICAL", 100);
    let state = unsafe { &mut *app_state() };
    state.root = root.clone();
    state.mode = BrowserMode::Browser;
    state.view_mode = ViewMode::Grid;
    state.items = list_browser(&root).expect("list browser corpus");
    state.selected = Some(0);
    state.selection_anchor = Some(0);
    state.selection_range = (0, 0);
    let mtm = objc2::MainThreadMarker::new().expect("AppKit benchmarks must run on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    let needs_window = { window_refs().is_none() };
    if needs_window {
        let delegate: Retained<AppDelegate> = unsafe { objc2::msg_send![AppDelegate::class(), new] };
        app.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*delegate)));
        let notification = unsafe { NSNotification::notificationWithName_object(&NSString::from_str("NSApplicationDidFinishLaunchingNotification"), None) };
        unsafe { let _: () = objc2::msg_send![&*delegate, applicationDidFinishLaunching: &*notification]; }
        let _ = Retained::into_raw(delegate);
    }
    let (window, grid) = {
        let refs = window_refs();
        let refs = refs.as_ref().expect("benchmark window");
        (refs.window, refs.grid)
    };
    let window = unsafe { &*window };
    let grid = unsafe { &*grid };
    window.setContentSize(NSSize::new(1000.0, 700.0));
    apply_view_mode_layout();
    (window, grid)
}

fn bench_appkit(bencher: Bencher, mut operation: impl FnMut()) {
    bencher.bench_local(|| objc2::rc::autoreleasepool(|_| operation()));
}

fn capture(grid: &GridView, bitmap: &objc2_app_kit::NSBitmapImageRep) {
    let visible = grid.visibleRect();
    assert!(visible.size.width > 0.0 && visible.size.height > 0.0, "empty viewport");
    grid.setNeedsDisplay(true);
    grid.cacheDisplayInRect_toBitmapImageRep(visible, bitmap);
    black_box(bitmap.bitmapData());
}

#[rustybench::bench(sample_size = 1)]
fn open_directory_and_render_grid(bencher: Bencher) {
    let (_, grid) = prepare_window();
    let root = unsafe { (*app_state()).root.clone() };
    let expected = unsafe { (*app_state()).items.len() };
    let visible = grid.visibleRect();
    let bitmap = grid.bitmapImageRepForCachingDisplayInRect(visible).expect("viewport bitmap");
    bench_appkit(bencher, || {
        let items = list_browser(black_box(&root)).expect("list browser corpus");
        assert_eq!(items.len(), expected, "browser listing changed during measurement");
        let state = unsafe { &mut *app_state() };
        state.items = items;
        state.selected = Some(0);
        state.selection_anchor = Some(0);
        state.selection_range = (0, 0);
        apply_view_mode_layout();
        capture(grid, &bitmap);
    });
}

#[rustybench::bench(sample_size = 1)]
fn scroll_and_render_grid(bencher: Bencher) {
    let (_, grid) = prepare_window();
    let visible = grid.visibleRect();
    let bitmap = grid.bitmapImageRepForCachingDisplayInRect(visible).expect("viewport bitmap");
    let count = unsafe { (*app_state()).items.len() };
    let origin = grid.visibleRect().origin.y;
    grid.scrollRectToVisible(grid.cell_rect(24));
    assert_ne!(grid.visibleRect().origin.y, origin, "scroll did not change the visible rows");
    grid.scrollRectToVisible(grid.cell_rect(0));
    let mut index = 0;
    capture(grid, &bitmap);
    bench_appkit(bencher, || {
        index = (index + 24) % count;
        let rect = grid.cell_rect(index);
        grid.scrollRectToVisible(rect);
        capture(grid, &bitmap);
        black_box(index);
    });
}

#[rustybench::bench(sample_size = 1)]
fn resize_and_render_grid(bencher: Bencher) {
    let (window, grid) = prepare_window();
    let bitmap = grid.bitmapImageRepForCachingDisplayInRect(NSRect::new(NSPoint::ZERO, NSSize::new(1000.0, 700.0))).expect("viewport bitmap");
    let mut wide = false;
    bench_appkit(bencher, || {
        wide = !wide;
        window.setContentSize(if wide { NSSize::new(1000.0, 700.0) } else { NSSize::new(800.0, 700.0) });
        capture(grid, &bitmap);
        black_box(grid.bounds().size);
    });
}

#[rustybench::bench(sample_size = 1)]
fn advance_carousel_preview(bencher: Bencher) {
    let (window, grid) = prepare_window();
    unsafe { (*app_state()).view_mode = ViewMode::Carousel; }
    apply_view_mode_layout();
    window.makeKeyAndOrderFront(None);
    assert!(window.makeFirstResponder(Some(grid)), "grid must receive keyboard events");
    let event = objc2_app_kit::NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        objc2_app_kit::NSEventType::KeyDown, NSPoint::ZERO,
        objc2_app_kit::NSEventModifierFlags::empty(), 0.0, window.windowNumber(), None,
        &NSString::from_str("\u{f703}"), &NSString::from_str("\u{f703}"), false, 124,
    ).expect("right arrow event");
    let visible = grid.visibleRect();
    let bitmap = grid.bitmapImageRepForCachingDisplayInRect(visible).expect("carousel bitmap");
    let (preview, image_view) = {
        let refs = window_refs();
        let refs = refs.as_ref().expect("benchmark window");
        unsafe { (&*refs.preview, &*refs.preview_image) }
    };
    let preview_bounds = preview.bounds();
    let preview_bitmap = preview.bitmapImageRepForCachingDisplayInRect(preview_bounds).expect("preview bitmap");
    let items = unsafe { &(*app_state()).items };
    let image_pairs: Vec<usize> = items.windows(2).enumerate()
        .filter_map(|(index, pair)| (pair[0].kind == MediaKind::Image && pair[1].kind == MediaKind::Image).then_some(index))
        .collect();
    assert!(!image_pairs.is_empty(), "typical corpus needs two adjacent images");
    let mut step = 0;
    bench_appkit(bencher, || {
        let current = image_pairs[step % image_pairs.len()];
        step += 1;
        let state = unsafe { &mut *app_state() };
        state.selected = Some(current);
        state.selection_anchor = Some(current);
        state.selection_range = (current, current);
        window.sendEvent(&event);
        assert_eq!(unsafe { (*app_state()).selected }, Some(current + 1), "carousel arrow did not advance");
        assert!(image_view.image().is_some(), "carousel preview is empty");
        capture(grid, &bitmap);
        preview.cacheDisplayInRect_toBitmapImageRep(preview_bounds, &preview_bitmap);
        black_box(preview_bitmap.bitmapData());
    });
}

#[rustybench::bench(sample_size = 1)]
fn generate_video_thumbnail(bencher: Bencher) {
    let (_, _) = prepare_window();
    let path = unsafe { (&(*app_state()).items).iter().find(|item| item.kind == MediaKind::Movie) }
        .expect("VIEW_BENCH_TYPICAL must contain a video").path.clone();
    let run_loop = objc2_foundation::NSRunLoop::currentRunLoop();
    bench_appkit(bencher, || {
        thumb_cache().remove(&path);
        request_quicklook_thumbnail(path.clone());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if let Some(image) = cached_thumbnail(&path) {
                assert!(image.size().width > 0.0 && image.size().height > 0.0, "video thumbnail is empty");
                black_box(image);
                break;
            }
            assert!(thumb_pending().contains(&path), "video thumbnail generation failed");
            assert!(std::time::Instant::now() < deadline, "video thumbnail timed out");
            let until = objc2_foundation::NSDate::dateWithTimeIntervalSinceNow(0.05);
            run_loop.runMode_beforeDate(unsafe { objc2_foundation::NSDefaultRunLoopMode }, &until);
        }
    });
}
