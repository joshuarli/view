#![cfg(target_os = "macos")]

#[test]
fn app_interactions() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".work/tmp/app-regression");
    if root.exists() { std::fs::remove_dir_all(&root).unwrap(); }
    let folder = root.join("folder");
    std::fs::create_dir_all(&folder).unwrap();
    let png: &[u8] = &[
        137,80,78,71,13,10,26,10,0,0,0,13,73,72,68,82,0,0,0,1,0,0,0,1,8,6,0,0,0,31,21,196,137,
        0,0,0,13,73,68,65,84,8,215,99,248,207,192,240,31,0,5,0,1,255,137,153,61,29,0,0,0,0,73,69,78,68,174,66,96,130,
    ];
    for name in ["a.png", "b.png", "c.png"] { std::fs::write(folder.join(name), png).unwrap(); }
    std::fs::write(folder.join("weird % café.png"), png).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_app"))
        .arg("--regression-test").arg(&root).output().unwrap();
    assert!(output.status.success(), "app failed: {}", String::from_utf8_lossy(&output.stderr));
    std::fs::remove_dir_all(root).unwrap();
}
