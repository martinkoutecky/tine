#!/usr/bin/env bash
# GH #783 driver, run on the booted emulator. Usage: drive.sh <apk> <outdir> <label>
# The page-side probe (probe.js) creates the demo graph, opens a tall page and
# announces tap targets as "GH783 ... TARGET trial=N cx= cy= dpr= mcTop="; this
# script turns each into a real `adb shell input tap`, so the soft keyboard opens
# the way it does for a user, and records screenshots and IME state around it.
set -uo pipefail
apk="$1"; out="$2"; label="$3"
mkdir -p "$out"
log="$out/logcat-a.txt"
adb logcat -c || true
adb logcat -v threadtime > "$log" 2>&1 &
lc=$!
trap 'kill $lc 2>/dev/null; adb logcat -d -v threadtime > "$out/logcat-final.txt" 2>&1 || true; adb shell dumpsys webviewupdate > "$out/webview.txt" 2>&1 || true' EXIT

# Optional second IME (IME=florisboard): a different input method, installed and selected.
if [ "${IME:-default}" = "florisboard" ]; then
  curl -sSL -o "$out/florisboard.apk" https://github.com/florisboard/florisboard/releases/download/v0.5.2/florisboard-0.5.2-stable.apk || true
  adb install -r "$out/florisboard.apk" > "$out/ime-install.txt" 2>&1 || true
  adb shell ime enable dev.patrickgold.florisboard/.FlorisImeService >> "$out/ime-install.txt" 2>&1 || true
  adb shell ime set dev.patrickgold.florisboard/.FlorisImeService >> "$out/ime-install.txt" 2>&1 || true
fi
# Make the soft keyboard show on the emulator (it has a virtual hardware keyboard).
adb shell settings put secure show_ime_with_hard_keyboard 1
adb shell ime list -s > "$out/ime-list.txt" 2>&1 || true
adb shell settings get secure default_input_method >> "$out/ime-list.txt" 2>&1 || true
adb shell wm size > "$out/wm.txt" 2>&1; adb shell wm density >> "$out/wm.txt" 2>&1
adb shell getprop ro.build.version.sdk >> "$out/wm.txt"; adb shell getprop ro.product.model >> "$out/wm.txt"

aapt2=$(ls "$ANDROID_SDK_ROOT"/build-tools/*/aapt2 | tail -1)
pkg=$("$aapt2" dump badging "$apk" | sed -n "s/^package: name='\([^']*\)'.*/\1/p")
echo "package=$pkg" | tee "$out/package.txt"
adb install -r "$apk" > "$out/install.txt" 2>&1 || { cat "$out/install.txt"; exit 1; }

waitfor() { # pattern timeout-seconds
  local end=$((SECONDS + $2))
  while [ $SECONDS -lt $end ]; do
    grep -aEq "$1" "$log" && return 0
    sleep 1
  done
  return 1
}
shot() { adb exec-out screencap -p > "$out/$1.png" 2>/dev/null || true; }
ime_shown() { adb shell dumpsys input_method 2>/dev/null | grep -aoE 'mInputShown=(true|false)' | head -1; }

adb shell settings put global hide_error_dialogs 1 || true
nav="${NAV:-3button}"
if [ "$nav" = "gestural" ]; then adb shell cmd overlay enable com.android.internal.systemui.navbar.gestural > "$out/nav.txt" 2>&1 || true; sleep 3; fi
adb shell settings get secure navigation_mode >> "$out/nav.txt" 2>&1 || true

launch() { adb shell monkey -p "$pkg" -c android.intent.category.LAUNCHER 1 >> "$out/launch.txt" 2>&1; }

# Phase A: create the demo graph through the real dialog, then stop the app.
launch
waitfor 'GH783 .* BOOT ' 120 || { echo "probe never booted" >&2; shot no-boot; exit 1; }
waitfor 'GH783 .* CLICKED create-graph' 120 || { echo "no create click" >&2; shot no-create; exit 1; }
waitfor 'GH783 .* CLICKED confirm-create' 30 || echo "no confirm dialog (older UI?)"
sleep 15; shot after-create
adb shell am force-stop "$pkg"; sleep 2

# Write a long journal for today into the new graph (app-private dir, debug APK).
day=$(adb shell date +%Y_%m_%d | tr -d '\r')
adb shell run-as "$pkg" find . -type d -name journals > "$out/journals-dirs.txt" 2>&1 || true
jdir=$(grep -a '^\./' "$out/journals-dirs.txt" | head -1 | tr -d '\r')
echo "journals dir: $jdir day: $day" | tee -a "$out/taps.txt"
if [ "${SEED:-plain}" = "media" ]; then
python3 -I - > "$out/journal.md" <<'PY'
def grp(a, b):
    print("\t\t- " + a)
    print("\t\t- ![clip](../assets/clip.mp4)")
    print("\t\t- " + b)
    print("\t\t- ![photo](../assets/photo.png)")
for h in ("Hall", "Living room", "Dining room"):
    print("- # " + h)
for sec in ("Kitchen", "Bathroom", "Bedroom", "Balcony", "Study"):
    print("- # " + sec)
    for sub in ("Cabinets", "Counter"):
        print("\t- ## " + sub)
        grp("Right of the stove, make a spice drawer (%s)" % sec, "Between the dishwasher and the counter, a thin drawer for utensils and small tools.")
        grp("Pull-out shelf for the pans, with a soft close", "A tall cabinet for the brooms and the mop, with hooks on the inside of the door.")
PY

else
python3 -I - > "$out/journal.md" <<'PY'
words = "the quick brown fox jumps over a lazy dog while notes accumulate on every page of this scratch graph".split()
long_en = " ".join(words[(k * 7) % len(words)] for k in range(260))
cjk = "这是一段很长的中文笔记用来测试键盘弹出以后光标的位置是否仍然在屏幕可见的范围之内" * 6
for i in range(1, 91):
    n = 6 + (i * 7) % 18
    print("- Block %02d: %s" % (i, " ".join(words[(i + k) % len(words)] for k in range(n))))
    if i == 6:
        print("- Long A: " + long_en)
    if i == 30:
        print("- Long B: " + cjk)
PY

fi
adb shell "run-as $pkg sh -c 'cat > $jdir/$day.md'" < "$out/journal.md"
if [ "${SEED:-plain}" = "media" ]; then
  adbdir=$(dirname "$jdir")/assets
  python3 -I - "$out/photo.png" <<'PY'
import struct, sys, zlib
w, h = 540, 960
raw = bytearray()
for y in range(h):
    raw.append(0)
    for x in range(w):
        raw += bytes(((x * 255) // w, (y * 255) // h, ((x // 40 + y // 40) % 2) * 160 + 40))
def chunk(t, d):
    c = struct.pack(">I", len(d)) + t + d
    return c + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b"")
open(sys.argv[1], "wb").write(png)
PY
  ffmpeg -v error -y -f lavfi -i "testsrc2=size=360x640:rate=15:duration=2" -c:v libx264 -preset ultrafast -crf 32 -pix_fmt yuv420p -movflags +faststart "$out/clip.mp4"
  ls -l "$out/photo.png" "$out/clip.mp4" > "$out/fixtures.txt" 2>&1
  adb shell run-as "$pkg" mkdir -p "$adbdir"
  for f in photo.png clip.mp4; do adb shell "run-as $pkg sh -c 'cat > $adbdir/$f'" < "$out/$f"; done
  adb shell run-as "$pkg" ls -l "$adbdir" >> "$out/fixtures.txt" 2>&1
fi
adb shell run-as "$pkg" ls -l "$jdir" > "$out/journals-ls.txt" 2>&1 || true

# Phase B: relaunch; the graph (with the long journal) loads.
kill $lc 2>/dev/null; adb logcat -c || true
log="$out/logcat.txt"
adb logcat -v threadtime > "$log" 2>&1 &
lc=$!
launch
waitfor 'GH783 [0-9.]+ (TALL-OK|TALL-FAIL)' 240 || { echo "no tall page" >&2; shot no-tall; exit 1; }
sleep 3; shot tall-page

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

for n in 1 2 3 4 5 6 7; do
  if ! waitfor "GH783 [0-9.]+ (TARGET|TARGET-NONE) trial=$n( |$)" 90; then echo "trial $n: no target" >&2; continue; fi
  line=$(grep -aE "GH783 [0-9.]+ TARGET trial=$n " "$log" | tail -1)
  [ -z "$line" ] && { echo "trial $n: no usable target"; continue; }
  cx=$(echo "$line" | sed -n 's/.* cx=\([0-9.]*\).*/\1/p'); cy=$(echo "$line" | sed -n 's/.* cy=\([0-9.]*\).*/\1/p')
  dpr=$(echo "$line" | sed -n 's/.* dpr=\([0-9.]*\).*/\1/p')
  read -r wl wt wr wb <<< "$(webview_bounds "t$n")"
  x=$(python3 -I -c "print(int($wl + $cx * $dpr))"); y=$(python3 -I -c "print(int($wt + $cy * $dpr))")
  echo "trial $n: css=($cx,$cy) dpr=$dpr webview=[$wl,$wt][$wr,$wb] -> tap ($x,$y)" | tee -a "$out/taps.txt"
  echo "ime-before-$n $(ime_shown)" >> "$out/taps.txt"
  shot "t$n-0-before"
  adb shell input tap "$x" "$y"
  sleep 0.5; shot "t$n-1-p0.5s"
  sleep 1.0; shot "t$n-2-p1.5s"
  sleep 2.0; shot "t$n-3-p3.5s"
  echo "ime-after-$n $(ime_shown)" >> "$out/taps.txt"
  waitfor "GH783 [0-9.]+ TRIAL-(DONE|NOEDIT) trial=$n" 60 || true
  sleep 1
  if [ "$(ime_shown)" = "mInputShown=true" ]; then adb shell input keyevent 4; sleep 1; fi
  sleep 1
done
waitfor 'GH783 .* ALLDONE' 60 || true
grep -a "GH783" "$log" | sed 's/^.*GH783/GH783/' > "$out/gh783.txt"
wc -l "$out/gh783.txt"
exit 0
