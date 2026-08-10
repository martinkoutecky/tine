#!/usr/bin/env bash
set -euo pipefail

output="${1:-certification-receipt.txt}"
commit="$(git rev-parse HEAD)"
ref="${GITHUB_REF_NAME:-candidate}"
run_url="${GITHUB_SERVER_URL:-https://github.com}/${GITHUB_REPOSITORY:-martinkoutecky/tine-storage}/actions/runs/${GITHUB_RUN_ID:-local}"
lock_sha="$(sha256sum Cargo.lock | cut -d' ' -f1)"
api_sha="$(sha256sum api.txt | cut -d' ' -f1)"
manifest="$(cargo run --locked --quiet --example format_manifest)"

{
  echo "tine-storage certification receipt"
  echo "ref=$ref"
  echo "commit=$commit"
  echo "run=$run_url"
  echo "rustc=$(rustc --version)"
  echo "cargo=$(cargo --version)"
  echo "cargo_lock_sha256=$lock_sha"
  echo "public_api_sha256=$api_sha"
  echo "required_jobs=linux-complete,windows-complete,android-compile,api-semver"
  echo "format_manifest_begin"
  printf '%s\n' "$manifest"
  echo "format_manifest_end"
} > "$output"
