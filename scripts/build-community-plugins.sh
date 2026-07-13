#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
set +u
source "$ROOT/scripts/env.sh"
set -u

build_args=(--release --locked)
if [[ "${TINE_PLUGIN_OFFLINE:-0}" == "1" ]]; then
  build_args+=(--offline)
fi

for plugin in bullet-threading query-filter heading-level-shortcuts; do
  echo "==> building $plugin"
  (cd "$ROOT/community-plugins/$plugin" && cargo build "${build_args[@]}")
  node "$ROOT/scripts/tine-plugin.mjs" check "$ROOT/community-plugins/$plugin"
done
