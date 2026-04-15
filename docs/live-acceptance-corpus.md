# Live Acceptance Corpus

This document defines the intended shape of the env-gated live acceptance inputs used by `tests/live_acceptance.rs` and `scripts/release-readiness.sh --with-live`.

## Purpose

The live suite is meant to prove that `rs-torrent` still works against representative real-world inputs before a release is trusted for personal use.

The goal is not broad swarm coverage. The goal is a small, stable, repeatable corpus that exercises the main risk areas of the current product scope.

## Required scenarios

### 1. Single-file torrent (`RS_TORRENT_LIVE_TORRENT`)
Use a known-good tracker-backed torrent that:
- completes reliably
- does not require DHT/PEX/LSD
- has stable HTTP or mixed HTTP/UDP tracker coverage
- produces one final file

### 2. Magnet download (`RS_TORRENT_LIVE_MAGNET`)
Use a known-good tracker-backed magnet that:
- includes explicit `tr=` parameters
- can resolve metadata through peers reached from its trackers
- does not rely on DHT-only discovery

### 3. Resume smoke (`RS_TORRENT_LIVE_RESUME_SOURCE`)
Use a source that:
- is large enough that an interrupt/resume cycle is meaningful
- is stable enough to survive a stop/restart test
- preferably uses the same source repeatedly to reduce variance

### 4. Multi-file torrent (`RS_TORRENT_LIVE_MULTI_FILE_TORRENT`)
Use a torrent that:
- materializes a nested file tree
- has a predictable root directory name
- does not contain path edge cases beyond normal nested directories

Optional supporting variable:
- `RS_TORRENT_LIVE_MULTI_FILE_ROOT`
  - expected top-level root directory name under the chosen destination root

### 5. UDP-tracker-backed torrent (`RS_TORRENT_LIVE_UDP_TORRENT`)
Use a torrent where:
- UDP tracker discovery is truly exercised
- peer discovery still works without depending purely on HTTP trackers
- the input is stable enough for repeated release checks

## Selection guidelines

Prefer a corpus that is:
- legally safe for your own use
- stable over time
- small enough to keep release checks practical
- tracker-backed rather than DHT-dependent
- varied enough to cover:
  - single-file finalize
  - multi-file finalize
  - metadata discovery
  - UDP tracker transport
  - interruption/resume

## What this corpus does not try to cover

It does not guarantee:
- every public swarm shape
- every tracker implementation
- DHT/PEX/LSD behavior
- daemon/seeding/ratio workflows
- pathological bandwidth or huge-content stress tests

Those should be treated as later test expansions, not blockers for the current CLI-first scope.

## Release usage

Before a release, set the env vars and run:

```bash
./scripts/release-readiness.sh --with-live
```

If the corpus changes, update this file and `README.md` together.
