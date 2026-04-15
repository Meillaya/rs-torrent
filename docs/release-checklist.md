# Release Checklist

## Deterministic gate

Run:

```bash
./scripts/release-readiness.sh
```

This must pass before tagging a release.

## Optional live gate

If the live acceptance corpus env vars are configured, run:

```bash
./scripts/release-readiness.sh --with-live
```

## Manual sanity

Recommended quick manual checks:

```bash
cargo run -- --help
cargo run -- info sample.torrent
cargo run -- decode '5:hello'
```

## What to confirm before tagging

- changelog is updated
- README reflects current CLI behavior
- remaining risks are still accurate
- release notes match the actual scope of the release
- Cargo.toml and Cargo.lock versions are aligned

## Tagging

Example:

```bash
git tag -a vX.Y.Z -m 'rs-torrent vX.Y.Z'
git push origin vX.Y.Z
```

The release workflow will verify the repository and publish the GitHub release from `CHANGELOG.md`.
