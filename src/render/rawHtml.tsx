import { t } from "../i18n";
import { Show, createEffect, createMemo, onCleanup, type JSX } from "solid-js";
import { rawHtmlPresentation } from "./htmlSanitize";
import { allowLocalFileImages } from "../localFileSettings";
import { acquireLocalImageBlob } from "../assetCache";
import type { SpanDomAttrs } from "./spans";

// Video-embed hosts (youtube/vimeo/loom/bilibili) that reject an embed when it
// arrives with no referrer — YouTube in particular fails with error 153. For
// those we send the app origin (`strict-origin-when-cross-origin`), matching OG
// (og-1.0.0 youtube.cljs:63). Every other pasted `<iframe>` keeps `no-referrer`
// so an arbitrary third-party embed still can't see where it was opened from.
const EMBED_REFERRER_HOSTS =
  /(?:^|\.)(?:youtube\.com|youtube-nocookie\.com|youtu\.be|vimeo\.com|loom\.com|bilibili\.com)$/i;
function embedReferrerPolicy(src: string): "strict-origin-when-cross-origin" | "no-referrer" {
  // Extract the host with a regex rather than `new URL()` — the referrer decision
  // must not depend on a global others can shim, and a malformed src stays private.
  const host = /^https?:\/\/([^/?#]+)/i.exec(src)?.[1]?.replace(/^[^@]*@/, "").replace(/:.*$/, "") ?? "";
  return EMBED_REFERRER_HOSTS.test(host) ? "strict-origin-when-cross-origin" : "no-referrer";
}

// Sandboxed-iframe rendering, reused for inline `inline_html` and block `raw_html`.
function renderIframe(src: string, width?: string, height?: string, spanAttrs?: SpanDomAttrs): JSX.Element {
  return (
    <span class="embed-iframe-wrap" style={{ ...(width || height ? { "max-width": "none" } : {}), ...(width ? { width } : {}), ...(height ? { "aspect-ratio": "auto", height } : {}) }} {...(spanAttrs ?? {})}>
      <iframe class="embed-iframe" src={src} sandbox="allow-scripts allow-same-origin allow-popups allow-forms" referrerpolicy={embedReferrerPolicy(src)} title={t("render-embed")} />
    </span>
  );
}
// Raw HTML (inline_html / raw_html): the https `<iframe>` subset renders as a
// SANDBOXED iframe (a deliberate embed feature, kept above the allowlist); all
// other raw HTML is sanitized to the shared allowlist (html_sanitize.ts, mirrored
// by the Rust export) and rendered LIVE. Handlers/`style`/`<script>` are stripped,
// so `innerHTML` is XSS-safe here. See ADR 0019.
export function renderRawHtml(text: string, spanAttrs?: SpanDomAttrs): JSX.Element {
  return <RawHtmlContent text={text} spanAttrs={spanAttrs} allowIframe />;
}

/** The sanitizer-only HTML insertion path. Unlike `renderRawHtml`, this helper
 * never performs iframe dispatch; macro-expanded Hiccup must enter here. */
export function renderSanitizedHtml(text: string, spanAttrs?: SpanDomAttrs): JSX.Element {
  return <RawHtmlContent text={text} spanAttrs={spanAttrs} />;
}

// Renders sanitized raw HTML, and — when the user has opted into local-file images
// (Settings → "Load local-file images") — swaps a blob URL into any `<img>` whose
// `src` was a local path (the sanitizer strips those, so we re-attach them by
// document-order match to the scanned paths). Off by default; see ADR 0019.
function RawHtmlContent(props: { text: string; spanAttrs?: SpanDomAttrs; allowIframe?: boolean }): JSX.Element {
  const presentation = createMemo(() => rawHtmlPresentation(props.text, props.allowIframe));
  let host: HTMLSpanElement | undefined;
  createEffect(() => {
    const shown = presentation(); // one projection for HTML, iframe and local resources
    if (!host || !allowLocalFileImages()) return;
    const locals = shown.localImages;
    if (!locals.some(Boolean)) return;
    let active = true;
    const releases: (() => void)[] = [];
    host.querySelectorAll("img").forEach((img, idx) => {
      const path = locals[idx];
      if (path) void acquireLocalImageBlob(path).then((lease) => {
        if (!active) {
          lease.release();
          return;
        }
        releases.push(lease.release);
        if (lease.url) img.src = lease.url;
      });
    });
    onCleanup(() => {
      active = false;
      releases.forEach((release) => release());
    });
  });
  return <Show when={presentation().iframe} fallback={<span ref={host} class="raw-html" innerHTML={presentation().html} {...(props.spanAttrs ?? {})} />}>
    {(frame) => renderIframe(frame().src, frame().width, frame().height, props.spanAttrs)}
  </Show>;
}

