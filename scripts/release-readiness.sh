#!/usr/bin/env bash
set -euo pipefail

run_live=false
explain_live=false

for arg in "$@"; do
  case "$arg" in
    --with-live)
      run_live=true
      ;;
    --explain-live)
      explain_live=true
      ;;
    *)
      echo "Unknown argument: $arg" >&2
      echo "Usage: $0 [--with-live] [--explain-live]" >&2
      exit 1
      ;;
  esac
done

required_vars=(
  RS_TORRENT_LIVE_TORRENT
  RS_TORRENT_LIVE_MAGNET
  RS_TORRENT_LIVE_RESUME_SOURCE
  RS_TORRENT_LIVE_MULTI_FILE_TORRENT
  RS_TORRENT_LIVE_UDP_TORRENT
)

missing=()
configured=()
for name in "${required_vars[@]}"; do
  if [ -z "${!name:-}" ]; then
    missing+=("$name")
  else
    configured+=("$name")
  fi
done

if [ "$explain_live" = true ]; then
  if [ "${#missing[@]}" -eq 0 ]; then
    echo "[release-readiness] all live test env vars are configured"
  elif [ "${#configured[@]}" -eq 0 ]; then
    echo "[release-readiness] no live test env vars are configured"
  else
    echo "[release-readiness] partial live test configuration detected"
    echo "[release-readiness] configured live test env vars:"
    printf '  + %s\n' "${configured[@]}"
    echo "[release-readiness] missing live test env vars:"
    printf '  - %s\n' "${missing[@]}"
  fi
fi

echo "[release-readiness] running deterministic verification"
cargo fmt --all --check
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings

if [ "$run_live" = true ]; then
  if [ "${#missing[@]}" -gt 0 ]; then
    echo "[release-readiness] missing live test env vars:" >&2
    printf '  - %s\n' "${missing[@]}" >&2
    exit 1
  fi

  echo "[release-readiness] running ignored live acceptance tests"
  cargo test --test live_acceptance -- --ignored
else
  echo "[release-readiness] skipped live acceptance tests (pass --with-live to require them)"
fi
