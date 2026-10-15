#!/usr/bin/env python3
"""Copy store-demo-graph to DEST, shifting the journal dates so the newest
journal is "today" on the runner (the app's journal view is anchored on today)."""
import datetime as dt, pathlib, re, shutil, sys

src, dst = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
today = dt.date.today()
# Original graph: 2026-10-14 is "today", 2026-10-13 yesterday.
base = dt.date(2026, 10, 14)
def fmt(d): return d.strftime("%Y-%m-%d %a")
def shifted(m):
    d = dt.date(*map(int, m.group(1).split("-")))
    return m.group(0).replace(m.group(1) + " " + m.group(2), fmt(today + (d - base)))
shutil.copytree(src, dst, dirs_exist_ok=True)
for f in sorted((dst / "journals").glob("*.md"), reverse=True) + sorted((dst / "pages").glob("*.md")):
    text = re.sub(r"<(\d{4}-\d{2}-\d{2}) (\w{3})>", shifted, f.read_text())
    f.write_text(text)
    m = re.fullmatch(r"(\d{4})_(\d{2})_(\d{2})\.md", f.name)
    if m:
        d = today + (dt.date(*map(int, m.groups())) - base)
        f.rename(f.with_name(d.strftime("%Y_%m_%d") + ".md"))
