# Changelog

## Unreleased

## v0.2.0

### Added
- Multi-file torrent layout preservation through the normalized `TorrentInfo` execution model
- Safe staged multi-file finalization under a destination root
- UDP tracker support with BEP 15-style connect/announce packet handling
- Smarter peer-health scheduling with cooldowns, remembered missing pieces, and bounded piece requeue
- Cooperative interruption handling with durable piece-level resume checkpoints
- Expanded deterministic coverage for tracker packets, multi-file output, scheduler behavior, and interruption paths

### Changed
- `.torrent` files now preserve tracker fallback candidates from `announce-list` metadata
- Tracker querying is transport-agnostic from the caller’s point of view (HTTP and UDP)
- Full downloads now report clearer finalized-path and interruption state

### Fixed
- Multi-file finalize now stages output before moving it into place
- Piece checkpoint durability is stronger through `sync_data()` and atomic resume-state writes
- Shutdown now stops dequeuing new work at the queue boundary instead of aborting workers

## v0.1.2

### Fixed
- Aligned the release tag line with the current master branch after the 0.1.1 binary rename follow-up
- Synchronized Cargo metadata and lockfile state for the renamed `rs-torrent` binary release

## v0.1.1

### Changed
- Renamed the release binary from `bittorrent-starter-rust` to `rs-torrent`
- Updated integration tests and documentation to use the new binary name

### Added
- Shared CLI command layer built on `clap`
- Shared peer ID generation and source-resolution helpers
- Structured download progress reporting
- Resume storage using `.part` files and `.resume.json` sidecars
- Deterministic integration tests for CLI entrypoints
- Opt-in live acceptance tests for torrent, magnet, and resume smoke checks

### Changed
- Tracker requests now use raw-byte-safe percent encoding for `peer_id` and `info_hash`
- Metadata exchange now supports multi-piece BEP 9 / BEP 10 metadata transfer
- Full-file downloads now write verified pieces incrementally instead of buffering the whole file in memory
- Downloader startup now probes peer bitfields and orders missing pieces by observed availability when possible
- Peer state handling now preserves and updates piece availability from bitfield and `have` messages

### Fixed
- Restored buildability by adding the missing `rand` dependency
- Removed duplicated peer ID generation and duplicated torrent-vs-magnet source resolution logic
- Resume state now re-validates completed pieces and invalidates corrupted partial data on restart
- CLI help now matches the actual command surface
