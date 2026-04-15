use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bittorrent-starter-rust"))
}

#[test]
fn help_lists_expected_commands() {
    let output = bin().arg("--help").output().expect("help should run");
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("download_piece"));
    assert!(stdout.contains("magnet_info"));
    assert!(stdout.contains("magnet_download"));
}

#[test]
fn decode_command_outputs_json() {
    let output = bin()
        .args(["decode", "5:hello"])
        .output()
        .expect("decode should run");
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert_eq!(stdout.trim(), "\"hello\"");
}

#[test]
fn info_command_reports_sample_fixture_metadata() {
    let output = bin()
        .args(["info", "sample.torrent"])
        .output()
        .expect("info should run");
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("Tracker URL: http://bittorrent-test-tracker.codecrafters.io/announce"));
    assert!(stdout.contains("Length: 92063"));
    assert!(stdout.contains("Info Hash: d69f91e6b2ae4c542468d1073a71d4ea13879a7f"));
    assert!(stdout.contains("Number of Pieces: 3"));
}

#[test]
fn magnet_parse_reports_tracker_and_info_hash() {
    let magnet = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&tr=http://tracker.test/announce";
    let output = bin()
        .args(["magnet_parse", magnet])
        .output()
        .expect("magnet_parse should run");
    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf8");
    assert!(stdout.contains("Tracker URL: http://tracker.test/announce"));
    assert!(stdout.contains("Info Hash: 0123456789abcdef0123456789abcdef01234567"));
}

#[test]
fn invalid_subcommand_exits_with_failure() {
    let output = bin()
        .arg("definitely-not-a-command")
        .output()
        .expect("invalid command should run");
    assert!(!output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf8");
    assert!(stderr.contains("unrecognized subcommand"));
}
