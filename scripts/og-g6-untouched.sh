#!/usr/bin/env bash
# G6 data-safety gate (og parity campaign): on a COPY of a graph,
#  1. open + read every page + close through tine-store writes 0 bytes (tree hash unchanged, no new files);
#  2. parse -> serialize of every Markdown file loses no structure (tine-core roundtrip_dir).
# Usage: scripts/og-g6-untouched.sh <graph dir>   (default: ~/research/logseq-anonymized)
set -euo pipefail
SRC="${1:-$HOME/research/logseq-anonymized}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT
cp -a "$SRC" "$WORK/graph"
tree_hash() { (cd "$1" && find . -type f -print0 | sort -z | xargs -0 sha256sum); }
tree_hash "$WORK/graph" > "$WORK/before"
cargo run -q --release -p tine-store --example open_untouched -- "$WORK/graph"
tree_hash "$WORK/graph" > "$WORK/after"
if ! diff -u "$WORK/before" "$WORK/after"; then echo "G6 FAIL: opening the graph changed files" >&2; exit 1; fi
echo "G6 open-untouched: 0 byte diffs"
OUT="$(cargo run -q --release -p tine-core --example roundtrip_dir -- "$WORK/graph" | tail -1)"
echo "G6 roundtrip: $OUT"
case "$OUT" in *" 0 structural bugs"*) ;; *) echo "G6 FAIL: structural round-trip bugs" >&2; exit 1;; esac
