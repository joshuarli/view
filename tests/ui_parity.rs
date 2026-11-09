mod support;

use std::collections::HashMap;
use std::path::Path;

use view_core::{
    GridCommand, GridKeyModifier, MediaKind, MediaItem,
    TrashError, TrashHistory, TrashService, TrashTransaction, TrashedFile,
};

fn write_png(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]).unwrap();
}

#[test]
fn drag_item_initialization_does_not_crash() {
    #[cfg(target_os = "macos")]
    {
        use objc2::ClassType;
        use objc2_foundation::{NSRect, NSSize, NSString};
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

#[test]
fn undo_fails_when_destination_reappears() {
    let dir = support::TestDirectory::new("undo-fail");
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
}

#[test]
fn quicklook_thumbnail_generator_produces_video_frame() {
    let dir = support::TestDirectory::new("ql");
    let movie_path = dir.join("clip.mp4");
    std::fs::write(&movie_path, include_bytes!("fixtures/carousel.mp4")).unwrap();
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
}
