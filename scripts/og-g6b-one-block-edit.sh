#!/usr/bin/env bash
# G6b: exercise frontend pageToDto projection and guarded backend save on a private graph copy.
set -euo pipefail
SRC="${1:-$HOME/research/logseq-anonymized}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
cp -a "$SRC" "$WORK/original"
cp -a "$SRC" "$WORK/edit"
cd "$ROOT"
cargo run -q --release -p tine-store --example g6b_edit -- dump "$WORK/edit" "$WORK/dump.jsonl"
G6B_DUMP="$WORK/dump.jsonl" G6B_EDITS="$WORK/edits.jsonl" npx vitest run src/g6bProjection.test.ts
REPORT="${G6B_REPORT:-$ROOT/G6B-06d-results.tsv}"
G6B_REPORT="$REPORT" cargo run -q --release -p tine-store --example g6b_edit -- save "$WORK/edit" "$WORK/edits.jsonl" "$WORK/original"
LC_ALL=C sort -o "$REPORT" "$REPORT"
