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
phases = ["OFF1", "ON", "OFF2"]
out = ["| metric (median of 3, ms) | OFF1 | ON | OFF2 | ON / mean(OFF) | raw ON |", "|---|---:|---:|---:|---:|---|"]
for name, mode, g in M:
    meds = [med(p, mode, g) for p in phases]
    f = lambda m: "n/a" if m[0] is None else f"{m[0]:.1f}"
    offs = [m[0] for m in (meds[0], meds[2]) if m[0] is not None]
    ratio = "n/a"
    if meds[1][0] is not None and offs and statistics.mean(offs) > 0:
        ratio = f"{meds[1][0] / statistics.mean(offs):.2f}x"
    out.append(f"| {name} | {f(meds[0])} | {f(meds[1])} | {f(meds[2])} | {ratio} | {', '.join(f'{x:.0f}' for x in meds[1][1])} |")
print("\n".join(out))
print()
for r in rows:
    if r.get("mode") == "proof":
        p = r["proof"]
        print(r["phase"], {k: p[k] for k in ("AMRunningMode","RealTimeProtectionEnabled","OnAccessProtectionEnabled","IoavProtectionEnabled","BehaviorMonitorEnabled","ExclusionPath","ForceDefenderPassiveMode","EicarBlocked") if k in p})
