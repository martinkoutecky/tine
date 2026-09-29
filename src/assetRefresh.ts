// Refresh a graph asset's rendered <img> after it changed on disk outside Tine.
//
// Two triggers feed the same cache invalidation (`assetCache.refreshAsset`):
//  * the native watcher observes the assets directory (in-graph, or the approved
//    external target of an `assets` link) and emits `asset-changed` batches of
//    assets-relative paths (master d017d1afc, 2f54a8d5e) — an image replaced by
//    an editor, Syncthing, Dropbox or another Tine window refreshes in place;
//  * GH #38: for an asset Tine launched an external editor for, focus return
//    still forces a re-read, as a fallback for a platform event that never came.
// The focus listener installs lazily on first use.

import { backend } from "./backend";
import { captureBinding } from "./binding";
import { invalidateAsset, refreshAsset } from "./assetCache";
import { ownedWhen, readOwnedResource } from "./owned";

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

const DEFERRED_MEDIA_EXTENSIONS = new Set([
  "pdf",
  "mp3", "mpeg", "m4a", "aac", "wav", "ogg", "oga", "opus", "flac",
  "mp4", "m4v", "webm", "ogv", "mov", "mkv",
]);

function deferHotSwap(rel: string): boolean {
  const ext = rel.split(/[?#]/, 1)[0].split(".").pop()?.toLowerCase();
  return !!ext && DEFERRED_MEDIA_EXTENSIONS.has(ext);
}

/** Apply one native watcher publication. Images (and image-like unknown embeds)
 *  refresh in place. PDF/audio/video bytes are only invalidated for their next
 *  open: an already-open document or playback session is deliberately left alone. */
export function applyObservedAssetChanges(paths: string[]): void {
  for (const rel of new Set(paths.filter(Boolean))) {
    if (deferHotSwap(rel)) invalidateAsset(rel);
    else refreshAsset(rel);
  }
}

/** Subscribe this window to the watcher's `asset-changed` events for the graph
 *  it is bound to; a stale binding's batch is dropped. Returns the unsubscribe. */
export function subscribeAssetChanges(): () => void {
  let alive = true;
  let unsub = () => {};
  void readOwnedResource(
    ownedWhen(() => alive),
    backend().onAssetChanged((batch) => {
      if (batch.binding_generation !== undefined && batch.binding_generation !== captureBinding().backendGeneration) return;
      applyObservedAssetChanges(batch.paths);
    }),
    (stop) => stop(),
  ).then((result) => { if (result.kind === "current") unsub = result.value; });
  return () => { alive = false; unsub(); };
}
