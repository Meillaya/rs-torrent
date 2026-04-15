use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;
use tempfile::tempdir;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bittorrent-starter-rust"))
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
