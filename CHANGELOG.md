# Changelog

## Unreleased

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
