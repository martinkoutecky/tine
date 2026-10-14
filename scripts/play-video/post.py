#!/usr/bin/env python3
"""Cut raw.mp4 down to the kept segments, add step captions, write final.mp4 + contact sheet.
Usage: post.py <outdir>"""
import json, subprocess, sys

OUT = sys.argv[1]
ev = json.load(open(f"{OUT}/events.json"))
segs = sorted(ev["segments"], key=lambda s: s["start"])
# merge overlaps (keep first caption)
merged = []
for s in segs:
    if merged and s["start"] <= merged[-1]["end"]:
        merged[-1]["end"] = max(merged[-1]["end"], s["end"])
        merged[-1].setdefault("caps", []).append(s)
    else:
        s = dict(s); s["caps"] = [dict(s)]; merged.append(s)
# output timeline: split caption ranges by original sub-segment starts
parts, out_t, caps = [], 0.0, []
for m in merged:
    parts.append((m["start"], m["end"]))
    for c in m["caps"]:
        pass
    dur = m["end"] - m["start"]
    # caption of a merged block = caption of the sub-segment active at each moment
    subs = sorted(m["caps"], key=lambda c: c["start"])
    for i, c in enumerate(subs):
        a = max(c["start"], m["start"])
        b = subs[i + 1]["start"] if i + 1 < len(subs) else m["end"]
        caps.append((out_t + a - m["start"], out_t + b - m["start"], c["caption"]))
    out_t += dur
total = out_t
print("total kept seconds:", round(total, 1))
fc, labels = [], []
for i, (a, b) in enumerate(parts):
    fc.append(f"[0:v]trim=start={a:.3f}:end={b:.3f},setpts=PTS-STARTPTS,fps=30[v{i}]")
    labels.append(f"[v{i}]")
chain = "".join(labels) + f"concat=n={len(parts)}:v=1:a=0[cat]"
font = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"
dt = []
for (a, b, text) in caps:
    t = text.replace(":", "\\:").replace("'", "")
    dt.append(f"drawtext=fontfile={font}:text='{t}':fontcolor=white:fontsize=30:box=1:boxcolor=black@0.62:"
              f"boxborderw=14:x=(w-text_w)/2:y=h-130:enable='between(t,{a:.3f},{b:.3f})'")
fc.append(chain)
fc.append("[cat]" + ",".join(dt) + ",format=yuv420p[out]")
subprocess.check_call(["ffmpeg", "-y", "-loglevel", "error", "-i", f"{OUT}/raw.mp4",
                       "-filter_complex", ";".join(fc), "-map", "[out]", "-c:v", "libx264",
                       "-preset", "medium", "-crf", "23", "-movflags", "+faststart", "-an",
                       f"{OUT}/all-files-access-demo.mp4"])
json.dump({"total": total, "captions": caps}, open(f"{OUT}/timeline.json", "w"), indent=1)
# contact sheet: one frame per 2 s, 8 columns
subprocess.check_call(["ffmpeg", "-y", "-loglevel", "error", "-i", f"{OUT}/all-files-access-demo.mp4",
                       "-vf", "fps=1/2,scale=200:-1,tile=8x2:padding=4:margin=4:color=white",
                       "-frames:v", "1", f"{OUT}/contact-sheet.png"])
# also per-step full-size frames
for i, (a, b, text) in enumerate(caps):
    subprocess.check_call(["ffmpeg", "-y", "-loglevel", "error", "-ss", f"{(a+b)/2:.2f}",
                           "-i", f"{OUT}/all-files-access-demo.mp4", "-frames:v", "1",
                           f"{OUT}/frame-{i:02d}.png"])
