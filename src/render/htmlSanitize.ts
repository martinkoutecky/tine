import policy from "../../fixtures/html-sanitize-policy.json";
import DOMPurify from "dompurify";
import { deferAnchorHref, hasWebUrlPrefix, previewResourceDecision } from "../linkSchemePolicy";
import { color_only_styles } from "./wasm/lsdoc_wasm.js";

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
 *  separately, see `colorOnlyStyles`), `autoplay`, and any `on*` handler.
 *  `controls` exposes user-driven playback; `loop`/`muted` retain playback
 *  state without starting it; `preload` is the browser's media fetch hint;
 *  `poster` is the video placeholder; `type` lets `<source>` advertise its
 *  codec. `width`/`height` were already admitted for images and also bound the
 *  video box. URL-bearing `src`/`poster` receive the scheme guard below. */
export const RAW_HTML_ATTRS = policy.browser.attributes;

// --- Local-file `<img>` support (opt-in; see localFileSettings + ADR 0019) ---
// The sanitizer strips a `file:`/absolute-path `src`, so a raw-HTML `<img>` pointing
// at a local file loses its src. When the user has opted in, the app matches the
// sanitized `<img>` elements (in document order) back to these scanned paths and
// swaps in a blob URL read over the gated IPC. The paths are projected from the same inert DOM used for sanitizing.

// --- Colour-only `style` (GH #243) ---
// OG's DOMPurify default keeps `style` on hiccup/raw HTML, which is how Logseq users
// write `[:span {:style "color: red"} "x"]`. Tine keeps only colour declarations, so a
// shared note can recolor text but never position, resize, hide or fetch anything.
// The rule is the export sanitizer's (`color_only_style` in tine-core's html_sanitize,
// through wasm), so the app and a published page keep the same declarations.

/** The colour declarations each `style` value keeps ("" if none), in order: one
 *  wasm call for all of them, equal values computed once. Values travel as one
 *  string plus UTF-16 lengths (no separator an attribute value could contain).
 *  O(total bytes). */
export function colorOnlyStyles(values: readonly string[]): string[] {
  if (values.length === 0) return [];
  const slot = new Map<string, number>();
  const unique: string[] = [];
  const index = values.map((value) => {
    let i = slot.get(value);
    if (i === undefined) slot.set(value, (i = unique.push(value) - 1));
    return i;
  });
  const lengths = Uint32Array.from(unique, (value) => value.length);
  const keptLengths = new Uint32Array(unique.length);
  const joined = color_only_styles(unique.join(""), lengths, keptLengths);
  const kept: string[] = [];
  let at = 0;
  for (const n of keptLengths) kept.push(joined.slice(at, (at += n)));
  return index.map((i) => kept[i]);
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

/** `text` parsed as an HTML fragment in an inert document: the one parse raw HTML is
 * rendered from (so its character references are decoded exactly as the page shows them).
 * O(fragment bytes). */
export function inertHtml(text: string): HTMLDivElement {
  const root = document.implementation.createHTMLDocument("").createElement("div");
  root.innerHTML = text;
  return root;
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
  const root = inertHtml(text);
  const frame = allowIframe ? root.querySelector("iframe") : null;
  const src = frame?.getAttribute("src");
  if (frame && src && hasWebUrlPrefix(src)) {
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
  const paths = new WeakMap<Element, string>();
  for (const element of root.querySelectorAll("[src], [poster]")) {
    for (const attr of ["src", "poster"] as const) {
      const value = element.getAttribute(attr);
      if (value === null) continue;
      const decision = previewResourceDecision(value);
      if (decision === "drop") element.removeAttribute(attr);
      if (decision === "local-candidate" && attr === "src" && element.localName === "img") {
        const path = localImagePath(value);
        if (path !== null) paths.set(element, path);
      }
    }
  }
  // DOMPurify runs IN_PLACE, so surviving elements keep identity: capture the colour
  // declarations first and put them back on whatever survives.
  const colors = new Map<Element, string>();
  const styled = [...root.querySelectorAll("[style]")];
  colorOnlyStyles(styled.map((element) => element.getAttribute("style") ?? "")).forEach((kept, i) => {
    if (kept) colors.set(styled[i], kept);
  });
  DOMPurify.sanitize(root, {
    ALLOWED_TAGS: RAW_HTML_TAGS,
    ALLOWED_ATTR: RAW_HTML_ATTRS,
    ALLOW_DATA_ATTR: false,
    IN_PLACE: true,
  });
  for (const anchor of root.querySelectorAll("a[href]")) deferAnchorHref(anchor);
  for (const [element, kept] of colors) if (root.contains(element)) element.setAttribute("style", kept);
  return { html: root.innerHTML, localImages: [...root.querySelectorAll("img")].map((image) => paths.get(image) ?? null), iframe: null };
}
