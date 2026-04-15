# Summary

This PR upgrades the project from a challenge-style BitTorrent prototype into a more usable CLI downloader foundation.

It centralizes the CLI surface, hardens tracker and metadata protocol handling, adds persisted resume storage, improves scheduler behavior with peer availability signals, and introduces a much broader deterministic and opt-in live test suite.

# What changed

## Product / CLI
- replaced ad hoc argument parsing with a `clap`-based command layer
- kept the existing underscore-style commands (`download_piece`, `magnet_parse`, etc.)
- added cleaner progress and warning output for foreground CLI use

## Protocol / core
- fixed raw-byte tracker query encoding for `peer_id` and `info_hash`
- improved BEP 9 / BEP 10 metadata exchange behavior
- preserved and parsed peer bitfields
- tracked `have` messages in peer state

## Downloading / resume
- added `.part` + `.resume.json` storage
- validated resumed pieces before trusting saved state
- wrote full-file downloads incrementally instead of buffering the whole file in memory
- used observed peer availability to sort missing pieces where possible

## Tests
- expanded unit coverage across parsing, protocol helpers, storage, progress, and scheduling
- added integration tests for the binary CLI entrypoints
- added ignored live acceptance tests for torrent, magnet, and resume smoke checks

# Verification

Passed locally:

```bash
cargo fmt --all --check
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo run -- --help
cargo run -- info sample.torrent
cargo run -- decode '5:hello'
```

# Remaining limitations

- live acceptance tests are env-gated and ignored by default
- scheduler still only partially exploits live `have` traffic beyond the current peer-state tracking improvements
- no daemon/UI/selective-download scope yet by design
