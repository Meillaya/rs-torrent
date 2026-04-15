# BitTorrent Client

A Rust BitTorrent CLI downloader with:
- `.torrent` support
- magnet support
- tracker + peer protocol handling
- persisted partial downloads via `.part` + `.resume.json`
- resume validation on restart
- deterministic unit/integration coverage plus opt-in live acceptance tests

## Current focus

This project is currently optimized as a **personal CLI downloader** with a reusable internal core.

Intentionally out of scope for now:
- daemon/server mode
- GUI/web UI
- DHT / PEX / LSD
- NAT traversal / UPnP
- protocol encryption
- stable public library API

## Build and verify

```bash
cargo fmt --all --check
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## CLI usage

### Show help

```bash
cargo run -- --help
```

### Decode a bencoded value

```bash
cargo run -- decode '5:hello'
```

### Inspect a torrent file

```bash
cargo run -- info sample.torrent
```

### List peers for a torrent

```bash
cargo run -- peers sample.torrent
```

### Download a full file from a torrent

```bash
cargo run -- download -o output.bin sample.torrent
```

### Download a single piece

```bash
cargo run -- download_piece -o piece.bin sample.torrent 0
```

### Parse a magnet link

```bash
cargo run -- magnet_parse 'magnet:?xt=urn:btih:<info-hash>&tr=http://tracker.example/announce'
```

### Download from a magnet link

```bash
cargo run -- magnet_download -o output.bin 'magnet:?xt=urn:btih:<info-hash>&tr=http://tracker.example/announce'
```

## Progress output

The downloader now emits structured human-readable progress lines such as:

```text
[progress] resume state loaded: 2/10 pieces already complete
[progress] stored piece 3 (4/10 complete)
[progress] finalized download to output.bin
[warn] failed to probe peer bitfield from 127.0.0.1:6881: timeout
```

These are intended to make foreground CLI use easier to understand while the downloader is running.

## Resume behavior

For full-file downloads the downloader writes:
- `<output>.part`
- `<output>.resume.json`

On restart it:
1. reloads the saved state
2. re-validates completed pieces against piece hashes
3. marks corrupted resumed pieces incomplete
4. resumes only the missing pieces
5. renames the `.part` file to the final output once complete

## Test suite

### Deterministic tests

The default `cargo test` suite includes:
- unit tests for parsing / protocol helpers / storage / scheduling
- integration tests for CLI entrypoints

### Opt-in live acceptance tests

Live tests are intentionally **ignored by default** because they require real swarms and network access.

Available live smoke tests:
- torrent download
- magnet download
- interrupted download + resume

Environment variables:

```bash
export RS_TORRENT_LIVE_TORRENT='<torrent-file-or-url-you-use-for-live-smoke>'
export RS_TORRENT_LIVE_MAGNET='magnet:?xt=urn:btih:...'
export RS_TORRENT_LIVE_RESUME_SOURCE='<same kind of source used for resume smoke>'
```

Run them with:

```bash
cargo test --test live_acceptance -- --ignored
```

## Current limitations

- piece availability is sampled from startup bitfields, but live `have` traffic is only partially exploited
- no daemonized/background mode yet
- no selective file download yet
- live-swarm validation is opt-in, not part of the default test path

