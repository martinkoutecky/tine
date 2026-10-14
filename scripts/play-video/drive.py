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
SHOTS = os.environ.get("SHOTS", "0") == "1"


def adb(*args, check=True, timeout=120):
    r = subprocess.run(["adb", *args], capture_output=True, text=True, timeout=timeout)
    if check and r.returncode != 0:
        raise RuntimeError(f"adb {args} failed: {r.stderr}")
    return r.stdout


def sh(cmd, **kw):
    return adb("shell", cmd, **kw)


_dumpn = [0]


def shot(tag):
    os.makedirs(f"{OUT}/shots", exist_ok=True)
    png = subprocess.run(["adb", "exec-out", "screencap", "-p"], capture_output=True).stdout
    with open(f"{OUT}/shots/{_dumpn[0]:03d}-{tag}.png", "wb") as f:
        f.write(png)


def dump(tag=""):
    for _ in range(4):
        sh("uiautomator dump /sdcard/ui.xml", check=False)
        xml = adb("exec-out", "cat", "/sdcard/ui.xml", check=False)
        if xml.strip().startswith("<?xml"):
            break
        time.sleep(0.7)
    _dumpn[0] += 1
    if SHOTS:
        shot(tag)
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


log = []


def tap_bounds(b):
    log.append({"t": round(time.time() - t_rec, 2) if t_rec else 0, "tap": list(b)})
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


SETTLE = 1.8  # the screen the dump saw reaches the encoded video a little later on this slow emulator


def step(needle, cap, pre=1.2, post=1.2, tag="", settle=SETTLE, **kw):
    """Wait for an element, let the screen settle, tap it. The kept clip runs from
    `pre` s before the tap to `post` s after."""
    node, b = wait_for(needle, tag=tag, **kw)
    time.sleep(settle)
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
    step("Open an existing graph", "1  Open a graph folder", pre=1.4, post=0.6, tag="welcome")

    # 2. system All files access page
    step("Allow access to manage all files", "2  Allow \u201cAll files access\u201d", pre=1.3, post=1.5,
         tag="allfiles-settings", settle=2.2)  # tapping the row toggles the switch
    sh("input keyevent KEYCODE_BACK")
    time.sleep(1.0)

    # 3. back in Tine, open again -> system folder picker
    CAP3 = "3  Pick the notes folder"
    step("Open an existing graph", CAP3, pre=1.0, post=0.6, tag="welcome2")

    # picker navigation: Documents -> Tine demo -> Use this folder -> Allow
    wait_for("Documents", tag="picker-start", timeout=60)
    for _ in range(6):
        root = dump("picker-nav")
        if find(root, "Tine demo", exact=True):
            break
        h = find(root, "Documents", exact=True)
        if h:
            time.sleep(1.2)
            t = now(); tap_bounds(h[0][1]); time.sleep(1.2); keep(t - 1.2, now(), CAP3)
            continue
        h = find(root, "Show roots")
        if h:
            tap_bounds(h[0][1]); time.sleep(1.0)
    step("Tine demo", CAP3, pre=1.0, post=1.0, tag="picker-found", exact=True)
    step("Use this folder", CAP3, pre=1.0, post=0.8, tag="use-folder")
    step("Allow", CAP3, pre=1.4, post=0.5, tag="allow-dialog", settle=1.4)

    # 4. graph opens
    wait_for("Tine demo", timeout=150, tag="graph-open")
    time.sleep(1.5)
    root = dump("after-open")
    h = find(root, "Close navigation sidebar")
    if h:
        tap_bounds(h[0][1]); time.sleep(1.0)
    for _ in range(3):  # dismiss emulator-only toasts (software rendering) and the Guide teaser
        root = dump("toasts")
        ds = find(root, "Dismiss", exact=True)
        if not ds:
            break
        tap_bounds(ds[0][1]); time.sleep(0.7)
    wait_for("Morning review", timeout=60, tag="journal")
    time.sleep(1.8)
    t = now()
    time.sleep(2.2)
    keep(t - 0.3, now(), "4  Tine opens it in place")
    step("[[Seed Workshop]]", "4  Tine opens it in place", pre=0.2, post=3.0, tag="to-page", exact=True, settle=0.3)
    status = "ok"
except Exception as e:  # keep the evidence either way
    status = f"FAILED: {e!r}"
    print(status, file=sys.stderr)
    dump("failure")
    SHOTS = True
    shot("failure")

# End on a visible change (Home) so the recording's last frame time ~ this instant,
# which lets post.py align the recording clock to ours from the END of the file.
sh("input keyevent KEYCODE_HOME", check=False)
t_home = now()
time.sleep(1.5)
sh("pkill -2 screenrecord", check=False)
try:
    rec.wait(timeout=30)
except subprocess.TimeoutExpired:
    rec.kill()
time.sleep(1.5)
adb("pull", "/sdcard/raw.mp4", f"{OUT}/raw.mp4")
json.dump({"status": status, "t_home": t_home, "segments": segments, "log": log}, open(f"{OUT}/events.json", "w"), indent=1)
sh("logcat -d -s Tine/FolderPicker:I chromium:I | tail -80 > /sdcard/tine.log", check=False)
with open(f"{OUT}/logcat-tine.txt", "w", errors="replace") as f:
    f.write(subprocess.run(["adb", "logcat", "-d", "-b", "all", "-t", "6000"], capture_output=True, text=True, errors="replace").stdout)
sys.exit(0 if status == "ok" else 1)
