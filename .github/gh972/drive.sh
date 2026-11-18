#!/usr/bin/env bash
# GH #972 driver, run on the booted emulator. Usage: drive.sh <apk> <outdir> <label>
# probe.js announces "TARGET step=N name=.. cx= cy= dpr=" or "KEY step=N .. code=4";
# this script turns each into a real adb input event.
set -uo pipefail
apk="$1"; out="$2"; label="$3"
mkdir -p "$out"
log="$out/logcat.txt"
adb logcat -c || true
adb logcat -v threadtime > "$log" 2>&1 &
lc=$!
trap 'kill $lc 2>/dev/null; adb shell dumpsys webviewupdate > "$out/webview.txt" 2>&1 || true' EXIT
adb shell wm size > "$out/wm.txt" 2>&1; adb shell wm density >> "$out/wm.txt" 2>&1
adb shell getprop ro.build.version.sdk >> "$out/wm.txt"; adb shell getprop ro.product.model >> "$out/wm.txt"
aapt2=$(ls "$ANDROID_SDK_ROOT"/build-tools/*/aapt2 | tail -1)
pkg=$("$aapt2" dump badging "$apk" | sed -n "s/^package: name='\([^']*\)'.*/\1/p")
echo "package=$pkg ref=$label" | tee "$out/package.txt"
adb install -r "$apk" > "$out/install.txt" 2>&1 || { cat "$out/install.txt"; exit 1; }
adb shell settings put global hide_error_dialogs 1 || true
nav="${NAV:-3button}"
if [ "$nav" = "gestural" ]; then adb shell cmd overlay enable com.android.internal.systemui.navbar.gestural > "$out/nav.txt" 2>&1 || true; sleep 3; fi
adb shell settings get secure navigation_mode >> "$out/nav.txt" 2>&1 || true

waitfor() { local end=$((SECONDS + $2)); while [ $SECONDS -lt $end ]; do grep -aEq "$1" "$log" && return 0; sleep 1; done; return 1; }
shot() { adb exec-out screencap -p > "$out/$1.png" 2>/dev/null || true; }

launch() { adb shell monkey -p "$pkg" -c android.intent.category.LAUNCHER 1 >> "$out/launch.txt" 2>&1; }
# Phase A: create the demo graph through the real dialog, then stop the app.
launch
waitfor 'GH972 .* BOOT ' 120 || { echo "probe never booted" >&2; shot no-boot; exit 1; }
waitfor 'GH972 .* GRAPH-CREATED' 180 || { echo "graph not created" >&2; shot no-create; exit 1; }
sleep 5; shot after-create
adb shell am force-stop "$pkg"; sleep 2
day=$(adb shell date +%Y_%m_%d | tr -d '\r')
adb shell run-as "$pkg" find . -type d -name journals > "$out/journals-dirs.txt" 2>&1 || true
jdir=$(grep -a '^\./' "$out/journals-dirs.txt" | head -1 | tr -d '\r')
echo "journals dir: $jdir day: $day" | tee -a "$out/taps.txt"
python3 -I - > "$out/journal.md" <<'PY2'
words = "the quick brown fox jumps over a lazy dog while notes accumulate on every page of this scratch graph".split()
for i in range(1, 91):
    n = 6 + (i * 7) % 18
    print("- Block %02d: %s" % (i, " ".join(words[(i + k) % len(words)] for k in range(n))))
PY2
adb shell "run-as $pkg sh -c 'cat > $jdir/$day.md'" < "$out/journal.md"
adb shell run-as "$pkg" ls -l "$jdir" > "$out/journals-ls.txt" 2>&1 || true
# Phase B: relaunch; the graph (with the long journal) loads.
kill $lc 2>/dev/null; adb logcat -c || true
log="$out/logcat-b.txt"
adb logcat -v threadtime > "$log" 2>&1 &
lc=$!
launch
waitfor 'GH972 .* BOOT ' 120 || { echo "probe never booted (B)" >&2; shot no-boot-b; exit 1; }
waitfor 'GH972 .* READY' 240 || { echo "never ready" >&2; shot no-ready; exit 1; }
sleep 2; shot ready

webview_bounds() {
  for i in 1 2 3; do
    adb shell uiautomator dump /sdcard/ui.xml > /dev/null 2>&1
    adb pull /sdcard/ui.xml "$out/ui-$1.xml" > /dev/null 2>&1 || true
    b=$(python3 -I - "$out/ui-$1.xml" <<'PY'
import re, sys
s = open(sys.argv[1], errors="replace").read()
m = re.search(r'class="android.webkit.WebView"[^>]*bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', s)
print(m.group(1), m.group(2), m.group(3), m.group(4)) if m else None
PY
    )
    [ -n "$b" ] && [ "$b" != None ] && { echo "$b"; return 0; }
    sleep 2
  done
  echo "0 128 1080 2274"
}
read -r wl wt wr wb <<< "$(webview_bounds start)"
echo "webview=[$wl,$wt][$wr,$wb]" | tee -a "$out/taps.txt"

for n in $(seq 1 14); do
  waitfor "GH972 [0-9.]+ (TARGET|KEY|TARGET-NONE) step=$n( |$)" 90 || { echo "step $n: nothing announced" >&2; continue; }
  line=$(grep -aE "GH972 [0-9.]+ (TARGET|KEY) step=$n " "$log" | tail -1)
  [ -z "$line" ] && { echo "step $n: no target"; continue; }
  shot "s$n-0-before"
  if echo "$line" | grep -aq " KEY "; then
    echo "step $n: BACK key" | tee -a "$out/taps.txt"
    adb shell input keyevent 4
  else
    cx=$(echo "$line" | sed -n 's/.* cx=\([0-9.]*\).*/\1/p'); cy=$(echo "$line" | sed -n 's/.* cy=\([0-9.]*\).*/\1/p')
    dpr=$(echo "$line" | sed -n 's/.* dpr=\([0-9.]*\).*/\1/p')
    x=$(python3 -I -c "print(int($wl + $cx * $dpr))"); y=$(python3 -I -c "print(int($wt + $cy * $dpr))")
    echo "step $n: css=($cx,$cy) dpr=$dpr -> tap ($x,$y)" | tee -a "$out/taps.txt"
    adb shell input tap "$x" "$y"
  fi
  sleep 2.2; shot "s$n-1-after"
  waitfor "GH972 [0-9.]+ (STEP-DONE|STEP-TIMEOUT) step=$n( |$)" 40 || true
done
waitfor 'GH972 .* ALLDONE' 60 || true
shot final
grep -a "GH972" "$log" | sed 's/^.*GH972/GH972/' > "$out/gh972.txt"
wc -l "$out/gh972.txt"
exit 0
