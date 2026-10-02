import json, statistics, sys
rows = [json.loads(l) for l in open("results.jsonl", encoding="utf-8-sig") if l.strip()]
def med(phase, mode, getter):
    v = []
    for r in rows:
        if r.get("phase") == phase and r.get("mode") == mode:
            try:
                x = getter(r["result"])
                if x is not None: v.append(x)
            except (KeyError, TypeError, IndexError):
                pass
    return (statistics.median(v), v) if v else (None, [])
M = [
 ("cold readyMs", "cold", lambda d: d["readyMs"]),
 ("cold firstPageMs", "cold", lambda d: d["firstPageMs"]),
 ("cold diag.publishMs", "cold", lambda d: d["diagPublishMs"]),
 ("cold load read ms", "cold", lambda d: d["diagLoadPass"]["read"]["ms"]),
 ("cold load stat ms", "cold", lambda d: d["diagLoadPass"]["stat"]["ms"]),
 ("cold load parse ms", "cold", lambda d: d["diagLoadPass"]["parse"]["ms"]),
 ("cold load listing ms", "cold", lambda d: d["diagLoadPass"]["listing"]["ms"]),
 ("cold load wall ms", "cold", lambda d: d["diagLoadPass"]["wallMs"]),
 ("cold get_page largest ms", "cold", lambda d: d["pageLargestMs"]),
 ("cold get_page median ms", "cold", lambda d: d["pageMedianMs"]),
 ("cold Ctrl-K ms", "cold", lambda d: d["ctrlKMs"]),
 ("cold scan_refresh #1 ms", "cold", lambda d: d["scanRefreshMs"][0]),
 ("cold scan_refresh #3 ms", "cold", lambda d: d["scanRefreshMs"][2]),
 ("cold full-diff statMs (diag)", "cold", lambda d: [x for x in d["diagFullDiffs"]["recent"] if x["trigger"] != "launch_diff"][-1]["statMs"]),
 ("cold checkpoint write ms", "cold", lambda d: d["checkpointWrite"]["ms"]),
 ("cold2 (rescanned files) readyMs", "cold2", lambda d: d["readyMs"]),
 ("cold2 load read ms", "cold2", lambda d: d["diagLoadPass"]["read"]["ms"]),
 ("cold2 load wall ms", "cold2", lambda d: d["diagLoadPass"]["wallMs"]),
 ("cold2 diag.publishMs", "cold2", lambda d: d["diagPublishMs"]),
 ("warm firstPageMs", "warm", lambda d: d["firstPageMs"]),
 ("warm readyMs", "warm", lambda d: d["readyMs"]),
 ("warm get_page largest ms", "warm", lambda d: d["pageLargestMs"]),
 ("warm Ctrl-K ms", "warm", lambda d: d["ctrlKMs"]),
 ("warm scan_refresh #1 ms", "warm", lambda d: d["scanRefreshMs"][0]),
 ("prims list+stat ms", "prims", lambda d: d["prims"]["listAndStatMs"]),
 ("prims stat-all ms (12.9k pages)", "prims", lambda d: d["prims"]["statMs"]),
 ("prims open-all ms", "prims", lambda d: d["prims"]["openMs"]),
 ("prims read-all FIRST touch ms", "prims", lambda d: d["prims"]["readFirstMs"]),
 ("prims read-all second touch ms", "prims", lambda d: d["prims"]["readSecondMs"]),
]
phases = ["OFF1", "ON1", "OFF2", "ON2"]
out = ["| metric (median of 3, ms) | OFF1 | ON1 | OFF2 | ON2 | ON / OFF | raw ON1 | raw ON2 |", "|---|---:|---:|---:|---:|---:|---|---|"]
for name, mode, g in M:
    meds = [med(p, mode, g) for p in phases]
    f = lambda m: "n/a" if m[0] is None else f"{m[0]:.1f}"
    offs = [m[0] for m in (meds[0], meds[2]) if m[0] is not None]
    ons = [m[0] for m in (meds[1], meds[3]) if m[0] is not None]
    ratio = "n/a"
    if ons and offs and statistics.mean(offs) > 0:
        ratio = f"{statistics.mean(ons) / statistics.mean(offs):.2f}x"
    raw = lambda m: ', '.join(f'{x:.0f}' for x in m[1])
    out.append(f"| {name} | {f(meds[0])} | {f(meds[1])} | {f(meds[2])} | {f(meds[3])} | {ratio} | {raw(meds[1])} | {raw(meds[3])} |")
print("\n".join(out))
print()
print("| MsMpEng CPU ms during run (median) | OFF1 | ON1 | OFF2 | ON2 |\n|---|---:|---:|---:|---:|")
for mode in ("cold","cold2","warm","prims"):
    cells=[]
    for p in phases:
        v=[r["msMpEngCpuMs"] for r in rows if r.get("phase")==p and r.get("mode")==mode and "msMpEngCpuMs" in r]
        cells.append(f"{statistics.median(v):.0f}" if v else "n/a")
    print(f"| {mode} | " + " | ".join(cells) + " |")
print()
for r in rows:
    if r.get("mode") == "proof":
        p = r["proof"]
        print(r["phase"], {k: p[k] for k in ("AMRunningMode","RealTimeProtectionEnabled","OnAccessProtectionEnabled","IoavProtectionEnabled","BehaviorMonitorEnabled","ExclusionPath","ForceDefenderPassiveMode","EicarBlocked") if k in p})
