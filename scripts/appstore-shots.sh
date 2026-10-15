#!/usr/bin/env bash
# PROOF BRANCH ONLY. Takes App Store screenshots of Tine in iOS Simulators.
# usage: appstore-shots.sh <Tine.app> <outdir>
set -euo pipefail
APP="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
OUT="$2"
mkdir -p "$OUT"
BUNDLE=page.tine.Tine

udid_of() { xcrun simctl list devices available -j | jq -r --arg n "$1" \
  '[.devices | to_entries[] | select(.key | test("iOS")) | .value[] | select(.isAvailable and .name == $n)] | last | .udid'; }

shoot() { # udid slug name
  xcrun simctl io "$1" screenshot --type=png "$OUT/$2/$3.png"
}

run_theme() { # udid slug theme scenes...
  local udid="$1" slug="$2" theme="$3"
  xcrun simctl uninstall "$udid" "$BUNDLE" 2>/dev/null || true
  xcrun simctl install "$udid" "$APP"
  local data graph
  data="$(xcrun simctl get_app_container "$udid" "$BUNDLE" data)"
  graph="$data/Documents/Tine Demo"
  mkdir -p "$data/Documents"
  python3 scripts/appstore-demo-graph.py store-demo-graph "$graph"
  xcrun simctl ui "$udid" appearance "$theme"
  SIMCTL_CHILD_RUST_LOG=info \
  SIMCTL_CHILD_TINE_DEBUG=1 \
  SIMCTL_CHILD_TINE_DEBUG_LOG="$data/tmp/tine-debug-$theme.log" \
  SIMCTL_CHILD_TINE_GRAPH="$graph" \
  SIMCTL_CHILD_TINE_SHOT_THEME="$theme" \
    xcrun simctl launch "$udid" "$BUNDLE"
  sleep 15
  shoot "$udid" "$slug" "$theme-0-launch"
  # tine:// routes (src/deepLinks.ts parseAppRoute) opened like a Home Screen quick action.
  local n=1 scene url
  for scene in "journal|tine://today" "seed-workshop|tine://page/Seed%20Workshop" \
               "field-guide|tine://page/Field%20Guide" "search|tine://search?q=soil"; do
    url="${scene#*|}"; scene="${scene%%|*}"
    xcrun simctl openurl "$udid" "$url"
    sleep 5
    shoot "$udid" "$slug" "$theme-$n-$scene"
    n=$((n+1))
  done
  mkdir -p "$OUT/logs"
  cp "$data/tmp/tine-debug-$theme.log" "$OUT/logs/$slug-$theme-debug.log" 2>/dev/null || true
  xcrun simctl spawn "$udid" log show --last 3m --style compact \
    --predicate 'process == "Tine"' > "$OUT/logs/$slug-$theme-sim.log" || true
  xcrun simctl terminate "$udid" "$BUNDLE" || true
}

one_device() { # device-name slug
  local udid; udid="$(udid_of "$1")"
  test -n "$udid" && test "$udid" != null
  echo "== $1 -> $udid"
  mkdir -p "$OUT/$2"
  xcrun simctl boot "$udid"
  xcrun simctl bootstatus "$udid" -b
  xcrun simctl status_bar "$udid" override --time 9:41 --dataNetwork wifi --wifiMode active \
    --wifiBars 3 --cellularMode active --cellularBars 4 --operatorName "" \
    --batteryState charged --batteryLevel 100
  run_theme "$udid" "$2" light
  run_theme "$udid" "$2" dark
  xcrun simctl shutdown "$udid"
}

one_device "iPhone 17 Pro Max" iphone
one_device "iPad Pro 13-inch (M5)" ipad
for f in "$OUT"/*/*.png; do echo "$f $(sips -g pixelWidth -g pixelHeight "$f" | awk '/pixel/{printf "%s ",$2}')"; done
