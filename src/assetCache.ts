import { createStore } from "solid-js/store";
import { backend } from "./backend";

// Cache of graph-asset blob URLs keyed by path relative to `assets/`. Without it
// every <img> mount (re-render, scroll back into view, or a second reference to
// the same image) re-reads the file over IPC and mints a fresh blob URL, then
// revokes it on unmount — pure churn on image-heavy pages. The cache holds one
// blob URL per asset for the lifetime of the open graph; it MUST be cleared on a
// graph switch (the URLs point at the old graph's bytes, and a new graph could
// reuse a filename). Stores the in-flight promise so concurrent mounts of the
// same asset share one read.
const cache = new Map<string, Promise<string>>();

function mimeFromExt(path: string): string {
  const ext = path.split(".").pop()?.toLowerCase();
  switch (ext) {
    case "png":
      return "image/png";
    case "jpg":
    case "jpeg":
      return "image/jpeg";
    case "gif":
      return "image/gif";
    case "svg":
      return "image/svg+xml";
    case "webp":
      return "image/webp";
    // Video — so a blob-URL <video> gets a playable type (codec permitting).
    case "mp4":
    case "m4v":
      return "video/mp4";
    case "webm":
      return "video/webm";
    case "ogv":
      return "video/ogg";
    case "mov":
      return "video/quicktime";
    case "mkv":
      return "video/x-matroska";
    // Audio.
    case "mp3":
    case "mpeg":
      return "audio/mpeg";
    case "m4a":
    case "aac":
      return "audio/mp4";
    case "wav":
      return "audio/wav";
    case "ogg":
    case "oga":
      return "audio/ogg";
    case "opus":
      return "audio/opus";
    case "flac":
      return "audio/flac";
    default:
      return "application/octet-stream";
  }
}

/** A blob URL for the asset at `rel` (relative to `assets/`), reading it over IPC
 *  at most once per open graph. Resolves to "" if the asset is missing/unreadable. */
export function loadAssetBlob(rel: string): Promise<string> {
  const hit = cache.get(rel);
  if (hit) return hit;
  const p = (async () => {
    try {
      const bytes = await backend().readAsset(rel);
      if (!bytes.length) return "";
      return URL.createObjectURL(new Blob([bytes as unknown as BlobPart], { type: mimeFromExt(rel) }));
    } catch {
      return "";
    }
  })();
  cache.set(rel, p);
  return p;
}

/** A blob URL for an image at an ABSOLUTE local `path` outside the graph, read over
 *  the gated `read_local_image` IPC (raw-HTML `<img>` the user opted into). Cached
 *  under a `local:` key so repeat references share one read; resolves to "" if the
 *  opt-in is off or the file is missing/unreadable/too big. */
export function loadLocalImageBlob(path: string): Promise<string> {
  const key = `local:${path}`;
  const hit = cache.get(key);
  if (hit) return hit;
  const p = (async () => {
    try {
      const bytes = await backend().readLocalImage(path);
      if (!bytes.length) return "";
      return URL.createObjectURL(new Blob([bytes as unknown as BlobPart], { type: mimeFromExt(path) }));
    } catch {
      return "";
    }
  })();
  cache.set(key, p);
  return p;
}

/** Pre-populate the cache for `rel` from in-memory bytes (e.g. a just-pasted
 *  image) so it renders instantly, before the disk write completes — and so the
 *  inline loader never races readAsset against a not-yet-written file (which would
 *  cache an empty result). Revokes any prior URL for this rel. */
export function seedAssetBlob(rel: string, bytes: Uint8Array): string {
  const prior = cache.get(rel);
  if (prior) void prior.then((url) => url && URL.revokeObjectURL(url)).catch(() => {});
  const url = URL.createObjectURL(new Blob([bytes as unknown as BlobPart], { type: mimeFromExt(rel) }));
  cache.set(rel, Promise.resolve(url));
  return url;
}

/** Revoke every cached blob URL and empty the cache. Call on graph switch so the
 *  next graph's images aren't served stale blobs from the previous one. */
export function clearAssetBlobCache(): void {
  for (const p of cache.values()) {
    void p.then((url) => url && URL.revokeObjectURL(url)).catch(() => {});
  }
  cache.clear();
}

// Per-asset version counters (GH #38). An <img> served from a blob URL caches the
// bytes at creation, so after an EXTERNAL app overwrites the file we must both
// drop the cached blob AND change the reactive key a bound resource depends on —
// invalidating the cache alone won't re-run a resource keyed on an unchanged URL.
const [versions, setVersions] = createStore<Record<string, number>>({});

/** Reactive version for `rel` — fold into a resource's source key so a bump re-runs it. */
export function assetVersion(rel: string): number {
  return versions[rel] ?? 0;
}

/** Drop the cached blob for a single `rel` (revoke + delete), so the next read hits disk. */
export function invalidateAsset(rel: string): void {
  const prior = cache.get(rel);
  if (prior) void prior.then((url) => url && URL.revokeObjectURL(url)).catch(() => {});
  cache.delete(rel);
}

/** Invalidate `rel` AND bump its reactive version, forcing bound <img>s to re-read
 *  from disk. Use after an external editor overwrote the asset. */
export function refreshAsset(rel: string): void {
  invalidateAsset(rel);
  setVersions(rel, (versions[rel] ?? 0) + 1);
}
