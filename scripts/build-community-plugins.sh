#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
set +u
source "$ROOT/scripts/env.sh"
set -u

for plugin in bullet-threading query-filter; do
  echo "==> building $plugin"
  (cd "$ROOT/community-plugins/$plugin" && cargo build --release --offline)
  node "$ROOT/scripts/tine-plugin.mjs" check "$ROOT/community-plugins/$plugin"
done
