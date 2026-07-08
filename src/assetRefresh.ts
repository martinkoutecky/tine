// Refresh a graph asset's rendered <img> after it was edited in an external app
// (GH #38). Tine doesn't watch assets/ (that dir is intentionally outside the
// journals/+pages/ file-watcher), and the user necessarily alt-tabbed away to
// edit — so when Tine regains window focus we invalidate + version-bump every
// asset we launched an editor for, forcing its <img> to re-read from disk. No
// polling, no watcher churn. The focus listener installs lazily on first use.

import { refreshAsset } from "./assetCache";

const pending = new Set<string>();
let installed = false;

function flush(): void {
  if (!pending.size) return;
  for (const rel of pending) refreshAsset(rel);
  pending.clear();
}

function install(): void {
  if (installed || typeof window === "undefined") return;
  installed = true;
  window.addEventListener("focus", flush);
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) flush();
  });
}

/** Mark an asset for refresh when Tine next regains focus. Call right after
 *  launching an external editor for it. */
export function refreshAssetOnReturn(rel: string): void {
  if (!rel) return;
  pending.add(rel);
  install();
}
