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

/** Sanitize a filename stem for both the filesystem and the Markdown link that
 * will reference it. Keep Unicode names, but remove separators, control chars,
 * platform-forbidden punctuation, and Markdown destination delimiters. */
function sanitizeStem(s: string): string {
  return s
    .replace(/[\u0000-\u001f\u007f %/\\<>:"|?*#\[\]()]+/g, "_")
    .replace(/_+/g, "_")
    .replace(/^_+|_+$/g, "");
}

function dateParts(now: Date) {
  const p = (n: number) => String(n).padStart(2, "0");
  const yyyy = String(now.getFullYear());
  const MM = p(now.getMonth() + 1);
  const dd = p(now.getDate());
  const HH = p(now.getHours());
  const mm = p(now.getMinutes());
  const ss = p(now.getSeconds());
  return { yyyy, MM, dd, HH, mm, ss, stamp: `${yyyy}${MM}${dd}-${HH}${mm}${ss}` };
}

let clipboardPasteCounter = 0;

function clipboardPasteStem(now: Date): string {
  clipboardPasteCounter += 1;
  const ms = String(now.getMilliseconds()).padStart(3, "0");
  return `${dateParts(now).stamp}-${ms}-${clipboardPasteCounter}`;
}

function appendClipboardUniqueness(name: string, uniqueStem: string): string {
  if (name.includes(uniqueStem)) return name;
  const dot = name.lastIndexOf(".");
  if (dot > 0) return `${name.slice(0, dot)}-${uniqueStem}${name.slice(dot)}`;
  return `${name}-${uniqueStem}`;
}

/** Substitute the `%`-tokens of an asset-name template (see assetSettings.ts).
 *  PURE — the date is injected — so the Settings live-preview and tests can call
 *  it without touching the clock. `original` is the source filename (absent for a
 *  clipboard paste). Always returns a non-empty, separator-free name that ends in
 *  an extension: an empty stem (paste with the default `%assetname` template)
 *  falls back to a `yyyymmdd-hhmmss` stamp, and a paste with no extension defaults
 *  to `.png` (clipboard images are PNG). */
function formatAssetNameWithFallbackStem(
  template: string,
  original: string | undefined,
  now: Date,
  fallbackStem?: string,
  defaultExt: string = "png"
): string {
  const { yyyy, MM, dd, HH, mm, ss, stamp } = dateParts(now);
  const dot = original ? original.lastIndexOf(".") : -1;
  // A named file keeps its real extension (case preserved — "just the filename");
  // a clipboard paste has none → default ext (png for clipboard images, the real
  // capture ext for camera/mic). An empty stem (paste) falls back to a sortable
  // stamp, so %assetname is never blank.
  const rawExt = dot > 0 ? original!.slice(dot + 1) : original ? "" : defaultExt;
  const ext = rawExt.replace(/[^A-Za-z0-9.]+/g, "_").replace(/^\.+|\.+$/g, "");
  const stem = sanitizeStem(dot > 0 ? original!.slice(0, dot) : original ?? "") || fallbackStem || stamp;
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
  out = out
    .replace(/[\u0000-\u001f\u007f/\\<>:"|?*#\[\]()]+/g, "_")
    .replace(/_+/g, "_")
    .replace(/\.{2,}/g, ".")
    .replace(/^\.+|\.+$/g, "");
  // Never drop the real extension, even if the template omits %ext — a media file
  // with no extension wouldn't render. (If %ext is present, it already ends here.)
  if (ext && !out.toLowerCase().endsWith(`.${ext.toLowerCase()}`)) out = `${out}.${ext}`;
  return out || `${fallbackStem || stamp}.${defaultExt}`; // last-ditch guard (template was all separators)
}

export function formatAssetName(template: string, original: string | undefined, now: Date): string {
  return formatAssetNameWithFallbackStem(template, original, now);
}

/** On-disk name for a newly-inserted asset, per the user's format template
 *  (Settings → Backups → Asset names; default = the plain original filename).
 *  New inserts only — existing files are never renamed. Clipboard-paste
 *  candidates get millisecond + session-counter uniqueness before the backend's
 *  final collision de-dupe, so same-second optimistic links don't alias. */
export function assetFileName(original?: string): string {
  const now = new Date();
  if (original !== undefined) return formatAssetName(assetNameFormat(), original, now);
  const uniqueStem = clipboardPasteStem(now);
  return appendClipboardUniqueness(
    formatAssetNameWithFallbackStem(assetNameFormat(), undefined, now, uniqueStem),
    uniqueStem
  );
}

/** Paste-style unique asset name for a device CAPTURE (camera / mic), but with
 *  the capture's real extension instead of the clipboard-paste png default.
 *  Captures have no source filename, so — like a paste — they get a
 *  timestamp+ms+counter stem, guaranteeing uniqueness (the NAMED path would
 *  collapse to `photo.jpg`/`photo_1.jpg`). */
export function captureAssetFileName(ext: string): string {
  const clean = (ext || "").replace(/^\.+/, "").toLowerCase() || "bin";
  const now = new Date();
  const uniqueStem = clipboardPasteStem(now);
  return appendClipboardUniqueness(
    formatAssetNameWithFallbackStem(assetNameFormat(), undefined, now, uniqueStem, clean),
    uniqueStem
  );
}

/** File extension for a MediaRecorder blob, derived from its `mimeType`
 *  (which may carry a `;codecs=…` tail). Falls back to `webm` (the WebView's
 *  usual default) for anything unrecognized. */
export function recordingExt(mime: string): string {
  const base = (mime || "").split(";")[0].trim().toLowerCase();
  if (base === "audio/ogg") return "ogg";
  if (base === "audio/mp4") return "m4a";
  return "webm"; // audio/webm and unknown
}

export interface AssetMarkdownFixupTarget {
  insertedAt: number;
  occurrence: number;
}

function occurrenceAt(raw: string, needle: string, offset: number): number {
  let occurrence = 0;
  let pos = raw.indexOf(needle);
  while (pos >= 0) {
    if (pos === offset) return occurrence;
    if (pos > offset) return occurrence;
    occurrence += 1;
    pos = raw.indexOf(needle, pos + needle.length);
  }
  return occurrence;
}

export function insertedAssetMarkdownTarget(
  raw: string,
  markdown: string,
  insertedAt: number
): AssetMarkdownFixupTarget {
  return { insertedAt, occurrence: occurrenceAt(raw, markdown, insertedAt) };
}

function replaceAt(raw: string, start: number, len: number, replacement: string): string {
  return raw.slice(0, start) + replacement + raw.slice(start + len);
}

export function replaceInsertedAssetMarkdown(
  raw: string,
  candidate: string,
  stored: string,
  target: AssetMarkdownFixupTarget
): string {
  const from = assetMarkdown(candidate);
  const to = assetMarkdown(stored);
  if (!from || from === to) return raw;
  const positions: number[] = [];
  let pos = raw.indexOf(from);
  while (pos >= 0) {
    positions.push(pos);
    pos = raw.indexOf(from, pos + from.length);
  }
  if (!positions.length) return raw;
  const exact = positions[target.occurrence];
  if (exact !== undefined) return replaceAt(raw, exact, from.length, to);
  const afterOriginalOffset = positions.find((p) => p >= target.insertedAt);
  return replaceAt(raw, afterOriginalOffset ?? positions[0], from.length, to);
}

export function removeInsertedAssetMarkdown(
  raw: string,
  candidate: string,
  target: AssetMarkdownFixupTarget
): string {
  const from = assetMarkdown(candidate);
  if (!from) return raw;
  const positions: number[] = [];
  let pos = raw.indexOf(from);
  while (pos >= 0) {
    positions.push(pos);
    pos = raw.indexOf(from, pos + from.length);
  }
  if (!positions.length) return raw;
  const exact = positions[target.occurrence];
  if (exact !== undefined) return replaceAt(raw, exact, from.length, "");
  const afterOriginalOffset = positions.find((p) => p >= target.insertedAt);
  return replaceAt(raw, afterOriginalOffset ?? positions[0], from.length, "");
}
