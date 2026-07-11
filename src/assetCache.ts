import { createStore } from "solid-js/store";
import { backend } from "./backend";

// Cache of graph-asset blob URLs keyed by path relative to `assets/`. Without it
// every <img> mount (re-render, scroll back into view, or a second reference to
// the same image) re-reads the file over IPC and mints a fresh blob URL, then
// revokes it on unmount — pure churn on image-heavy pages. The cache holds one
// blob URL per asset for the lifetime of the open graph; it MUST be cleared on a
// graph switch (the URLs point at the old graph's bytes, and a new graph could
// reuse a filename). Stores the in-flight promise so concurrent mounts of the
// same asset share one read. Bound both count and retained backing bytes: Blob
// URLs keep their Blob alive even after every image unmounts.
interface CacheEntry {
  promise: Promise<string>;
  bytes: number;
  leases: number;
  evicted: boolean;
}
export interface BlobLease { url: string; release: () => void }
const cache = new Map<string, CacheEntry>();
// Entries evicted from the retained LRU while still displayed remain keyed here
// (but do not count toward the idle cache cap), so a repeated reference shares
// the same live Blob instead of rereading/duplicating it.
const liveEntries = new Map<string, CacheEntry>();
const MAX_CACHE_ENTRIES = 128;
const MAX_CACHE_BYTES = 128 * 1024 * 1024;
const MAX_IMAGE_BYTES = 32 * 1024 * 1024;
let cacheBytes = 0;
const MAX_ACTIVE_READS = 2;
let activeReads = 0;
const readQueue: (() => void)[] = [];

function startQueuedReads() {
  while (activeReads < MAX_ACTIVE_READS && readQueue.length) {
    activeReads++;
    readQueue.shift()!();
  }
}

function limitedRead<T>(task: () => Promise<T>): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    readQueue.push(() => {
      void task().then(resolve, reject).finally(() => {
        activeReads--;
        startQueuedReads();
      });
    });
    startQueuedReads();
  });
}

export function __assetCacheStatsForTests(): { entries: number; bytes: number } {
  return { entries: cache.size, bytes: cacheBytes };
}

function touch(key: string, entry: CacheEntry) {
  cache.delete(key);
  cache.set(key, entry);
}

function evictEntry(key: string, entry: CacheEntry, reusable = true) {
  if (cache.get(key) !== entry) return;
  cache.delete(key);
  cacheBytes = Math.max(0, cacheBytes - entry.bytes);
  entry.evicted = true;
  if (entry.leases > 0 && reusable) {
    liveEntries.set(key, entry);
  } else if (entry.leases === 0) {
    void entry.promise.then((url) => url && URL.revokeObjectURL(url)).catch(() => {});
  }
}

function prune(protectedKey: string) {
  while (cache.size > MAX_CACHE_ENTRIES || cacheBytes > MAX_CACHE_BYTES) {
    // Pending entries contain no Blob bytes and represent active consumers. Do
    // not evict them merely because many images were requested at once; reads
    // are separately bounded by MAX_ACTIVE_READS.
    const oldest = [...cache.entries()].find(([key, entry]) => key !== protectedKey && entry.bytes > 0);
    if (!oldest) break;
    evictEntry(oldest[0], oldest[1]);
  }
}

function cachedBlob(key: string, read: () => Promise<Uint8Array>, typePath: string): CacheEntry {
  const hit = cache.get(key);
  if (hit) {
    touch(key, hit);
    return hit;
  }
  const live = liveEntries.get(key);
  if (live) return live;
  const entry: CacheEntry = { promise: Promise.resolve(""), bytes: 0, leases: 0, evicted: false };
  // Publish identity before starting the limiter: its first two tasks begin
  // synchronously, and must see themselves in the cache.
  cache.set(key, entry);
  prune(key);
  entry.promise = limitedRead(async () => {
    try {
      // It may have been evicted while waiting behind earlier image reads. Do
      // not start an IPC allocation for a URL nobody is waiting to cache.
      if (cache.get(key) !== entry) return "";
      const bytes = await read();
      if (!bytes.length) return "";
      const url = URL.createObjectURL(new Blob([bytes as unknown as BlobPart], { type: mimeFromExt(typePath) }));
      if (cache.get(key) === entry) {
        entry.bytes = bytes.byteLength;
        cacheBytes += entry.bytes;
        prune(key);
      }
      return url;
    } catch {
      if (cache.get(key) === entry) cache.delete(key);
      return "";
    }
  });
  return entry;
}

async function acquire(entry: CacheEntry): Promise<BlobLease> {
  entry.leases++;
  let released = false;
  const release = () => {
    if (released) return;
    released = true;
    entry.leases = Math.max(0, entry.leases - 1);
    if (entry.evicted && entry.leases === 0) {
      for (const [key, candidate] of liveEntries) {
        if (candidate === entry) liveEntries.delete(key);
      }
      void entry.promise.then((url) => url && URL.revokeObjectURL(url)).catch(() => {});
    }
  };
  const url = await entry.promise;
  if (!url) release();
  return { url, release };
}

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
export function acquireAssetBlob(rel: string): Promise<BlobLease> {
  return acquire(cachedBlob(rel, () => backend().readAsset(rel, MAX_IMAGE_BYTES), rel));
}

/** A blob URL for an image at an ABSOLUTE local `path` outside the graph, read over
 *  the gated `read_local_image` IPC (raw-HTML `<img>` the user opted into). Cached
 *  under a `local:` key so repeat references share one read; resolves to "" if the
 *  opt-in is off or the file is missing/unreadable/too big. */
export function acquireLocalImageBlob(path: string): Promise<BlobLease> {
  const key = `local:${path}`;
  return acquire(cachedBlob(key, () => backend().readLocalImage(path), path));
}

/** Pre-populate the cache for `rel` from in-memory bytes (e.g. a just-pasted
 *  image) so it renders instantly, before the disk write completes — and so the
 *  inline loader never races readAsset against a not-yet-written file (which would
 *  cache an empty result). Revokes any prior URL for this rel. */
export function seedAssetBlob(rel: string, bytes: Uint8Array): string {
  const prior = cache.get(rel);
  if (prior) evictEntry(rel, prior, false);
  const livePrior = liveEntries.get(rel);
  if (livePrior) {
    liveEntries.delete(rel);
    livePrior.evicted = true;
    if (livePrior.leases === 0) {
      void livePrior.promise.then((old) => old && URL.revokeObjectURL(old)).catch(() => {});
    }
  }
  const url = URL.createObjectURL(new Blob([bytes as unknown as BlobPart], { type: mimeFromExt(rel) }));
  if (bytes.byteLength > MAX_CACHE_BYTES) {
    // The caller already owns these bytes (paste/capture), so this is not a read
    // amplification path. Avoid graph-lifetime retention; revoke after the
    // mounted image has had ample time to consume the URL.
    setTimeout(() => URL.revokeObjectURL(url), 60_000);
    return url;
  }
  const entry: CacheEntry = {
    promise: Promise.resolve(url),
    bytes: bytes.byteLength,
    leases: 0,
    evicted: false,
  };
  cache.set(rel, entry);
  cacheBytes += entry.bytes;
  prune(rel);
  return url;
}

/** Revoke every cached blob URL and empty the cache. Call on graph switch so the
 *  next graph's images aren't served stale blobs from the previous one. */
export function clearAssetBlobCache(): void {
  for (const [key, entry] of [...cache.entries()]) evictEntry(key, entry, false);
  for (const entry of liveEntries.values()) entry.evicted = true;
  liveEntries.clear();
  cacheBytes = 0;
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
  if (prior) evictEntry(rel, prior, false);
  const live = liveEntries.get(rel);
  if (live) {
    liveEntries.delete(rel);
    live.evicted = true;
    if (live.leases === 0) {
      void live.promise.then((url) => url && URL.revokeObjectURL(url)).catch(() => {});
    }
  }
}

/** Invalidate `rel` AND bump its reactive version, forcing bound <img>s to re-read
 *  from disk. Use after an external editor overwrote the asset. */
export function refreshAsset(rel: string): void {
  invalidateAsset(rel);
  setVersions(rel, (versions[rel] ?? 0) + 1);
}
