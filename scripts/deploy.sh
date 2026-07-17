#!/usr/bin/env bash
# The ONE sanctioned way to build + deploy the Tine release binary.
#
# Why this exists: a raw `cargo build` does NOT rebuild the frontend
# (`beforeBuildCommand: npm run build` only fires under the Tauri CLI), so it
# happily re-links a fresh binary around a STALE `dist/` — the "Built" time in
# Settings then lags the binary and a frontend change ships silently stale. This
# script always runs `npm run build` first, so dist (and its wall-clock build
# stamp) can never fall behind the binary. Run this; don't hand-run cargo.
#
#   ./scripts/deploy.sh
#
# NOTE: this does NOT regenerate the vendored wasm parser. If you bumped the lsdoc
# pin (or are trialing an lsdoc branch), run `npm run build:wasm` FIRST, then this.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.." && pwd)"
cd "$ROOT"

DEST="${TINE_DEPLOY_DEST:-$HOME/research/tine}"
BIN="$ROOT/target/release/tine"
RECEIPT="$BIN.build.json"
SNAPSHOT="$BIN.build.before.json"

# 1) Toolchain (cargo/rustc live outside the repo — see scripts/env.sh). env.sh is
#    written for interactive shells and appends to $LD_LIBRARY_PATH, so relax
#    nounset just around it.
set +u
# shellcheck source=/dev/null
source "$ROOT/scripts/env.sh"
set -u

# 2) Snapshot before either builder can mutate a source input. The receipt helper
#    refuses an input or HEAD change and verifies the exact output binary later.
node scripts/build-e2e-receipt.mjs before --snapshot "$SNAPSHOT"
echo "==> npm run build (frontend + dist)"
npm run build

# 3) Release binary. custom-protocol is mandatory (else it loads from the dev
#    server and shows "Could not connect to localhost").
echo "==> cargo build --release --features custom-protocol"
cargo build --release --features custom-protocol --manifest-path src-tauri/Cargo.toml

# 4) A checkout SHA alone cannot prove a binary. The helper compares the
#    pre-build snapshot, verifies the embedded frontend asset, hashes this exact
#    app, and writes the receipt consumed by run-e2e.
node scripts/build-e2e-receipt.mjs after --snapshot "$SNAPSHOT" --app "$BIN" --receipt "$RECEIPT"

# 5) Deliver (Syncthing carries $DEST to Martin's machines).
cp -f "$BIN" "$DEST"
cp -f "$RECEIPT" "$DEST.build.json"
SIZE="$(stat -c %s "$DEST" 2>/dev/null || wc -c < "$DEST")"
echo "==> deployed → $DEST  ($SIZE bytes)"
echo "    receipt → $DEST.build.json"
echo "    Settings › Built should now read $(date '+%H:%M') (local)."
