# Changelog

## Unreleased

## v0.4.0

### Added
- Release-readiness automation via `scripts/release-readiness.sh`
- Live acceptance corpus and release checklist docs
- Cooperative interruption handling with durable piece-level resume checkpoints
- Selected-tracker reporting and stronger tracker fallback diagnostics across CLI paths
- Adaptive scheduler updates driven by peer health, live bitfield knowledge, missing-piece knowledge, shared availability counts, and requeue reordering

### Changed
- GitHub release workflow now verifies the repository before publishing tags
- `download_piece` now surfaces tracker selection/fallback warnings consistently with full downloads
- Peer ranking now uses deterministic tie-breaking and richer weighting from known-good pieces and recent error severity
- Verified piece writes now use a trusted fast path while preserving the safe public wrapper

### Fixed
- Tracker fallback warnings now survive successful and empty-peer fallback paths across CLI commands
- The scheduler no longer double-counts startup bitfield availability
- Shutdown now stops dequeuing new work at the queue boundary instead of aborting workers
- True ranking ties are deterministic and tested

## v0.3.1

### Fixed
- Aligned the release line with the current branch after the release-workflow secret-gating fix
- Kept the GitHub release workflow manually dispatchable for existing tags after the 0.3.0 release

## v0.3.0

### Added
- Cooperative interruption handling with durable piece-level resume checkpoints
- Adaptive scheduler updates driven by peer health, live bitfield knowledge, missing-piece knowledge, and requeue reordering
- Selected-tracker reporting and stronger fallback diagnostics across CLI paths
- Release-readiness automation via `scripts/release-readiness.sh`
- Release confidence documentation via `docs/live-acceptance-corpus.md` and `docs/release-checklist.md`

### Changed
- Peer ranking now uses deterministic tie-breaking plus richer signal weighting
- Shared piece-availability state is updated during a run instead of being treated as startup-only knowledge
- Verified piece writes now use a trusted fast path while preserving the safe public wrapper
- GitHub release workflow now verifies the repository before publishing tags

### Fixed
- Tracker fallback warnings now survive successful and empty-peer fallback cases
- The scheduler no longer double-counts startup bitfield availability
- True ranking ties are now deterministic

## v0.2.1

### Fixed
- Aligned the release tag line with the current master branch after the 0.2.0 lockfile metadata follow-up
- Synchronized Cargo metadata and lockfile state for the 0.2.x release line

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
