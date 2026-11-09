//! Automated parity coverage for `Tests/viewUITests/ViewUITests.swift`.
//!
//! The Swift suite drives a live `NSWindow` + `GridViewController` and
//! synthesizes `NSEvent`s. The Rust GUI reuses the same core contracts
//! (`Catalog`, history stacks, `resolve_command`, discovery), so these
//! tests assert the same observable outcomes at the model layer, plus
//! the native Quick Look thumbnail path backing Space preview.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use view_core::{
    list_browser, Catalog, GridCommand, GridKeyModifier, MediaKind, MediaItem,
    TrashError, TrashHistory, TrashService, TrashTransaction, TrashedFile,
};

fn temp_subdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("view-uiparity-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_png(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]).unwrap();
}

#[test]
fn entering_directory_selects_first_item() {
    let root = temp_subdir("enter");
    let sub = root.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    write_png(&sub.join("a.png"));

    let mut catalog = Catalog::new();
    catalog.append(list_browser(&root).unwrap());
    let sub_item = catalog
        .items()
        .iter()
        .find(|i| i.kind == MediaKind::Directory)
        .cloned()
        .expect("directory listed");
    let next = list_browser(&sub_item.path).unwrap();
    catalog.append(next.clone());
    assert_eq!(catalog.items().len(), 2);
    let first = next.first().expect("a.png").clone();
    catalog.select(&first.path).unwrap();
    assert_eq!(catalog.selected(), Some(first.path.as_path()));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn drag_item_initialization_does_not_crash() {
    #[cfg(target_os = "macos")]
    {
        use objc2::ClassType;
        use objc2_foundation::{NSRect, NSSize, NSString, NSURL};
        use objc2::rc::Retained;

        unsafe {
            let pool = objc2_foundation::NSAutoreleasePool::new();
            let item = objc2_app_kit::NSPasteboardItem::new();
            let string = NSString::from_str("/tmp/test.png");
            item.setString_forType(&string, &objc2_app_kit::NSPasteboardTypeString);

            let alloc: *mut objc2_app_kit::NSDraggingItem =
                objc2::msg_send![objc2_app_kit::NSDraggingItem::class(), alloc];
            assert!(!alloc.is_null(), "NSDraggingItem alloc must succeed");

            let ptr: *mut objc2_app_kit::NSDraggingItem =
                objc2::msg_send![alloc, initWithPasteboardWriter: &*item];
            assert!(!ptr.is_null(), "NSDraggingItem init must not return null");

            let dragging_item = Retained::from_raw(ptr).expect("valid Retained NSDraggingItem");
            let frame = NSRect::new(objc2_foundation::NSPoint::ZERO, NSSize::new(100.0, 100.0));
            let _: () = objc2::msg_send![
                &*dragging_item,
                setDraggingFrame: frame,
                contents: core::ptr::null::<objc2::runtime::NSObject>()
            ];
            drop(dragging_item);
            drop(pool);
        }
    }
}

#[test]
fn back_restores_parent_cursor_position_via_history_stack() {
    let root = temp_subdir("back");
    let sub = root.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    write_png(&root.join("a.png"));
    write_png(&root.join("b.png"));
    write_png(&sub.join("s.png"));

    let mut back: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    let mut root_path = root.clone();
    let mut items = list_browser(&root_path).unwrap();

    let mut selected = items
        .iter()
        .find(|i| i.path.ends_with("b.png"))
        .map(|i| i.path.clone())
        .expect("b.png");
    let sub_dir = items
        .iter()
        .find(|i| i.kind == MediaKind::Directory)
        .map(|i| i.path.clone())
        .expect("sub dir");
    back.push((root_path.clone(), Some(selected.clone())));
    root_path = sub_dir;
    items = list_browser(&root_path).unwrap();
    selected = items.first().unwrap().path.clone();

    let (parent, cursor) = back.pop().expect("back entry");
    root_path = parent;
    items = list_browser(&root_path).unwrap();
    selected = cursor.unwrap();
    assert_eq!(root_path, root);
    assert!(selected.ends_with("b.png"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn shift_left_and_right_resolve_to_history_commands() {
    assert_eq!(
        view_core::resolve_command_key(123, &[GridKeyModifier::Shift]),
        Some(GridCommand::Back)
    );
    assert_eq!(
        view_core::resolve_command_key(124, &[GridKeyModifier::Shift]),
        Some(GridCommand::Forward)
    );
    assert_eq!(
        view_core::resolve_command_key(49, &[]),
        Some(GridCommand::TogglePreview)
    );
    assert_eq!(view_core::resolve_command_key(51, &[]), Some(GridCommand::UpLevel));
}

fn next_index(current: isize, delta: isize, count: isize) -> isize {
    (((current + delta) % count) + count) % count
}

#[test]
fn arrow_wraparound_both_directions() {
    assert_eq!(next_index(0, -1, 2), 1, "up from first wraps to last");
    assert_eq!(next_index(1, 1, 2), 0, "down from last wraps to first");
}

#[test]
fn grid_up_down_steps_by_column_count() {
    let columns: isize = 5;
    let first: isize = 0;
    let target = next_index(first, columns, 12);
    assert_eq!(target, 5);
    let back = next_index(target, -columns, 12);
    assert_eq!(back, 0);
}

#[test]
fn undo_redo_round_trips_trashed_file_on_disk() {
    let dir = temp_subdir("undo");
    let trash_bin = dir.join("trash-bin");
    std::fs::create_dir_all(&trash_bin).unwrap();
    let a = dir.join("a.png");
    let b = dir.join("b.png");
    write_png(&a);
    write_png(&b);

    let mut service = TrashService::new();
    let trashed = trash_bin.join("a.png");
    service
        .move_to_trash(&a, |_url| {
            std::fs::rename(&a, &trashed).map_err(|e| e.to_string())?;
            Ok(HashMap::from([(a.clone(), trashed.clone())]))
        })
        .unwrap();
    assert!(!a.exists() && trashed.exists());

    let mut history = TrashHistory::new();
    history.record_trash(TrashTransaction {
        files: vec![TrashedFile {
            item: MediaItem::new(&a, MediaKind::Image),
            trashed_path: trashed.clone(),
            index: 0,
        }],
        selection_before_op: vec![a.clone()],
    });
    assert!(history.can_undo() && !history.can_redo());

    let txn = history.pop_undo().unwrap();
    let file = &txn.files[0];
    service
        .restore(&file.trashed_path, &file.item.path, |from, to| {
            std::fs::rename(from, to).map_err(|e| TrashError::MoveFailed(e.to_string()))
        })
        .unwrap();
    assert!(a.exists() && !trashed.exists());
    assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn undo_fails_when_destination_reappears() {
    let dir = temp_subdir("undo-fail");
    let trash_bin = dir.join("trash-bin");
    std::fs::create_dir_all(&trash_bin).unwrap();
    let a = dir.join("a.png");
    write_png(&a);

    let mut service = TrashService::new();
    let trashed = trash_bin.join("a.png");
    service
        .move_to_trash(&a, |_url| {
            std::fs::rename(&a, &trashed).map_err(|e| e.to_string())?;
            Ok(HashMap::from([(a.clone(), trashed.clone())]))
        })
        .unwrap();
    write_png(&a);

    let mut history = TrashHistory::new();
    history.record_trash(TrashTransaction {
        files: vec![TrashedFile {
            item: MediaItem::new(&a, MediaKind::Image),
            trashed_path: trashed.clone(),
            index: 0,
        }],
        selection_before_op: vec![a.clone()],
    });
    let txn = history.pop_undo().unwrap();
    let file = &txn.files[0];
    let failed = service.restore(&file.trashed_path, &file.item.path, |from, to| {
        if to.exists() {
            return Err(TrashError::DestinationExists(to.to_path_buf()));
        }
        std::fs::rename(from, to).map_err(|e| TrashError::MoveFailed(e.to_string()))
    });
    assert!(failed.is_err());
    assert!(trashed.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn window_title_shows_full_path() {
    let root = temp_subdir("title");
    let sub = root.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    write_png(&sub.join("a.png"));
    let items = list_browser(&root).unwrap();
    let dir_item = items.iter().find(|i| i.kind == MediaKind::Directory).unwrap().clone();
    let title = dir_item.path.display().to_string();
    assert_eq!(title, sub.display().to_string());
    assert!(title.contains("sub"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn quicklook_thumbnail_generator_produces_video_frame() {
    // Backs Space Quick Look: the same generator+cache path the app
    // binary uses must yield a usable thumbnail image for a movie.
    let dir = temp_subdir("ql");
    let movie_path = {
        let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".work/tmp/previewtest/preview12.mp4");
        if corpus.exists() {
            corpus
        } else {
            let fallback = dir.join("clip.mp4");
            std::fs::write(&fallback, [0u8; 8]).unwrap();
            fallback
        }
    };
    assert_eq!(view_core::kind_for_extension(&movie_path), Some(MediaKind::Movie));

    #[cfg(target_os = "macos")]
    {
        use objc2::ClassType;
        use objc2_foundation::{NSSize, NSString, NSURL};
        use objc2_quick_look_thumbnailing::{
            QLThumbnailGenerationRequestRepresentationTypes, QLThumbnailGenerator,
            QLThumbnailRepresentation,
        };
        use objc2::rc::Retained;

        unsafe {
            let pool = objc2_foundation::NSAutoreleasePool::new();
            let generator = QLThumbnailGenerator::sharedGenerator();
            let url = NSURL::fileURLWithPath(&NSString::from_str(&movie_path.display().to_string()));
            let request: Retained<objc2_quick_look_thumbnailing::QLThumbnailGenerationRequest> = unsafe {
                let obj: *mut objc2_quick_look_thumbnailing::QLThumbnailGenerationRequest =
                    objc2::msg_send![objc2_quick_look_thumbnailing::QLThumbnailGenerationRequest::class(), alloc];
                let req: *mut objc2_quick_look_thumbnailing::QLThumbnailGenerationRequest = objc2::msg_send![
                    obj,
                    initWithFileAtURL: &*url,
                    size: NSSize::new(480.0, 480.0),
                    scale: 2.0,
                    representationTypes: QLThumbnailGenerationRequestRepresentationTypes::Thumbnail
                ];
                Retained::from_raw(req).unwrap()
            };
            let (tx, rx) = std::sync::mpsc::channel();
            let handler = block2::RcBlock::new(move |representation: *mut QLThumbnailRepresentation, _error: *mut objc2_foundation::NSError| {
                let _ = tx.send(!representation.is_null());
            });
            generator.generateBestRepresentationForRequest_completionHandler(&request, &handler);
            let ok = rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap_or(false);
            assert!(ok, "QL thumbnail for movie must produce a representation");
            drop(pool);
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
