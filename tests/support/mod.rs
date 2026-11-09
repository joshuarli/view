use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

pub struct TestDirectory(PathBuf);

impl TestDirectory {
    pub fn new(tag: &str) -> Self {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join(".work/tmp");
        std::fs::create_dir_all(&parent).unwrap();
        let path = parent.join(format!("view-test-{}-{}-{tag}", std::process::id(), NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl std::ops::Deref for TestDirectory {
    type Target = Path;

    fn deref(&self) -> &Path { &self.0 }
}

impl AsRef<Path> for TestDirectory {
    fn as_ref(&self) -> &Path { &self.0 }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            if std::thread::panicking() {
                eprintln!("failed to remove test directory {}: {error}", self.0.display());
            } else {
                panic!("failed to remove test directory {}: {error}", self.0.display());
            }
        }
    }
}
