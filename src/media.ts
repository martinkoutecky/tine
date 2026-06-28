// Media/asset helpers shared by insertion (naming + markdown) and rendering.
// Keeping the extension sets + the inserted-name policy in one place means the
// insert paths (paste / file-picker) and the renderer agree on what counts as
// image vs video vs audio.

import { assetNameFormat } from "./assetSettings";

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

/** Sanitize a filename STEM like OG: spaces/%/slashes → `_`, collapse + trim. */
function sanitizeStem(s: string): string {
  return s.replace(/[ %/\\]+/g, "_").replace(/_+/g, "_").replace(/^_+|_+$/g, "");
}

/** Substitute the `%`-tokens of an asset-name template (see assetSettings.ts).
 *  PURE — the date is injected — so the Settings live-preview and tests can call
 *  it without touching the clock. `original` is the source filename (absent for a
 *  clipboard paste). Always returns a non-empty, separator-free name that ends in
 *  an extension: an empty stem (paste with the default `%assetname` template)
 *  falls back to a `yyyymmdd-hhmmss` stamp, and a paste with no extension defaults
 *  to `.png` (clipboard images are PNG). The backend's `reserve_asset` still
 *  appends `_N` on a same-name collision, so bare-name templates stay safe. */
export function formatAssetName(template: string, original: string | undefined, now: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  const yyyy = String(now.getFullYear());
  const MM = p(now.getMonth() + 1);
  const dd = p(now.getDate());
  const HH = p(now.getHours());
  const mm = p(now.getMinutes());
  const ss = p(now.getSeconds());
  const dot = original ? original.lastIndexOf(".") : -1;
  // A named file keeps its real extension (case preserved — "just the filename");
  // a clipboard paste has none → png. An empty stem (paste) falls back to a
  // sortable stamp, so %assetname is never blank.
  const ext = dot > 0 ? original!.slice(dot + 1) : original ? "" : "png";
  const stamp = `${yyyy}${MM}${dd}-${HH}${mm}${ss}`;
  const stem = sanitizeStem(dot > 0 ? original!.slice(0, dot) : original ?? "") || stamp;
  const tokens: Record<string, string> = {
    "%yyyymmdd": `${yyyy}${MM}${dd}`,
    "%hhmmss": `${HH}${mm}${ss}`,
    "%yyyy": yyyy,
    "%yy": yyyy.slice(2),
    "%MM": MM,
    "%dd": dd,
    "%HH": HH,
    "%mm": mm,
    "%ss": ss,
    "%assetname": stem,
    "%ext": ext,
  };
  let out = template.replace(
    /%yyyymmdd|%hhmmss|%yyyy|%yy|%MM|%dd|%HH|%mm|%ss|%assetname|%ext/g,
    (m) => tokens[m] ?? m
  );
  // A filename is joined onto assets/ directly, so no separators may survive;
  // collapse accidental `..`; drop leading/trailing dots (an empty %ext leaves a
  // trailing ".") so a template can't yield a hidden or traversal name.
  out = out.replace(/[/\\]+/g, "_").replace(/\.{2,}/g, ".").replace(/^\.+|\.+$/g, "");
  // Never drop the real extension, even if the template omits %ext — a media file
  // with no extension wouldn't render. (If %ext is present, it already ends here.)
  if (ext && !out.toLowerCase().endsWith(`.${ext.toLowerCase()}`)) out = `${out}.${ext}`;
  return out || `${stamp}.png`; // last-ditch guard (template was all separators)
}

/** On-disk name for a newly-inserted asset, per the user's format template
 *  (Settings → Backups → Asset names; default = the plain original filename).
 *  New inserts only — existing files are never renamed. */
export function assetFileName(original?: string): string {
  return formatAssetName(assetNameFormat(), original, new Date());
}
