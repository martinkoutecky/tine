#!/usr/bin/env python3
"""Drive Tine on an emulator through the All-files-access flow while screenrecord runs.

Writes out/raw.mp4 (full recording), out/events.json (keep-segments with labels,
in recording time) and out/ui/*.xml (uiautomator dumps, for debugging).
Usage: drive.py <outdir>
"""
import json, os, re, subprocess, sys, time
import xml.etree.ElementTree as ET

OUT = sys.argv[1]
os.makedirs(f"{OUT}/ui", exist_ok=True)
PKG = "page.tine.app"


def adb(*args, check=True, timeout=120):
    r = subprocess.run(["adb", *args], capture_output=True, text=True, timeout=timeout)
    if check and r.returncode != 0:
        raise RuntimeError(f"adb {args} failed: {r.stderr}")
    return r.stdout


def sh(cmd, **kw):
    return adb("shell", cmd, **kw)


_dumpn = [0]


def dump(tag=""):
    for _ in range(4):
        sh("uiautomator dump /sdcard/ui.xml", check=False)
        xml = adb("exec-out", "cat", "/sdcard/ui.xml", check=False)
        if xml.strip().startswith("<?xml"):
            break
        time.sleep(0.7)
    _dumpn[0] += 1
    with open(f"{OUT}/ui/{_dumpn[0]:03d}-{tag}.xml", "w") as f:
        f.write(xml)
    try:
        return ET.fromstring(xml)
    except ET.ParseError:
        return None


def nodes(root):
    if root is None:
        return []
    out = []
    for n in root.iter("node"):
        m = re.match(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]", n.get("bounds", ""))
        if not m:
            continue
        x1, y1, x2, y2 = map(int, m.groups())
        if x2 <= x1 or y2 <= y1:
            continue
        out.append((n, (x1, y1, x2, y2)))
    return out


def label(n):
    return " | ".join(filter(None, [n.get("text"), n.get("content-desc")]))


def find(root, needle, exact=False, cls=None):
    needle = needle.lower()
    hits = []
    for n, b in nodes(root):
        if cls and n.get("class") != cls:
            continue
        for s in (n.get("text") or "", n.get("content-desc") or ""):
            s = s.strip().lower()
            if (s == needle) if exact else (needle in s and s):
                hits.append((n, b))
                break
    # prefer the smallest (innermost) match
    hits.sort(key=lambda h: (h[1][2] - h[1][0]) * (h[1][3] - h[1][1]))
    return hits


def tap_bounds(b):
    sh(f"input tap {(b[0] + b[2]) // 2} {(b[1] + b[3]) // 2}")


def wait_for(needle, timeout=90, tag="", **kw):
    t0 = time.time()
    while time.time() - t0 < timeout:
        root = dump(tag or needle[:20].replace(" ", "_"))
        h = find(root, needle, **kw)
        if h:
            return h[0]
        time.sleep(1.0)
    raise TimeoutError(f"timed out waiting for {needle!r}")


# --- recording clock -----------------------------------------------------
rec = None
t_rec = None
segments = []  # (start, end, caption) in recording time


def now():
    return time.time() - t_rec


def keep(start, end, cap):
    segments.append({"start": max(0.0, start), "end": end, "caption": cap})


def step(needle, cap, pre=1.0, post=1.2, tag="", **kw):
    """Wait for an element, then tap it. The kept clip runs from slightly before the
    element was first seen... no: from `pre` s before the tap to `post` s after."""
    node, b = wait_for(needle, tag=tag, **kw)
    t_tap = now()
    tap_bounds(b)
    time.sleep(post)
    keep(t_tap - pre, now(), cap)
    return t_tap


# --- setup ---------------------------------------------------------------
sh("settings put global window_animation_scale 0.5; "
   "settings put global transition_animation_scale 0.5; "
   "settings put global animator_duration_scale 0.5", check=False)
# clean status bar (demo mode)
sh("settings put global sysui_demo_allowed 1", check=False)
for c in ["-e command enter",
          "-e command clock -e hhmm 0941",
          "-e command battery -e level 100 -e plugged false",
          "-e command network -e wifi show -e level 4 -e mobile hide",
          "-e command notifications -e visible false"]:
    sh(f"am broadcast -a com.android.systemui.demo {c}", check=False)
sh("input keyevent KEYCODE_HOME")
time.sleep(1)

rec = subprocess.Popen(["adb", "shell", "screenrecord", "--size", "720x1280",
                        "--bit-rate", "8000000", "--time-limit", "170", "/sdcard/raw.mp4"])
t_rec = time.time() + 0.6  # screenrecord needs ~0.6 s to start producing frames
time.sleep(1.5)

try:
    sh(f"monkey -p {PKG} -c android.intent.category.LAUNCHER 1")
    # 1. first run: tap "Open an existing graph"
    step("Open an existing graph", "1  Tine opens: tap Open an existing graph", pre=2.2, post=1.0, tag="welcome")

    # 2. system All files access page
    node, b = wait_for("Allow access to manage all files", tag="allfiles-settings")
    time.sleep(0.8)
    t0 = now()
    sw = [h for h in nodes(dump("settings-switch")) if h[0].get("class", "").endswith("Switch")]
    if sw:
        tap_bounds(sw[0][1])
    else:
        tap_bounds(b)
    time.sleep(1.3)
    keep(t0 - 1.6, now(), "2  Android asks for All files access: switch it on for Tine")
    sh("input keyevent KEYCODE_BACK")
    time.sleep(1.0)

    # 3. back in Tine, open again -> system folder picker
    step("Open an existing graph", "3  Back in Tine: open again, pick the notes folder", pre=0.8, post=1.5, tag="welcome2")

    # picker navigation
    wait_for("Documents", tag="picker-start", timeout=60)
    for _ in range(6):
        root = dump("picker-nav")
        h = find(root, "Tine demo", exact=True)
        if h:
            break
        h = find(root, "Documents", exact=True)
        if h:
            t = now(); tap_bounds(h[0][1]); time.sleep(1.4); keep(t - 0.8, now(), "3  Back in Tine: open again, pick the notes folder")
            continue
        h = find(root, "Show roots")
        if h:
            tap_bounds(h[0][1]); time.sleep(1.0)
    t = now()
    root = dump("picker-found")
    h = find(root, "Tine demo", exact=True)
    tap_bounds(h[0][1]); time.sleep(1.5)
    keep(t - 0.6, now(), "3  Back in Tine: open again, pick the notes folder")
    step("Use this folder", "3  Back in Tine: open again, pick the notes folder", pre=1.0, post=1.3, tag="use-folder")
    step("Allow", "3  Back in Tine: open again, pick the notes folder", pre=0.8, post=0.8, tag="allow-dialog", exact=False)

    # 4. graph opens
    wait_for("Morning review", timeout=150, tag="graph-open")
    t = now()
    time.sleep(2.2)
    keep(t - 0.4, now(), "4  Tine opens your notes in place")
    step("Field Guide", "4  Tine opens your notes in place", pre=0.3, post=2.6, tag="to-page")
    status = "ok"
except Exception as e:  # keep the evidence either way
    status = f"FAILED: {e!r}"
    print(status, file=sys.stderr)
    dump("failure")
    adb("exec-out", "screencap", "-p", check=False)
    with open(f"{OUT}/failure.png", "wb") as f:
        f.write(subprocess.run(["adb", "exec-out", "screencap", "-p"], capture_output=True).stdout)

time.sleep(0.5)
sh("pkill -2 screenrecord", check=False)
try:
    rec.wait(timeout=30)
except subprocess.TimeoutExpired:
    rec.kill()
time.sleep(1.5)
adb("pull", "/sdcard/raw.mp4", f"{OUT}/raw.mp4")
json.dump({"status": status, "segments": segments}, open(f"{OUT}/events.json", "w"), indent=1)
sh("logcat -d -s Tine/FolderPicker:I chromium:I | tail -80 > /sdcard/tine.log", check=False)
with open(f"{OUT}/logcat-tine.txt", "w") as f:
    f.write(adb("logcat", "-d", "-t", "4000", check=False))
sys.exit(0 if status == "ok" else 1)
