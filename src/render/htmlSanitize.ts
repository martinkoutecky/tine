import DOMPurify from "dompurify";

// The raw-HTML render policy — the single allowlist that both render surfaces
// apply to raw inline/block HTML embedded in a Markdown/Org block.
//
// WHY a policy and not a parse: lsdoc emits `inline_html`/`raw_html` nodes with
// the source bytes VERBATIM (byte-parity with mldoc — mldoc doesn't sanitize
// either). Sanitizing is a *render-layer* safety decision, so it lives at each
// render boundary, NOT in the parser. Tine has two such boundaries in two
// languages, so this list is MIRRORED in crates/tine-core/src/html_sanitize.rs
// (ammonia, for the static-HTML export). fixtures/html-sanitize-cases.json
// contract-tests that the two agree. Keep them in lockstep — if you add a tag
// or attribute here, add it there and add a fixture.
//
// Threat model: notes aren't self-authored (Syncthing sync, import, paste,
// shared graphs), and in Tauri an injected `onerror=`/`<script>` can call Tine's
// IPC to read/write the whole graph or exfiltrate via `fetch`; the export
// re-publishes raw HTML as served content. So: allowlist tags + attrs, drop all
// event handlers and `style`, and lean on DOMPurify's URI safety (which blocks
// `javascript:` etc.).

/** Tags that survive sanitization — inline text formatting plus a small set of
 *  containers, links, images, and native playback elements. OG 6e7afa8eb sends
 *  raw HTML through DOMPurify (src/main/frontend/security.cljs:5-11; raw block
 *  insertion at src/main/frontend/components/block.cljs:3258-3261), whose HTML
 *  profile preserves native media. `audio`/`video` provide playback and `source` provides codec
 *  alternatives without admitting an executable or embedded browsing context.
 *  Deliberately excludes `<iframe>`, `<script>`, `<object>`, `<embed>`, forms,
 *  and anything executable. (The app renders a sandboxed-https `<iframe>` via a
 *  SEPARATE path in `renderRawHtml`, layered above this allowlist.) */
export const RAW_HTML_TAGS = [
  "b", "strong", "i", "em", "u", "ins", "del", "s", "strike", "sub", "sup",
  "mark", "kbd", "abbr", "small", "code", "cite", "q", "span", "br",
  "p", "div", "blockquote", "details", "summary", "a", "img",
  "audio", "video", "source",
];

/** Attributes that survive, across all allowed tags. Note the absence of
 *  `style` (positioning/tracking), `autoplay`, and any `on*` handler.
 *  `controls` exposes user-driven playback; `loop`/`muted` retain playback
 *  state without starting it; `preload` is the browser's media fetch hint;
 *  `poster` is the video placeholder; `type` lets `<source>` advertise its
 *  codec. `width`/`height` were already admitted for images and also bound the
 *  video box. URL-bearing `src`/`poster` receive the scheme guard below. */
export const RAW_HTML_ATTRS = [
  "class", "title", "href", "src", "alt", "width", "height", "open",
  "controls", "loop", "muted", "preload", "poster", "type",
];

/** Defense-in-depth on top of DOMPurify: reject `javascript:` in `src`/`poster`
 *  even under control-character-obfuscated spellings. `data:` is deliberately
 *  NOT denied here — DOMPurify (= OG's sanitizer, security.cljs:5-11) allows
 *  `data:` URIs on media tags (DATA_URI_TAGS: img/audio/video/source), and
 *  base64-embedded images in raw HTML are a real user payload; scripts do not
 *  execute in an image/media src context. */
function hasDeniedResourceScheme(value: string): boolean {
  const compact = value.replace(/[\u0000-\u0020]/g, "");
  const scheme = /^([a-z][a-z0-9+.-]*):/i.exec(compact)?.[1]?.toLowerCase();
  return scheme === "javascript";
}

// --- Local-file `<img>` support (opt-in; see localFileSettings + ADR 0019) ---
// The sanitizer strips a `file:`/absolute-path `src`, so a raw-HTML `<img>` pointing
// at a local file loses its src. When the user has opted in, the app matches the
// sanitized `<img>` elements (in document order) back to these scanned paths and
// swaps in a blob URL read over the gated IPC. Pure/string-only so it's unit-tested.

const IMG_RE = /<img\b[^>]*>/gi;
const SRC_RE = /\bsrc\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))/i;

/** True if `src` is a local filesystem path (not a web / data / blob URL). */
function isLocalSrc(src: string): boolean {
  return /^(?:file:\/\/|\/(?!\/)|[a-zA-Z]:[\\/]|\\\\)/.test(src);
}

/** Normalize a local `<img>` `src` (possibly `file://…`, percent-encoded) to a
 *  filesystem path, or null if it isn't a local path. */
export function localImagePath(src: string): string | null {
  if (!isLocalSrc(src)) return null;
  let p = src.replace(/^file:\/\//i, "");
  try {
    p = decodeURIComponent(p);
  } catch {
    /* leave as-is if it isn't valid percent-encoding */
  }
  // file:///C:/x → /C:/x → C:/x  (drop the leading slash before a drive letter)
  if (/^\/[a-zA-Z]:[\\/]/.test(p)) p = p.slice(1);
  return p;
}

/** For each `<img>` in `text` (document order), its local filesystem path, or null
 *  for a web/data/blob/relative src. Aligns 1:1 with the `<img>` elements the
 *  sanitized HTML yields — `img` is allowlisted, so every one survives sanitizing. */
export function rawHtmlLocalImages(text: string): (string | null)[] {
  const out: (string | null)[] = [];
  for (const m of text.matchAll(IMG_RE)) {
    const s = SRC_RE.exec(m[0]);
    const src = s ? (s[1] ?? s[2] ?? s[3] ?? "") : "";
    out.push(src ? localImagePath(src) : null);
  }
  return out;
}

/** Sanitize a raw-HTML fragment down to {@link RAW_HTML_TAGS}/{@link RAW_HTML_ATTRS}.
 *  Returns a safe HTML string for `innerHTML`. Everything outside the allowlist
 *  (tags, attributes, `javascript:`/`data:text` URIs) is stripped; the text
 *  content of stripped elements is preserved where DOMPurify preserves it. */
export function sanitizeRawHtml(html: string): string {
  const clean = DOMPurify.sanitize(html, {
    ALLOWED_TAGS: RAW_HTML_TAGS,
    ALLOWED_ATTR: RAW_HTML_ATTRS,
    ALLOW_DATA_ATTR: false,
  });

  // Work on DOMPurify's already-safe output so entity/whitespace-obfuscated
  // schemes are compared as the browser will interpret them, without installing
  // a process-global DOMPurify hook shared with the editor's paste sanitizer.
  const template = document.createElement("template");
  template.innerHTML = clean;
  for (const element of template.content.querySelectorAll<HTMLElement>("[src], [poster]")) {
    for (const attr of ["src", "poster"] as const) {
      const value = element.getAttribute(attr);
      if (value !== null && hasDeniedResourceScheme(value)) element.removeAttribute(attr);
    }
  }
  return template.innerHTML;
}
