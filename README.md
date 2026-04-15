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
./target/release/rs-torrent
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

For a multi-file torrent, pass a destination root directory instead of a final file path:

```bash
cargo run -- download -o ./downloads path/to/multi-file.torrent
```

That will materialize the torrent under:

```text
./downloads/<torrent-root-name>/
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

Trackers may now be HTTP(S) or UDP, and multiple `tr=` parameters are preserved from magnet links.

## Resume behavior

Full-file downloads write to a temporary part file and a JSON resume sidecar.

```text
<output>.part
<output>.resume.json
```

On restart, the downloader reloads the saved state, re-validates completed pieces, discards corrupted partial data, and resumes only the missing pieces.

For multi-file torrents, pieces are still stored contiguously during download and then finalized into a safe file tree under the chosen destination root.

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
export RS_TORRENT_LIVE_MULTI_FILE_TORRENT='<multi-file torrent source>'
export RS_TORRENT_LIVE_UDP_TORRENT='<torrent source backed by UDP trackers>'
```

Run the ignored live suite:

```bash
cargo test --test live_acceptance -- --ignored
```

## Current limitations

Piece availability is sampled from peer bitfields at startup and updated from some peer state changes, but it is not yet a fully adaptive production scheduler.

UDP tracker support is intentionally conservative. The client can speak to UDP trackers and validates core packet semantics, but it does not yet try to aggressively optimize connection reuse.

Live-swarm validation is opt-in rather than part of the default local test path, so real-world swarm confidence still depends on running the ignored acceptance tests before release or personal use.

Magnet downloads still depend on tracker-backed discovery. Because DHT / PEX / LSD remain out of scope, magnets without useful tracker coverage may still fail even when the protocol implementation is otherwise correct.

The multi-file finalize path is now staged and path-safe, but it still uses one contiguous `.part` payload internally. That keeps the implementation simple and correct, but it is not yet tuned for very large torrents or highly optimized disk behavior.

## Next major milestones

The next major milestone is a more adaptive scheduler. That would mean better peer scoring, better use of live availability updates, and fewer wasted retries against slow or unreliable peers.

The second milestone is stronger release confidence. The project now has ignored live acceptance tests, but the next step is to turn those into a repeatable release gate with a stable real-world torrent corpus.

The third milestone is richer tracker and magnet resilience. That includes better fallback diagnostics, more operational visibility, and tighter handling for mixed tracker environments.

The fourth milestone is storage and throughput optimization for larger torrents. The current contiguous `.part` strategy is correct and simple, but future work could reduce disk churn and improve large-download efficiency.

The fifth milestone, if the project scope grows, is selective download and richer user control. That would stay below daemon/UI scope, but would make the CLI much more practical for everyday torrent use.
