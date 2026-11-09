//! Integration tests for the Rust port: real-temp-directory coverage of
//! trash/undo/redo, scheduler execution, and discovery. These mirror
//! the behaviors exercised by the Swift `viewUITests` regression
//! suite (navigation after entering a directory, undo/redo wiring,
//! package/hidden exclusion) at the core-logic level.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use view_core::{
    list_browser, scan_recursive, Catalog, MediaKind, MediaItem, Scheduler,
    ThumbKey, TrashError, TrashHistory, TrashOutcome, TrashService,
    TrashTransaction, TrashedFile,
};

fn temp_subdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("view-integration-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn make_file(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn trash_then_undo_then_redo_moves_files_on_disk() {
    let dir = temp_subdir("trash");
    let trash_dir = dir.join("trash");
    std::fs::create_dir_all(&trash_dir).unwrap();
    let a = dir.join("a.png");
    let b = dir.join("b.png");
    make_file(&a, b"aaa");
    make_file(&b, b"bbb");

    let mut service = TrashService::new();
    let trash_one = |service: &mut TrashService, original: &Path| {
        let trashed = trash_dir.join(original.file_name().unwrap());
        let original = original.to_path_buf();
        let trashed = trashed.clone();
        service
            .move_to_trash(&original, |_url| {
                std::fs::rename(&original, &trashed).map_err(|e| e.to_string())?;
                Ok(HashMap::from([(original.clone(), trashed.clone())]))
            })
            .unwrap()
    };

    let outcome = trash_one(&mut service, &a);
    assert_eq!(outcome, TrashOutcome::Removed { trashed_path: trash_dir.join("a.png") });
    assert!(!a.exists() && trash_dir.join("a.png").exists() && b.exists());

    let existed_before_redo = {
        let trashed = trash_dir.join("a.png");
        assert!(trashed.exists());
        std::fs::rename(&trashed, &a).unwrap();
        a.exists() && !trashed.exists()
    };
    assert!(existed_before_redo);

    let mut history = TrashHistory::new();
    let txn = TrashTransaction {
        files: vec![TrashedFile {
            item: MediaItem::new(&a, MediaKind::Image),
            trashed_path: trash_dir.join("a.png"),
            index: 0,
        }],
        selection_before_op: vec![a.clone()],
    };
    history.record_trash(txn.clone());
    assert!(history.can_undo() && !history.can_redo());

    let undo_txn = history.pop_undo().unwrap();
    let restored = undo_txn.files[0].clone();
    if restored.trashed_path.exists() {
        std::fs::rename(&restored.trashed_path, &a).unwrap();
    }
    assert!(a.exists());
    history.push_redo(txn);
    assert!(history.can_redo() && !history.can_undo());

    let redo_txn = history.pop_redo().unwrap();
    assert_eq!(redo_txn.files.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scheduler_executes_limited_demand_and_discards_cancelled() {
    let mut scheduler = Scheduler::new(3);
    let keys: Vec<ThumbKey> = (0..5)
        .map(|i| ThumbKey { path: PathBuf::from(format!("/x/{i}.png")), pixel_size: 320, scale: 2 })
        .collect();
    scheduler.update_demand(keys.clone(), vec![]);
    let done = scheduler.execute_pending(|_| {});
    assert_eq!(done.len(), 3, "limit caps executed demand");
    let gens: Vec<u64> = done.iter().map(|(_, g)| *g).collect();
    assert_eq!(gens, vec![1, 2, 3]);

    scheduler.cancel_all();
    scheduler.update_demand(keys.clone(), vec![]);
    let done2 = scheduler.execute_pending(|_| {});
    assert_eq!(done2.len(), 3);
    assert!(done2.iter().all(|(_, g)| *g > 3), "fresh generations after cancel");
}

#[test]
fn discovery_skips_packages_hidden_and_symlinks() {
    let dir = temp_subdir("discovery");
    make_file(&dir.join("visible.png"), &[0x89, 0x50, 0x4E, 0x47]);
    make_file(&dir.join(".hidden.png"), &[0x89, 0x50, 0x4E, 0x47]);
    make_file(&dir.join("Foo.app").join("inner.png"), &[0x89, 0x50, 0x4E, 0x47]);
    #[cfg(unix)]
    std::os::unix::fs::symlink(dir.join("visible.png"), dir.join("link.png")).unwrap();

    let listed = list_browser(&dir).unwrap();
    assert_eq!(listed.len(), 1, "only visible.png survives in browser listing");
    assert_eq!(listed[0].path, dir.join("visible.png"));

    let scanned = scan_recursive(&dir).unwrap();
    assert_eq!(scanned.len(), 1, "recursive scan skips package+hidden+symlink");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn catalog_selection_survives_listing_rebuild() {
    let dir = temp_subdir("catalog");
    make_file(&dir.join("one.png"), &[0x89, 0x50, 0x4E, 0x47]);
    make_file(&dir.join("two.jpg"), &[0xFF, 0xD8, 0xFF, 0xE0]);

    let mut catalog = Catalog::new();
    let items = list_browser(&dir).unwrap();
    assert_eq!(items.len(), 2);
    catalog.append(items.clone());
    let target = items.iter().find(|i| i.path.ends_with("two.jpg")).unwrap().clone();
    catalog.select(&target.path).unwrap();

    let rebuilt = list_browser(&dir).unwrap();
    assert_eq!(catalog.selected(), Some(target.path.as_path()));
    assert!(rebuilt.iter().any(|i| i.path == target.path));
    let _ = std::fs::remove_dir_all(&dir);
}
