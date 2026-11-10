#![cfg(target_os = "macos")]

mod support;

struct TestProcess(std::process::Child);

impl Drop for TestProcess {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[test]
fn app_interactions() {
    let root = support::TestDirectory::new("app-regression");
    let folder = root.join("folder");
    std::fs::create_dir_all(&folder).unwrap();
    let png = include_bytes!("fixtures/orientation.png");
    for name in ["a.png", "b.png", "c.png", "d.png", "e.png", "f.png", "g.png", "h.png"] { std::fs::write(folder.join(name), png).unwrap(); }
    std::fs::write(folder.join("weird % café.png"), png).unwrap();
    std::fs::write(root.join(".video.mp4"), include_bytes!("fixtures/carousel.mp4")).unwrap();
    let stderr_path = root.join(".stderr");
    let stderr = std::fs::File::create(&stderr_path).unwrap();
    let mut child = TestProcess(std::process::Command::new(env!("CARGO_BIN_EXE_app"))
        .arg("--regression-test").arg(&*root)
        .stdout(std::process::Stdio::null()).stderr(stderr).spawn().unwrap());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() { break status; }
        assert!(std::time::Instant::now() < deadline, "app regression timed out");
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let stderr = std::fs::read_to_string(stderr_path).unwrap();
    assert!(status.success(), "app failed: {stderr}");
}

#[test]
fn inline_video_playback() {
    let root = support::TestDirectory::new("player");
    std::fs::write(root.join("clip.mp4"), include_bytes!("fixtures/carousel.mp4")).unwrap();
    let stderr_path = root.join(".stderr");
    let stderr = std::fs::File::create(&stderr_path).unwrap();
    let mut child = TestProcess(std::process::Command::new(env!("CARGO_BIN_EXE_app"))
        .arg("--player-test").arg(&*root)
        .stdout(std::process::Stdio::null()).stderr(stderr).spawn().unwrap());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() { break status; }
        assert!(std::time::Instant::now() < deadline, "player test timed out");
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let stderr = std::fs::read_to_string(stderr_path).unwrap();
    assert!(status.success(), "player test failed: {stderr}");
}
