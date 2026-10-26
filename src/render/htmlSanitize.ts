import policy from "../../fixtures/html-sanitize-policy.json";
import DOMPurify from "dompurify";

// The raw-HTML render policy — the single allowlist that both render surfaces
// apply to raw inline/block HTML embedded in a Markdown/Org block.
//
// WHY a policy and not a parse: lsdoc emits `inline_html`/`raw_html` nodes with
// the source bytes VERBATIM (byte-parity with mldoc — mldoc doesn't sanitize
// either). Sanitizing is a *render-layer* safety decision, so it lives at each
// render boundary, NOT in the parser. Tine has two such boundaries in two
// languages. fixtures/html-sanitize-policy.json owns the inventories; the
// browser uses global attributes, native export uses tag-scoped attributes and
// explicit URL schemes/rel. These engine policies intentionally remain distinct.
// fixtures/html-sanitize-cases.json checks their common safety outcomes.
//
// Threat model: notes aren't self-authored (Syncthing sync, import, paste,
// shared graphs), and in Tauri an injected `onerror=`/`<script>` can call Tine's
// IPC to read/write the whole graph or exfiltrate via `fetch`; the export
// re-publishes raw HTML as served content. So: allowlist tags + attrs, drop all
// event handlers and (all but colour declarations of) `style`, and lean on DOMPurify's URI safety (which blocks
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
export const RAW_HTML_TAGS = policy.tags;

/** Attributes that survive, across all allowed tags. Note the absence of
 *  `style` here (positioning/tracking; colour declarations are carried over
 *  separately, see `colorOnlyStyle`), `autoplay`, and any `on*` handler.
 *  `controls` exposes user-driven playback; `loop`/`muted` retain playback
 *  state without starting it; `preload` is the browser's media fetch hint;
 *  `poster` is the video placeholder; `type` lets `<source>` advertise its
 *  codec. `width`/`height` were already admitted for images and also bound the
 *  video box. URL-bearing `src`/`poster` receive the scheme guard below. */
export const RAW_HTML_ATTRS = policy.browser.attributes;

/** Defense-in-depth on top of DOMPurify: reject `javascript:` in `src`/`poster`
 *  even under control-character-obfuscated spellings. `data:` is deliberately
 *  NOT denied here — DOMPurify (= OG's sanitizer, security.cljs:5-11) allows
 *  `data:` URIs on media tags (DATA_URI_TAGS: img/audio/video/source), and
 *  base64-embedded images in raw HTML are a real user payload; scripts do not
 *  execute in an image/media src context. */
function hasDeniedResourceScheme(value: string): boolean {
  const compact = value.replace(/[\u0000-\u0020]/g, "");
  const scheme = /^([a-z][a-z0-9+.-]*):/i.exec(compact)?.[1]?.toLowerCase();
  return scheme !== undefined && policy.browser.deniedResourceSchemes.includes(scheme);
}

// --- Local-file `<img>` support (opt-in; see localFileSettings + ADR 0019) ---
// The sanitizer strips a `file:`/absolute-path `src`, so a raw-HTML `<img>` pointing
// at a local file loses its src. When the user has opted in, the app matches the
// sanitized `<img>` elements (in document order) back to these scanned paths and
// swaps in a blob URL read over the gated IPC. The paths are projected from the same inert DOM used for sanitizing.

// --- Colour-only `style` (GH #243) ---
// OG's DOMPurify default keeps `style` on hiccup/raw HTML, which is how Logseq users
// write `[:span {:style "color: red"} "x"]`. Tine keeps only colour declarations, so a
// shared note can recolor text but never position, resize, hide or fetch anything:
// property names come from the policy's `browser.styleProperties`, and a value must be
// one closed colour literal (#hex, a bare colour word, or a flat rgb()/hsl()/oklch()-
// style function with only numbers, `%`, `/`, `,` and `.`): no url(), var(), calc(),
// image-set() or `!important`. Anything else in the attribute is dropped.
const STYLE_PROPERTIES: readonly string[] = policy.browser.styleProperties;
const COLOR_VALUE = /^(?:#(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})|[a-z]{3,30}|(?:rgb|rgba|hsl|hsla|hwb|lab|lch|oklab|oklch)\(\s*[-+0-9a-z.%/,\s]{1,80}\))$/i;
const COLOR_FUNCTION_BANNED = /(?:var|calc|url|env|attr|image|min|max|clamp)/i;

/** The colour declarations of a `style` attribute value, serialized; "" if none. */
export function colorOnlyStyle(value: string): string {
  const kept: string[] = [];
  for (const declaration of value.split(";")) {
    const colon = declaration.indexOf(":");
    if (colon < 0) continue;
    const property = declaration.slice(0, colon).trim().toLowerCase();
    const v = declaration.slice(colon + 1).trim();
    if (!STYLE_PROPERTIES.includes(property) || !COLOR_VALUE.test(v)) continue;
    // A function body may contain letters (`from`, `none`, channel names) but never a nested call.
    if (v.includes("(") && COLOR_FUNCTION_BANNED.test(v.slice(v.indexOf("(")))) continue;
    kept.push(`${property}: ${v}`);
  }
  return kept.join("; ");
}

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

export interface HtmlIframe { src: string; width?: string; height?: string }
export interface HtmlPresentation { html: string; localImages: (string | null)[]; iframe: HtmlIframe | null }

/** Raw HTML presentation owner: parse once in an inert DOM, optionally select
 * the sandboxed HTTP(S) iframe, otherwise sanitize in place. Local image paths
 * align with surviving sanitized images, including removed ancestor subtrees.
 * O(fragment bytes/nodes); no IPC, fetching, shared hooks or retained DOM cache.
 * Browser and native export security policies stay distinct in the shared JSON.
 */
export function rawHtmlPresentation(text: string, allowIframe = false): HtmlPresentation {
  const root = document.implementation.createHTMLDocument("").createElement("div");
  root.innerHTML = text;
  const frame = allowIframe ? root.querySelector("iframe") : null;
  const src = frame?.getAttribute("src");
  if (frame && src && /^https?:\/\//i.test(src)) {
    const dimension = (name: "width" | "height") => {
      const attr = frame.getAttribute(name)?.trim();
      const style = frame.style.getPropertyValue(name);
      // Inline CSS overrides HTML presentation attributes, as in a browser.
      // Numeric HTML dimensions are pixels, never unitless CSS lengths.
      if (/^\d+(?:\.\d+)?(?:px|%)$/i.test(style)) return style;
      if (!attr || !/^\d+(?:\.\d+)?(?:px|%)?$/i.test(attr)) return undefined;
      return /^\d+(?:\.\d+)?$/.test(attr) ? `${attr}px` : attr;
    };
    return { html: "", localImages: [], iframe: { src, width: dimension("width"), height: dimension("height") } };
  }
  const paths = new WeakMap<Element, string | null>();
  for (const image of root.querySelectorAll("img")) paths.set(image, localImagePath(image.getAttribute("src") ?? ""));
  // DOMPurify runs IN_PLACE, so surviving elements keep identity: capture the colour
  // declarations first and put them back on whatever survives.
  const colors = new Map<Element, string>();
  for (const element of root.querySelectorAll("[style]")) {
    const kept = colorOnlyStyle(element.getAttribute("style") ?? "");
    if (kept) colors.set(element, kept);
  }
  DOMPurify.sanitize(root, {
    ALLOWED_TAGS: RAW_HTML_TAGS,
    ALLOWED_ATTR: RAW_HTML_ATTRS,
    ALLOW_DATA_ATTR: false,
    IN_PLACE: true,
  });
  for (const [element, kept] of colors) if (root.contains(element)) element.setAttribute("style", kept);
  for (const element of root.querySelectorAll<HTMLElement>("[src], [poster]")) {
    for (const attr of ["src", "poster"] as const) {
      const value = element.getAttribute(attr);
      if (value !== null && hasDeniedResourceScheme(value)) element.removeAttribute(attr);
    }
  }
  return { html: root.innerHTML, localImages: [...root.querySelectorAll("img")].map((image) => paths.get(image) ?? null), iframe: null };
}

/** Paths for exactly the sanitized image sequence, derived by the same DOM
 * owner as presentation (no source regex). O(fragment bytes/nodes). */
export function rawHtmlLocalImages(text: string): (string | null)[] {
  return rawHtmlPresentation(text).localImages;
}

/** Safe HTML insertion string, without iframe dispatch. O(fragment bytes/nodes). */
export function sanitizeRawHtml(html: string): string {
  return rawHtmlPresentation(html).html;
}
