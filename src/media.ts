// Media/asset helpers shared by insertion (naming + markdown) and rendering.
// Keeping the extension sets + the inserted-name policy in one place means the
// insert paths (paste / file-picker) and the renderer agree on what counts as
// image vs video vs audio.

// Image extensions Tine renders as <img> (unchanged from the prior inline check).
export const IMAGE_EXTS = ["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "avif"];
// OG's media format sets (deps/.../config.cljs): reused so a clone graph stays
// compatible. We recognize these for INSERTION (the `![]` media form, like OG)
// and RENDERING (inline <video>/<audio> with an external-open fallback).
export const VIDEO_EXTS = ["mp4", "webm", "mov", "flv", "avi", "mkv", "m4v", "ogv"];
export const AUDIO_EXTS = ["mp3", "ogg", "oga", "wav", "m4a", "flac", "wma", "aac", "opus", "mpeg"];

export type MediaKind = "image" | "video" | "audio";

/** Lowercased extension (no dot) of a filename or URL, ignoring any `?`/`#` tail. */
export function extOf(nameOrUrl: string): string {
  const q = nameOrUrl.split(/[?#]/)[0];
  const dot = q.lastIndexOf(".");
  return dot >= 0 ? q.slice(dot + 1).toLowerCase() : "";
}

/** Which media kind a filename/URL is, or `null` if it's not embeddable media. */
export function mediaKind(nameOrUrl: string): MediaKind | null {
  const e = extOf(nameOrUrl);
  if (IMAGE_EXTS.includes(e)) return "image";
  if (VIDEO_EXTS.includes(e)) return "video";
  if (AUDIO_EXTS.includes(e)) return "audio";
  return null;
}

/** Markdown for an inserted asset: the `![…]` media form for image/video/audio
 *  (matching OG, which reuses the image syntax for all media), else a plain link
 *  (e.g. a PDF, which Tine opens in its side viewer). */
export function assetMarkdown(name: string): string {
  return mediaKind(name) ? `![](../assets/${name})` : `[${name}](../assets/${name})`;
}

/** `yyyymmdd-hhmmss` local-time stamp — sortable AND human-readable. */
function stamp(): string {
  const d = new Date();
  const p = (n: number) => String(n).padStart(2, "0");
  return (
    `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}` +
    `-${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`
  );
}

/** On-disk name for a newly-inserted asset: `<yyyymmdd-hhmmss>-<stem>.<ext>` —
 *  timestamp FIRST so a plain name-sort in `assets/` is also a chronological
 *  sort (newest/oldest grouped together), then the original filename so the file
 *  is still recognizable. A nameless paste becomes `<stamp>.png`.
 *  Sanitized like OG (spaces/%/slashes → `_`). The backend's `reserve_asset` still
 *  appends `_N` for the rare same-second collision. New inserts only — existing
 *  files are never renamed. */
export function assetFileName(original?: string): string {
  const ts = stamp();
  if (!original) return `${ts}.png`;
  const dot = original.lastIndexOf(".");
  const ext = dot > 0 ? original.slice(dot) : "";
  const stem = (dot > 0 ? original.slice(0, dot) : original)
    .replace(/[ %/\\]+/g, "_")
    .replace(/_+/g, "_")
    .replace(/^_+|_+$/g, "");
  return `${ts}${stem ? "-" + stem : ""}${ext}`;
}
