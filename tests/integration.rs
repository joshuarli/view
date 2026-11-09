mod support;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use view_core::{
    list_browser, scan_recursive, Catalog, MediaKind, MediaItem, Scheduler,
    ThumbKey, TrashHistory, TrashOutcome, TrashService,
    TrashTransaction, TrashedFile,
};

fn make_file(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn trash_then_undo_then_redo_moves_files_on_disk() {
    let dir = support::TestDirectory::new("trash");
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

    let mut history = TrashHistory::new();
    history.record_trash(TrashTransaction {
        files: vec![TrashedFile {
            item: MediaItem::new(&a, MediaKind::Image),
            trashed_path: trash_dir.join("a.png"),
            index: 0,
        }],
        selection_before_op: vec![a.clone()],
    });
    let undo = history.pop_undo().unwrap();
    let file = &undo.files[0];
    service.restore(&file.trashed_path, &file.item.path, |from, to| {
        std::fs::rename(from, to).map_err(|e| view_core::TrashError::MoveFailed(e.to_string()))
    }).unwrap();
    assert_eq!(std::fs::read(&a).unwrap(), b"aaa");
    assert!(!file.trashed_path.exists());
    history.push_redo(undo);
    let redo = history.pop_redo().unwrap();
    let file = &redo.files[0];
    let outcome = trash_one(&mut service, &file.item.path);
    assert_eq!(outcome, TrashOutcome::Removed { trashed_path: file.trashed_path.clone() });
    assert!(!a.exists());
    assert_eq!(std::fs::read(&file.trashed_path).unwrap(), b"aaa");
    assert_eq!(std::fs::read(&b).unwrap(), b"bbb");
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
    let dir = support::TestDirectory::new("discovery");
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
}

#[test]
fn catalog_selection_survives_listing_rebuild() {
    let dir = support::TestDirectory::new("catalog");
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
}
