use std::path::Path;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;
use tempfile::tempdir;

fn bin() -> Command {
    Command::new(binary_path())
}

fn binary_path() -> PathBuf {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_rs-torrent") {
        return PathBuf::from(path);
    }

    let target_dir = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));

    let exe = if cfg!(windows) {
        "rs-torrent.exe"
    } else {
        "rs-torrent"
    };
    target_dir.join("debug").join(exe)
}

fn maybe_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn run_download(output: &Path, source: &str) -> std::process::Output {
    bin()
        .args([
            "download",
            "-o",
            output.to_str().expect("utf8 path"),
            source,
        ])
        .output()
        .expect("download command should spawn")
}

fn spawn_download(output: &Path, source: &str) -> Child {
    bin()
        .args([
            "download",
            "-o",
            output.to_str().expect("utf8 path"),
            source,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("download command should spawn")
}

#[test]
#[ignore = "requires RS_TORRENT_LIVE_TORRENT and network access"]
fn live_torrent_download_smoke() {
    let Some(source) = maybe_env("RS_TORRENT_LIVE_TORRENT") else {
        eprintln!("skipping: RS_TORRENT_LIVE_TORRENT not set");
        return;
    };

    let dir = tempdir().expect("tempdir should exist");
    let output = dir.path().join("live-torrent.bin");
    let result = run_download(&output, &source);

    assert!(
        result.status.success(),
        "torrent download failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let metadata = std::fs::metadata(&output).expect("downloaded output should exist");
    assert!(metadata.len() > 0, "downloaded output should be non-empty");
}

#[test]
#[ignore = "requires RS_TORRENT_LIVE_MAGNET and network access"]
fn live_magnet_download_smoke() {
    let Some(source) = maybe_env("RS_TORRENT_LIVE_MAGNET") else {
        eprintln!("skipping: RS_TORRENT_LIVE_MAGNET not set");
        return;
    };

    let dir = tempdir().expect("tempdir should exist");
    let output = dir.path().join("live-magnet.bin");
    let result = run_download(&output, &source);

    assert!(
        result.status.success(),
        "magnet download failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let metadata = std::fs::metadata(&output).expect("downloaded output should exist");
    assert!(metadata.len() > 0, "downloaded output should be non-empty");
}

#[test]
#[ignore = "requires RS_TORRENT_LIVE_RESUME_SOURCE and network access"]
fn live_resume_smoke() {
    let Some(source) = maybe_env("RS_TORRENT_LIVE_RESUME_SOURCE") else {
        eprintln!("skipping: RS_TORRENT_LIVE_RESUME_SOURCE not set");
        return;
    };

    let dir = tempdir().expect("tempdir should exist");
    let output = dir.path().join("live-resume.bin");

    let mut child = spawn_download(&output, &source);
    thread::sleep(Duration::from_secs(3));
    let _ = child.kill();
    let _ = child.wait();

    let result = run_download(&output, &source);
    assert!(
        result.status.success(),
        "resume download failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let metadata = std::fs::metadata(&output).expect("resumed output should exist");
    assert!(metadata.len() > 0, "resumed output should be non-empty");
}

#[test]
#[ignore = "requires RS_TORRENT_LIVE_MULTI_FILE_TORRENT and network access"]
fn live_multi_file_torrent_smoke() {
    let Some(source) = maybe_env("RS_TORRENT_LIVE_MULTI_FILE_TORRENT") else {
        eprintln!("skipping: RS_TORRENT_LIVE_MULTI_FILE_TORRENT not set");
        return;
    };

    let dir = tempdir().expect("tempdir should exist");
    let output_root = dir.path().join("multi-file-root");
    let result = run_download(&output_root, &source);

    assert!(
        result.status.success(),
        "multi-file torrent download failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let metadata = std::fs::metadata(&output_root).expect("output root should exist");
    assert!(
        metadata.is_dir(),
        "multi-file output target should be a directory"
    );
    let entries = std::fs::read_dir(&output_root)
        .expect("output root should be readable")
        .count();
    assert!(entries > 0, "multi-file output root should not be empty");
    if let Some(root_name) = maybe_env("RS_TORRENT_LIVE_MULTI_FILE_ROOT") {
        assert!(
            output_root.join(root_name).exists(),
            "expected multi-file root directory was not created"
        );
    }
}

#[test]
#[ignore = "requires RS_TORRENT_LIVE_UDP_TORRENT and network access"]
fn live_udp_tracker_torrent_smoke() {
    let Some(source) = maybe_env("RS_TORRENT_LIVE_UDP_TORRENT") else {
        eprintln!("skipping: RS_TORRENT_LIVE_UDP_TORRENT not set");
        return;
    };

    let dir = tempdir().expect("tempdir should exist");
    let output = dir.path().join("live-udp.bin");
    let result = run_download(&output, &source);

    assert!(
        result.status.success(),
        "udp tracker torrent download failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let metadata = std::fs::metadata(&output).expect("downloaded output should exist");
    assert!(metadata.len() > 0, "downloaded output should be non-empty");
}
