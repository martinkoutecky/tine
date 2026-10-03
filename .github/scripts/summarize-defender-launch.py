"""Summarize results.jsonl of run-defender-launch.ps1 (GH #623): median and raw per metric, ms.
Reports numbers only; it enforces no budget."""
import json, statistics

rows = [json.loads(l) for l in open("results.jsonl", encoding="utf-8-sig") if l.strip()]


def vals(mode, getter):
    out = []
    for r in rows:
        if r.get("mode") == mode:
            try:
                x = getter(r["result"])
                if x is not None:
                    out.append(x)
            except (KeyError, TypeError, IndexError, ValueError):
                pass
    return out


def asset_held_max(d):
    walks = d["diagAssetWalks"]["recent"]
    return max(w["heldMs"] for w in walks) if walks else None


METRICS = [
    ("cold readyMs", "cold", lambda d: d["readyMs"]),
    ("cold firstPageMs", "cold", lambda d: d["firstPageMs"]),
    ("cold load read ms", "cold", lambda d: d["diagLoadPass"]["read"]["ms"]),
    ("cold load wall ms", "cold", lambda d: d["diagLoadPass"]["wallMs"]),
    ("cold get_page largest ms", "cold", lambda d: d["pageLargestMs"]),
    ("cold Ctrl-K ms", "cold", lambda d: d["ctrlKMs"]),
    ("cold scan_refresh #1 ms", "cold", lambda d: d["scanRefreshMs"][0]),
    ("cold page click during rescan, worst ms", "cold", lambda d: max(d["clickDuringRescanMs"])),
    ("cold asset walk writer hold, worst ms", "cold", asset_held_max),
    ("cold checkpoint write ms", "cold", lambda d: d["checkpointWrite"]["ms"]),
    ("cold2 readyMs", "cold2", lambda d: d["readyMs"]),
    ("warm readyMs", "warm", lambda d: d["readyMs"]),
    ("warm firstPageMs", "warm", lambda d: d["firstPageMs"]),
    ("warm scan_refresh #1 ms", "warm", lambda d: d["scanRefreshMs"][0]),
    ("warm page click during rescan, worst ms", "warm", lambda d: max(d["clickDuringRescanMs"])),
    ("warm asset walk writer hold, worst ms", "warm", asset_held_max),
    ("prims open-all ms", "prims", lambda d: d["prims"]["openMs"]),
    ("prims read-all first touch ms", "prims", lambda d: d["prims"]["readFirstMs"]),
    ("prims read-all second touch ms", "prims", lambda d: d["prims"]["readSecondMs"]),
]
print("Defender ON, Ellis-shaped graph (median of reps, ms). No budget is enforced.\n")
print("| metric | median | raw |\n|---|---:|---|")
for name, mode, getter in METRICS:
    v = vals(mode, getter)
    med = f"{statistics.median(v):.1f}" if v else "n/a"
    print(f"| {name} | {med} | {', '.join(f'{x:.0f}' for x in v)} |")
for r in rows:
    if r.get("mode") == "proof":
        p = r["proof"]
        keys = ("AMRunningMode", "RealTimeProtectionEnabled", "OnAccessProtectionEnabled", "EicarBlocked")
        print("\nDefender proof:", {k: p[k] for k in keys if k in p})
