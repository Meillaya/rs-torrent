# rs-torrent

`rs-torrent` is a Rust BitTorrent CLI downloader.

It currently focuses on dependable foreground downloads from `.torrent` files and magnet links. The codebase includes resume support, piece verification, tracker and peer protocol handling, and a growing automated test suite.

## Scope

This project is aimed at a personal CLI downloader with a reusable internal core.

It is not currently targeting daemon mode, GUI/web interfaces, DHT/PEX/LSD, NAT traversal, protocol encryption, or a stable public library API.

## Install

Build a release binary:

```bash
cargo build --release
```

The binary will be available at:

```bash
./target/release/bittorrent-starter-rust
```

Install it into your Cargo bin directory:

```bash
cargo install --path .
```

## Build

Check and build the project locally:

```bash
cargo check
cargo build
cargo build --release
```

## Run

Show the available commands:

```bash
cargo run -- --help
```

Decode a bencoded value:

```bash
cargo run -- decode '5:hello'
```

Inspect a torrent file:

```bash
cargo run -- info sample.torrent
```

List tracker peers for a torrent:

```bash
cargo run -- peers sample.torrent
```

Download a full file from a torrent:

```bash
cargo run -- download -o output.bin sample.torrent
```

Download a single piece:

```bash
cargo run -- download_piece -o piece.bin sample.torrent 0
```

Parse a magnet link:

```bash
cargo run -- magnet_parse 'magnet:?xt=urn:btih:<info-hash>&tr=http://tracker.example/announce'
```

Download from a magnet link:

```bash
cargo run -- magnet_download -o output.bin 'magnet:?xt=urn:btih:<info-hash>&tr=http://tracker.example/announce'
```

## Resume behavior

Full-file downloads write to a temporary part file and a JSON resume sidecar.

```text
<output>.part
<output>.resume.json
```

On restart, the downloader reloads the saved state, re-validates completed pieces, discards corrupted partial data, and resumes only the missing pieces.

## Progress output

The CLI emits short progress and warning lines while downloading.

```text
[progress] resume state loaded: 2/10 pieces already complete
[progress] stored piece 3 (4/10 complete)
[progress] finalized download to output.bin
[warn] failed to probe peer bitfield from 127.0.0.1:6881: timeout
```

## Test

Run the deterministic local suite:

```bash
cargo test
```

Run the full verification set used during development:

```bash
cargo fmt --all --check
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## Live acceptance tests

Live swarm tests are available, but they are ignored by default.

Set one or more environment variables:

```bash
export RS_TORRENT_LIVE_TORRENT='<torrent-file-or-url>'
export RS_TORRENT_LIVE_MAGNET='magnet:?xt=urn:btih:...'
export RS_TORRENT_LIVE_RESUME_SOURCE='<torrent-or-magnet-source>'
```

Run the ignored live suite:

```bash
cargo test --test live_acceptance -- --ignored
```

## Current limitations

Piece availability is sampled from peer bitfields at startup and updated from some peer state changes, but it is not yet a fully adaptive production scheduler.

Live-swarm validation is opt-in rather than part of the default local test path.
