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

# 1) Toolchain (cargo/rustc live outside the repo — see scripts/env.sh). env.sh is
#    written for interactive shells and appends to $LD_LIBRARY_PATH, so relax
#    nounset just around it.
set +u
# shellcheck source=/dev/null
source "$ROOT/scripts/env.sh"
set -u

# 2) Frontend FIRST — this is the step a raw cargo build skips. Also stamps the
#    wall-clock build time into the bundle (vite.config.ts reproBuildTime).
echo "==> npm run build (frontend + dist)"
npm run build

# 3) Release binary. custom-protocol is mandatory (else it loads from the dev
#    server and shows "Could not connect to localhost").
echo "==> cargo build --release --features custom-protocol"
cargo build --release --features custom-protocol --manifest-path src-tauri/Cargo.toml

# 4) Prove the binary embeds the frontend we JUST built (not a stale dist). The
#    plaintext build stamp is gzipped inside the binary, so we can't grep it — but
#    the content-hashed asset FILENAMES stay plaintext and are unique per build.
ASSET="$(grep -oE '[A-Za-z0-9_]+-[A-Za-z0-9_-]+\.(js|css)' "$ROOT/dist/index.html" | head -1)"
if [ -z "$ASSET" ]; then
  echo "!! could not find a hashed asset in dist/index.html — aborting" >&2
  exit 1
fi
if ! grep -aq -- "$ASSET" "$BIN"; then
  echo "!! binary does NOT embed current dist ($ASSET) — stale build, aborting" >&2
  exit 1
fi
echo "==> embed OK ($ASSET present in binary)"

# 5) Deliver (Syncthing carries $DEST to Martin's machines).
cp -f "$BIN" "$DEST"
SIZE="$(stat -c %s "$DEST" 2>/dev/null || wc -c < "$DEST")"
echo "==> deployed → $DEST  ($SIZE bytes)"
echo "    Settings › Built should now read $(date '+%H:%M') (local)."
