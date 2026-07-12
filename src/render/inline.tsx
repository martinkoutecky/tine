// Inline markdown -> Solid components. Produces real interactive DOM (clickable
// [[links]] and #tags), not an innerHTML string. Used to render a block when it
// is not being edited.

import { For, Show, createEffect, createMemo, createResource, createSignal, onCleanup, useContext, type JSX } from "solid-js";
import { Dynamic } from "solid-js/web";
import { mediaKind } from "../media";
import { openPage, openPageInNewTab, openPageAtBlock, focusBlock } from "../router";
import { refClickZoom } from "../copySettings";
import { isJournalTitle } from "../journal";
import { openPdf, openPageInSidebar, openBlockInSidebar, openPageContextMenu, openBlockRefContextMenu, setLightbox, setAudioPlayer, graphEpoch, graphMeta, pushToast } from "../ui";
import { copyImageFromSrc } from "../copyImage";
import { parseBlock, parserReady } from "./parse";
import type { Inline, Url, MacroInline, TimestampInline, EmailValue, Block as AstBlock, Format } from "./ast";
import type { PageKind } from "../types";
import { timestampText } from "./renderedText";
import { EmojiText } from "./emoji";
import { sanitizeRawHtml, rawHtmlLocalImages } from "./htmlSanitize";
import { allowLocalFileImages } from "../localFileSettings";
import { pageIcon } from "../pageIconBatch";
import { typographic } from "./typography";
import { coarseSpanAttrs, literalSpanAttrs, plainSpanAttrs, typographicPlainSpanAttrs, type SpanDomAttrs } from "./spans";
import { typographyMode } from "../ui";
import { visibleBody } from "./block";
import { AstBody } from "./body";
import { backend } from "../backend";
import { acquireAssetBlob, acquireLocalImageBlob, assetVersion } from "../assetCache";
import { mediaEditorForAsset } from "../mediaEditors";
import { resolveMediaEditorCommand } from "../mediaEditorSettings";
import { refreshAssetOnReturn } from "../assetRefresh";
import { isMobilePlatform } from "../nativeChrome";
import { resolveBlockBatched } from "../resolveBatch";
import { doc, setRaw, formatForPage, formatForBlock } from "../store";
import { PaneContext, focusedPaneId, openRouteInOtherPane } from "../panes";
import { QueryMacro, EmbedMacro, VideoMacro, TweetMacro, YoutubeTimestamp, ClozeMacro, ZoteroMacro } from "../components/Macro";
import { NamespaceMacro } from "../components/Namespace";
import { guideTargetForLink, isGuidePageName } from "../guide";
import { PeekPopup, PeekContext, capBlockTree } from "./PeekPopup";
import { annotationInfoForBlock, pdfFileFromPreBlock } from "../editor/annotation";


// ===========================================================================
// AST renderer (lsdoc). Renders an `Inline[]` produced by the Rust parser to
// interactive DOM — the replacement for the parseInline → renderSeg path.
// Reuses every component above (EmojiText, MathView, BlockRefView, AssetImage,
// MediaEmbed, the macros). See subagent-tasks/notes/ast-render-contract.md.
// ===========================================================================

// Shared `{{macro}}` dispatch, keyed off a reconstructed body string for built-ins.
// User macros get parser-supplied args from the AST path so quoted commas survive.
function renderMacroBody(raw: string, blockId?: string, userArgs?: string[]): JSX.Element {
  const body = raw.trimStart();
  if (/^query\b/i.test(body)) return <QueryMacro body={body} blockId={blockId} />;
  if (/^embed\b/i.test(body)) return <EmbedMacro body={body} />;
  if (/^youtube-timestamp\b/i.test(body)) return <YoutubeTimestamp body={body} />;
  if (/^(video|youtube|vimeo|bilibili)\b/i.test(body)) return <VideoMacro body={body} />;
  if (/^(tweet|twitter)\b/i.test(body)) return <TweetMacro body={body} />;
  if (/^img\b/i.test(body)) return renderImgMacro(body);
  if (/^cloze\b/i.test(body)) return <ClozeMacro body={body} />;
  if (/^zotero-(imported|linked)-file\b/i.test(body)) return <ZoteroMacro body={body} />;
  if (/^namespace\b/i.test(body)) {
    const root = body.replace(/^namespace\s+/i, "").trim();
    if (root) return <NamespaceMacro root={root} />;
  }
  const um = /^(\S+)\s*([\s\S]*)$/.exec(body);
  const userMacros = graphMeta()?.macros;
  if (um && userMacros && Object.prototype.hasOwnProperty.call(userMacros, um[1])) {
    const args = userArgs ?? (um[2].trim() ? um[2].split(",").map((a) => a.trim()) : []);
    return <UserMacroView name={um[1]} template={userMacros[um[1]]} args={args} blockId={blockId} />;
  }
  return <span class="macro">{`{{${raw}}}`}</span>;
}

/** Render a parsed inline run (lsdoc `Inline[]`) to interactive DOM. */
export function renderInlines(inlines: Inline[], blockId?: string, spanMode = true): JSX.Element {
  return <For each={inlines}>{(s) => renderInline(s, blockId, spanMode)}</For>;
}

function renderInline(s: Inline, blockId?: string, spanMode = true): JSX.Element {
  switch (s.k) {
    case "plain": {
      // Render-time typographic replacement (`->`→`→`, `--`→`–`, …) is a Tine
      // opinion applied ONLY to plain text — code/links/math/tags are other node
      // kinds, so they're excluded for free. Source keeps the ASCII.
      const text = typographyMode() === "render" ? typographic(s.text) : s.text;
      const attrs = spanMode
        ? text === s.text
          ? plainSpanAttrs(s.span, s.span_map)
          : typographicPlainSpanAttrs(s.text, s.span, s.span_map)
        : undefined;
      return attrs ? <span {...attrs}><EmojiText text={text} /></span> : <EmojiText text={text} />;
    }
    case "code":
    case "verbatim":
      return (
        <span class="inline-copy-wrap">
          <code class="inline-code" {...((spanMode ? literalSpanAttrs(s.text, s.span) : undefined) ?? {})}>{s.text}</code>
          <CopyButton text={s.text} title="Copy code" class="copy-inline" />
        </span>
      );
    case "break":
    case "hardbreak":
      // Both render as <br>: today body.tsx joins every in-block line with <br>,
      // and a soft `break` is exactly such an in-block newline — match that look.
      return <br {...((spanMode ? coarseSpanAttrs(s.span) : undefined) ?? {})} />;
    case "emphasis": {
      const inner = renderInlines(s.children, blockId, spanMode);
      const attrs = (spanMode ? coarseSpanAttrs(s.span) : undefined) ?? {};
      switch (s.emph) {
        case "Bold": return <strong {...attrs}>{inner}</strong>;
        case "Italic": return <em {...attrs}>{inner}</em>;
        case "Strike_through": return <del {...attrs}>{inner}</del>;
        case "Highlight": return <mark {...attrs}>{inner}</mark>;
        case "Underline": return <u {...attrs}>{inner}</u>;
      }
      return inner;
    }
    case "subscript":
      return <sub {...((spanMode ? coarseSpanAttrs(s.span) : undefined) ?? {})}>{renderInlines(s.children, blockId, spanMode)}</sub>;
    case "superscript":
      return <sup {...((spanMode ? coarseSpanAttrs(s.span) : undefined) ?? {})}>{renderInlines(s.children, blockId, spanMode)}</sup>;
    case "link":
      return renderLink(s, blockId, spanMode);
    case "nested_link":
      // Logseq `[[a [[b]] c]]` — best-effort: route the whole inner as a page ref.
      return <PageRef name={s.content} blockId={blockId} spanAttrs={spanMode ? coarseSpanAttrs(s.span) : undefined} />;
    case "target":
      return <span class="org-target" {...((spanMode ? coarseSpanAttrs(s.span) : undefined) ?? {})}>{s.text}</span>;
    case "tag":
      return <PageRef name={astText(s.children)} blockId={blockId} tag spanAttrs={spanMode ? coarseSpanAttrs(s.span) : undefined} />;
    case "macro":
      return renderMacroBody(macroBody(s), blockId, s.args);
    case "latex":
      return <MathView tex={s.body} display={s.mode === "Displayed"} spanAttrs={spanMode ? coarseSpanAttrs(s.span) : undefined} />;
    case "timestamp":
      return renderTimestamp(s);
    case "fnref":
      return <sup class="footnote-ref" {...((spanMode ? coarseSpanAttrs(s.span) : undefined) ?? {})}>{s.name}</sup>;
    case "inline_html":
      return renderRawHtml(s.text, spanMode ? coarseSpanAttrs(s.span) : undefined);
    case "email":
      return renderEmail(s.text, spanMode ? coarseSpanAttrs(s.span) : undefined);
    case "entity":
      return spanMode && s.span ? <span {...(coarseSpanAttrs(s.span) ?? {})}>{s.unicode}</span> : <>{s.unicode}</>;
    case "hiccup":
      // Inline Clojure-hiccup `[:tag …]` — literal text for now (see ast.ts). Edge case.
      return spanMode && s.span ? <span {...(coarseSpanAttrs(s.span) ?? {})}>{s.v}</span> : <>{s.v}</>;
  }
}

// Flatten an inline run to plain text (tag names, link/block-ref labels, and the
// page-ref name fallback) — mirrors refs.rs `tag_text` / OG `get-tag`.
export function astText(inlines: Inline[]): string {
  let out = "";
  for (const s of inlines) {
    switch (s.k) {
      case "plain": case "code": case "verbatim": out += s.text; break;
      case "emphasis": case "subscript": case "superscript": out += astText(s.children); break;
      case "tag": out += "#" + astText(s.children); break;
      case "link": out += s.label && s.label.length ? astText(s.label) : urlDest(s.url); break;
      case "nested_link": out += s.content; break;
      case "target": out += s.text; break;
      case "entity": out += s.unicode; break;
      case "latex": out += s.body; break;
      case "hiccup": out += s.v; break;
    }
  }
  return out;
}

// The destination string of a link/image `url`.
function urlDest(url: Url): string {
  switch (url.type) {
    case "page_ref":
    case "block_ref":
    case "search":
    case "file":
    case "embed_data":
      return url.v;
    case "complex":
      return url.protocol && url.link != null ? `${url.protocol}://${url.link}` : url.link ?? "";
  }
}

function macroBody(s: MacroInline): string {
  return s.args.length ? `${s.name} ${s.args.join(", ")}` : s.name;
}

const PEEK_OPEN_MS = 350;
const PEEK_CLOSE_MS = 150;
const PEEK_BLOCK_CAP = 50;

function createPeekBridge(disabled: () => boolean) {
  const [open, setOpen] = createSignal(false);
  let openT: ReturnType<typeof setTimeout> | undefined;
  let closeT: ReturnType<typeof setTimeout> | undefined;
  const clearOpen = () => {
    if (openT) {
      clearTimeout(openT);
      openT = undefined;
    }
  };
  const clearClose = () => {
    if (closeT) {
      clearTimeout(closeT);
      closeT = undefined;
    }
  };
  const anchorEnter = () => {
    if (disabled()) return;
    clearClose();
    clearOpen();
    openT = setTimeout(() => setOpen(true), PEEK_OPEN_MS);
  };
  const anchorLeave = () => {
    clearOpen();
    if (disabled()) return;
    clearClose();
    closeT = setTimeout(() => setOpen(false), PEEK_CLOSE_MS);
  };
  const popupEnter = () => {
    if (disabled()) return;
    clearClose();
  };
  const popupLeave = () => {
    if (disabled()) return;
    clearClose();
    closeT = setTimeout(() => setOpen(false), PEEK_CLOSE_MS);
  };
  onCleanup(() => {
    clearOpen();
    clearClose();
  });
  return { open, anchorEnter, anchorLeave, popupEnter, popupLeave };
}

// A `[[page]]` / `#tag` anchor — shared by page_ref links, bare refs, and #tags.
function PageRef(props: { name: string; alias?: JSX.Element; tag?: boolean; blockId?: string; spanAttrs?: SpanDomAttrs }): JSX.Element {
  const pane = useContext(PaneContext);
  const insidePeek = useContext(PeekContext);
  let anchorEl: HTMLAnchorElement | undefined;
  const sourcePage = () => (props.blockId ? doc.byId[props.blockId]?.page : undefined);
  const targetName = () => guideTargetForLink(props.name, sourcePage());
  // The referenced page's `icon::`, shown as a prefix like OG (and Tine's own page
  // title / namespace macro). Emoji route through EmojiText → Twemoji SVG, since
  // WebKitGTK paints color-emoji webfonts blank. Reactive + batched + cached per
  // graph; an icon-less graph costs one IPC and no re-render (see pageIconBatch).
  const icon = () => (isGuidePageName(targetName()) ? null : pageIcon(targetName()));
  const kind = (): PageKind => (isGuidePageName(targetName()) ? "page" : isJournalTitle(targetName()) ? "journal" : "page");
  const open = (e: MouseEvent) => {
    e.stopPropagation();
    if (e.ctrlKey || e.metaKey)
      openRouteInOtherPane({ kind: "page", name: targetName(), pageKind: kind() }, pane?.paneId ?? focusedPaneId());
    else if (e.shiftKey && !isGuidePageName(targetName())) openPageInSidebar(targetName(), kind());
    else openPage(targetName(), kind());
  };

  // Hover peek (GH #40): after a short dwell, fetch the target page and show its
  // read-only RefBlocks tree in a portaled popup. The fetch is lazy and guarded:
  // guide pages and links already inside a peek never arm another preview.
  const peek = createPeekBridge(() => insidePeek || isGuidePageName(targetName()));
  const [preview] = createResource(
    () => (peek.open() && !isGuidePageName(targetName()) ? `${targetName()}\0${graphEpoch()}` : null),
    () => backend().getPage(targetName(), kind()),
  );
  const capped = createMemo(() => capBlockTree(preview()?.blocks ?? [], PEEK_BLOCK_CAP));

  return (
    <>
      <a
        ref={anchorEl}
        class={props.tag ? "tag" : "page-ref"}
        {...(props.spanAttrs ?? {})}
        // Shift+click opens the page in the sidebar (via `open`); suppress the
        // browser's native shift-range-selection so the main editor's text isn't
        // selected as a side effect (GH #42).
        onMouseDown={(e) => { if (e.shiftKey) e.preventDefault(); }}
        onClick={open}
        onMouseEnter={peek.anchorEnter}
        onMouseLeave={peek.anchorLeave}
        onAuxClick={(e) => {
          if (e.button === 1) {
            e.preventDefault();
            e.stopPropagation();
            // Middle-click belongs to the pane that rendered the link. Relying
            // on the globally focused router races pointer-focus tracking and
            // sent split-view tabs to the previously focused/top pane (GH #87).
            if (pane) pane.router.openPageInNewTab(targetName(), kind());
            else openPageInNewTab(targetName(), kind());
          }
        }}
        onContextMenu={(e) => {
          e.preventDefault();
          e.stopPropagation();
          if (!isGuidePageName(targetName())) openPageContextMenu(e.clientX, e.clientY, targetName());
        }}
      >
        <Show when={icon()}>
          <span class="page-icon page-ref-icon"><EmojiText text={icon()!} /></span>
        </Show>
        <Show when={props.tag} fallback={
          <Show when={props.alias} fallback={<><span class="bracket">[[</span>{props.name}<span class="bracket">]]</span></>}>
            {props.alias}
          </Show>
        }>
          #{props.name}
        </Show>
      </a>
      <Show when={peek.open() && preview() && capped().blocks.length > 0}>
        <PeekPopup
          anchor={() => anchorEl}
          title={<span class="peek-popup-title-name"><EmojiText text={props.name} /></span>}
          blocks={() => capped().blocks}
          page={targetName()}
          pageKind={kind()}
          truncatedCount={() => capped().truncated}
          onPointerEnter={peek.popupEnter}
          onPointerLeave={peek.popupLeave}
        />
      </Show>
    </>
  );
}

// One-click copy affordance (#24 — the logseq-copy-code / logseq-copy-url QoL).
// Copies RAW source text (off the AST, never the rendered DOM) through the
// backend clipboard path — WebKitGTK's navigator.clipboard is unreliable
// ([[tine-webkitgtk-confirm]]) — and toasts. Shared by inline code + links
// (here) and fenced code blocks (body.tsx). `class` positions it per site.
export function CopyButton(props: { text: string; title: string; class?: string }): JSX.Element {
  const onCopy = (e: MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    void backend()
      .writeText(props.text)
      .then(() => pushToast("Copied to clipboard", "success"))
      .catch(() => pushToast("Couldn’t copy to clipboard", "error"));
  };
  return (
    <button
      class={`copy-btn${props.class ? " " + props.class : ""}`}
      title={props.title}
      aria-label={props.title}
      onClick={onCopy}
    >
      <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2">
        <rect x="9" y="9" width="11" height="11" rx="2" />
        <path d="M5 15V5a2 2 0 0 1 2-2h10" />
      </svg>
    </button>
  );
}

function renderLink(s: Extract<Inline, { k: "link" }>, blockId?: string, spanMode = true): JSX.Element {
  const url = s.url;
  const spanAttrs = spanMode ? coarseSpanAttrs(s.span) : undefined;
  if (url.type === "page_ref") {
    const alias = s.label && s.label.length ? renderInlines(s.label, blockId, spanMode) : undefined;
    return <PageRef name={url.v} alias={alias} blockId={blockId} spanAttrs={spanAttrs} />;
  }
  if (url.type === "block_ref") {
    const label = s.label && s.label.length ? astText(s.label) : undefined;
    return <BlockRefView id={url.v} label={label} spanAttrs={spanAttrs} />;
  }
  const dest = urlDest(url);
  if (s.image) {
    const { width, height } = parseImageMetaBrace(s.metadata);
    const alt = s.label && s.label.length ? astText(s.label) : "";
    if (/\.pdf$/i.test(dest)) return <PdfAssetLink dest={dest} label={alt} spanAttrs={spanAttrs} />;
    const k = mediaKind(dest);
    if (k === "video" || k === "audio")
      return <MediaEmbed url={dest} kind={k} alt={alt} width={width} blockId={blockId} spanAttrs={spanAttrs} />;
    return <AssetImage url={dest} alt={alt} width={width} height={height} blockId={blockId} spanAttrs={spanAttrs} />;
  }
  if (/\.pdf$/i.test(dest)) {
    const labelStr = s.label && s.label.length ? astText(s.label) : pdfFilenameFromDest(dest);
    return <PdfAssetLink dest={dest} label={labelStr} spanAttrs={spanAttrs} />;
  }
  return (
    <span class="link-copy-wrap">
      <a
        class="external-link"
        href={dest}
        {...(spanAttrs ?? {})}
        onClick={(e) => { e.preventDefault(); e.stopPropagation(); void backend().openExternal(dest); }}
      >
        <Show when={s.label && s.label.length} fallback={dest}>{renderInlines(s.label!, blockId, spanMode)}</Show>
      </a>
      <CopyButton text={dest} title="Copy link" class="copy-inline" />
    </span>
  );
}

function pdfFilenameFromDest(dest: string): string {
  const normalized = dest.replace(/\\/g, "/");
  const rel = assetRelPath(normalized);
  const path = rel ?? normalized;
  return path.split("/").pop() || path;
}

function PdfAssetLink(props: { dest: string; label?: string; spanAttrs?: SpanDomAttrs }): JSX.Element {
  const filename = pdfFilenameFromDest(props.dest);
  const label = props.label || filename;
  return (
    <a class="external-link pdf-link" {...(props.spanAttrs ?? {})} onClick={(e) => { e.stopPropagation(); openPdf(filename, label); }}>
      📄 {label}
    </a>
  );
}

// Logseq image-metadata brace reader (`{:width 200, :height 100}` or
// `{:width "40%"}`) — same logic the old parseInline used; kept here so it
// survives parseInline.ts's eventual removal.
function parseImageMetaBrace(brace: string | undefined): { width?: string; height?: string } {
  if (!brace) return {};
  const out: { width?: string; height?: string } = {};
  const w = /:width\s+"?([0-9]+%?|[0-9]+px)"?/.exec(brace);
  const h = /:height\s+"?([0-9]+%?|[0-9]+px)"?/.exec(brace);
  if (w) out.width = /^\d+$/.test(w[1]) ? `${w[1]}px` : w[1];
  if (h) out.height = /^\d+$/.test(h[1]) ? `${h[1]}px` : h[1];
  return out;
}

// Org timestamp inline → the styled `<…>`(active)/`[…]`(inactive) badge. The
// display string comes from the ONE formatter shared with rendered-text export
// (render/renderedText.ts timestampText).
function renderTimestamp(s: TimestampInline): JSX.Element {
  const { text, active } = timestampText(s);
  return (
    <span class="org-timestamp" classList={{ inactive: !active }} {...(coarseSpanAttrs(s.span) ?? {})}>
      {text}
    </span>
  );
}

// Sandboxed-iframe rendering, reused for inline `inline_html` and block `raw_html`.
function renderIframe(src: string, width?: string, height?: string, spanAttrs?: SpanDomAttrs): JSX.Element {
  return (
    <span class="embed-iframe-wrap" style={{ ...(width ? { width } : {}), ...(height ? { "aspect-ratio": "auto", height } : {}) }} {...(spanAttrs ?? {})}>
      <iframe class="embed-iframe" src={src} sandbox="allow-scripts allow-same-origin allow-popups allow-forms" referrerpolicy="no-referrer" title="embed" />
    </span>
  );
}
// Raw HTML (inline_html / raw_html): the https `<iframe>` subset renders as a
// SANDBOXED iframe (a deliberate embed feature, kept above the allowlist); all
// other raw HTML is sanitized to the shared allowlist (html_sanitize.ts, mirrored
// by the Rust export) and rendered LIVE. Handlers/`style`/`<script>` are stripped,
// so `innerHTML` is XSS-safe here. See ADR 0019.
export function renderRawHtml(text: string, spanAttrs?: SpanDomAttrs): JSX.Element {
  const m = /<iframe\b([^>]*)>/i.exec(text);
  if (m) {
    const attrs = m[1];
    const src = /src\s*=\s*["']([^"']+)["']/i.exec(attrs)?.[1];
    if (src && /^https?:\/\//i.test(src)) {
      const attrWidth = /width\s*=\s*["']?(\d+px|\d+%|\d+)["']?/i.exec(attrs)?.[1];
      const attrHeight = /height\s*=\s*["']?(\d+px|\d+%|\d+)["']?/i.exec(attrs)?.[1];
      const style = !attrWidth || !attrHeight ? /style\s*=\s*(["'])(.*?)\1/i.exec(attrs)?.[2] : undefined;
      const styleWidth = style ? /(?:^|;)\s*width\s*:\s*(\d+px|\d+%)(?=\s*(?:;|$))/i.exec(style)?.[1] : undefined;
      const styleHeight = style ? /(?:^|;)\s*height\s*:\s*(\d+px|\d+%)(?=\s*(?:;|$))/i.exec(style)?.[1] : undefined;
      const width = attrWidth ?? styleWidth;
      const height = attrHeight ?? styleHeight;
      return renderIframe(src, width, height, spanAttrs);
    }
  }
  return <RawHtmlContent text={text} spanAttrs={spanAttrs} />;
}

// Renders sanitized raw HTML, and — when the user has opted into local-file images
// (Settings → "Load local-file images") — swaps a blob URL into any `<img>` whose
// `src` was a local path (the sanitizer strips those, so we re-attach them by
// document-order match to the scanned paths). Off by default; see ADR 0019.
function RawHtmlContent(props: { text: string; spanAttrs?: SpanDomAttrs }): JSX.Element {
  const clean = createMemo(() => sanitizeRawHtml(props.text));
  let host: HTMLSpanElement | undefined;
  createEffect(() => {
    clean(); // re-run if the sanitized markup changes
    if (!host || !allowLocalFileImages()) return;
    const locals = rawHtmlLocalImages(props.text);
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
  return <span ref={host} class="raw-html" innerHTML={clean()} {...(props.spanAttrs ?? {})} />;
}

function renderEmail(text: EmailValue, spanAttrs?: SpanDomAttrs): JSX.Element {
  let addr = "";
  if (typeof text === "string") addr = text;
  else if (text && typeof text === "object") {
    const lp = (text as Record<string, unknown>).local_part;
    const dom = (text as Record<string, unknown>).domain;
    if (typeof lp === "string" && typeof dom === "string") addr = `${lp}@${dom}`;
  }
  const href = `mailto:${addr}`;
  return (
    <a class="external-link" href={href} {...(spanAttrs ?? {})} onClick={(e) => { e.preventDefault(); e.stopPropagation(); void backend().openExternal(href); }}>
      {addr}
    </a>
  );
}

// KaTeX is heavy (hundreds of KB) — load it on first actual math, not eagerly
// into the initial bundle, and cache the module promise so it loads once.
let katexMod: Promise<typeof import("katex").default> | null = null;
function loadKatex() {
  if (!katexMod) {
    katexMod = (async () => {
      const m = await import("katex");
      // mhchem extends KaTeX with \ce{…}; registers on the (shared) katex instance.
      await import("katex/contrib/mhchem");
      return m.default;
    })();
  }
  return katexMod;
}

// KaTeX-typeset math. KaTeX output is trusted HTML, so innerHTML is safe here.
// Shows the raw TeX until KaTeX has loaded, then upgrades. The typeset result is
// memoized so re-renders of this span don't re-run the (non-trivial) typesetter.
export function MathView(props: { tex: string; display: boolean; spanAttrs?: SpanDomAttrs }): JSX.Element {
  const [katex] = createResource(loadKatex);
  const html = createMemo(() => {
    const k = katex();
    if (!k) return null;
    try {
      return k.renderToString(props.tex, { throwOnError: false, displayMode: props.display });
    } catch {
      return null;
    }
  });
  return (
    <span class="math" classList={{ "math-display": props.display }} {...(props.spanAttrs ?? {})}>
      <Show when={html()} fallback={<span class="math-raw">{props.tex}</span>}>
        <span innerHTML={html()!} />
      </Show>
    </span>
  );
}

// Resolve the path of a graph asset relative to the `assets/` dir.
function assetRelPath(url: string): string | null {
  const normalized = url.replace(/\\/g, "/");
  const i = normalized.toLowerCase().indexOf("assets/");
  return i === -1 ? null : normalized.slice(i + "assets/".length);
}

// The width `%` CSS resolves against is the nearest BLOCK-level ancestor's
// content box, so a drag-resize must measure that (not the inline image itself)
// to turn a pixel drag into a meaningful percentage.
function blockRefWidth(el: HTMLElement): number {
  let p = el.parentElement;
  while (p) {
    const d = getComputedStyle(p).display;
    if (d !== "inline" && d !== "inline-block" && d !== "contents") {
      return p.clientWidth || p.getBoundingClientRect().width;
    }
    p = p.parentElement;
  }
  return el.getBoundingClientRect().width;
}

// Persist a resized media width back into its block's raw text as the OG-native
// `{:width "N%"}` brace. Works for images AND video/audio — all use the
// `![alt](url)` form. We rewrite THIS token's trailing `{...}` only (matched by
// its exact alt+url), so other text/media in the block are untouched; width is
// stored as a percentage (Martin's choice — survives column width changes) and
// as a quoted string so it stays valid EDN OG can also read.
function writeMediaWidth(blockId: string, alt: string, url: string, pct: number) {
  const node = doc.byId[blockId];
  if (!node) return;
  const esc = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const re = new RegExp(`(!\\[${esc(alt)}\\]\\(${esc(url)}\\))(\\{[^}]*\\})?`);
  const next = node.raw.replace(re, `$1{:width "${pct}%"}`);
  if (next !== node.raw) setRaw(blockId, next);
}

// Remove THIS media token (`![alt](url){...}`) from its block's raw — the "trash"
// affordance drops the reference (OG's delete-asset-of-block! also rewrites the
// block, then unlinks the file). Eats one adjacent space so we don't leave a
// double space behind. Matched by exact alt+url, like writeMediaWidth.
function removeMediaToken(blockId: string, alt: string, url: string) {
  const node = doc.byId[blockId];
  if (!node) return;
  const esc = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const re = new RegExp(` ?!\\[${esc(alt)}\\]\\(${esc(url)}\\)(\\{[^}]*\\})?`);
  const next = node.raw.replace(re, "");
  if (next !== node.raw) setRaw(blockId, next);
}

// Image embed: external URLs load directly; graph assets (`../assets/x.png`)
// are read from disk via the backend and shown as a blob URL. When rendered
// inside a real block (`blockId` set, not the lightbox/capture scratch), a
// corner grip lets you drag-resize; the result is written back as a width %.
function AssetImage(props: {
  url: string;
  alt: string;
  width?: string;
  height?: string;
  blockId?: string;
  spanAttrs?: SpanDomAttrs;
}): JSX.Element {
  // Width sizes the WRAPPER, not the <img>: an inline-block sized by a
  // percentage child doesn't shrink to it, so the grip (positioned against the
  // wrapper) would otherwise stay at full column width after a resize. With the
  // width on the wrapper and the image filling it (width:100%), the wrapper is
  // exactly the image's box and the grip sits at the image's real corner. No
  // width set → the wrapper shrink-wraps the image's natural size.
  const wrapStyle = () => ({
    ...(props.width ? { width: props.width } : {}),
  });
  const imgStyle = () => ({
    ...(props.width ? { width: "100%" } : {}),
    ...(props.height ? { height: props.height } : {}),
  });
  const external = /^(https?:|data:|blob:)/.test(props.url);
  const rel = () => (external ? null : assetRelPath(props.url));
  // Served from a shared, graph-scoped blob cache so repeated references and
  // re-mounts don't re-read the file or mint duplicate blob URLs. The cache owns
  // the URL's lifetime (cleared on graph switch), so we don't revoke on unmount.
  // The source key folds in the per-asset version (GH #38) so an external edit —
  // which bumps that version after invalidating the cache — re-runs this resource
  // and the <img> re-reads the new bytes from disk.
  const [diskSrc, setDiskSrc] = createSignal("");
  createEffect(() => {
    const r = rel();
    if (!r) {
      setDiskSrc("");
      return;
    }
    // Subscribe to external-edit invalidation before starting the acquisition.
    assetVersion(r);
    let active = true;
    let release: (() => void) | undefined;
    setDiskSrc("");
    void acquireAssetBlob(r).then((lease) => {
      if (!active) {
        lease.release();
        return;
      }
      release = lease.release;
      setDiskSrc(lease.url);
    });
    onCleanup(() => {
      active = false;
      release?.();
    });
  });
  const src = () => (external ? props.url : diskSrc());

  let wrapEl: HTMLSpanElement | undefined;
  let imgEl: HTMLImageElement | undefined;
  const onGripDown = (e: PointerEvent) => {
    if (!wrapEl || !props.blockId) return;
    e.preventDefault();
    e.stopPropagation(); // don't start a block drag / open the lightbox
    const refW = blockRefWidth(wrapEl);
    const startX = e.clientX;
    const startW = wrapEl.getBoundingClientRect().width;
    if (imgEl) imgEl.style.width = "100%"; // make the image track the wrapper during the drag
    const move = (me: PointerEvent) => {
      const w = Math.max(24, Math.min(refW, startW + (me.clientX - startX)));
      if (wrapEl) wrapEl.style.width = `${w}px`; // live feedback during the drag
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      const w = wrapEl ? wrapEl.getBoundingClientRect().width : startW;
      const pct = Math.max(5, Math.min(100, Math.round((w / refW) * 100)));
      writeMediaWidth(props.blockId!, props.alt, props.url, pct);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  // Hover actions (graph assets in a real block only — like OG's asset action bar):
  // copy the image to the OS clipboard, and trash it (drop the block reference + move
  // the file to the recoverable trash).
  const assetActions = () => !external && !!props.blockId;
  const onCopyAsset = (e: MouseEvent) => {
    e.stopPropagation();
    const s = src();
    if (!s) return;
    copyImageFromSrc(s)
      .then(() => pushToast("Image copied to clipboard", "success"))
      .catch(() => pushToast("Couldn't copy the image", "error"));
  };
  const onTrashAsset = async (e: MouseEvent) => {
    e.stopPropagation();
    const name = assetRelPath(props.url);
    if (!name || !props.blockId) return;
    const ok = await backend().confirm(
      `Move "${name}" to the trash and remove it from this block? It stays recoverable in logseq/.tine-trash.`,
      "Trash asset",
    );
    if (!ok) return;
    removeMediaToken(props.blockId, props.alt, props.url); // drop the reference first (saves the block)
    try {
      await backend().trashAsset(name);
      pushToast("Asset moved to trash", "success");
    } catch (err) {
      pushToast(`Couldn't trash the asset (${String(err)})`, "error");
    }
  };

  // "Edit in <external editor>" (GH #38): shown only for a graph asset whose
  // filename matches a media-editor registry entry (e.g. *.drawio.svg). Launches
  // the configured command (empty = OS opener) and marks the asset to refresh
  // when Tine regains focus after the edit.
  const editor = () =>
    assetActions() && !isMobilePlatform ? mediaEditorForAsset(assetRelPath(props.url)) : undefined;
  const onEditAsset = async (e: MouseEvent) => {
    e.stopPropagation();
    const name = assetRelPath(props.url);
    const ed = editor();
    if (!name || !ed) return;
    const cmd = await resolveMediaEditorCommand(ed);
    void backend()
      .editAssetExternal(name, cmd)
      .catch(() => pushToast(`Couldn't open ${ed.label.replace(/^Edit in /, "")}`, "error"));
    refreshAssetOnReturn(name);
  };

  return (
    <Show
      when={external || src()}
      fallback={<span class="inline-image-missing">🖼 {props.alt || assetRelPath(props.url)}</span>}
    >
      <span ref={wrapEl} class="inline-image-wrap" style={wrapStyle()} {...(props.spanAttrs ?? {})}>
        <img
          ref={imgEl}
          class="inline-image"
          src={src()!}
          alt={props.alt}
          style={imgStyle()}
          onClick={(e) => { e.stopPropagation(); setLightbox(src()!); }}
        />
        <Show when={assetActions()}>
          <span class="asset-action-bar" aria-hidden="true">
            <Show when={editor()}>
              <button class="asset-action-btn" title={editor()!.label} onClick={onEditAsset}>
                <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
                  <path d="M12 20h9" />
                  <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4Z" />
                </svg>
              </button>
            </Show>
            <button class="asset-action-btn" title="Copy image" onClick={onCopyAsset}>
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
                <rect x="9" y="9" width="11" height="11" rx="2" />
                <path d="M5 15V5a2 2 0 0 1 2-2h10" />
              </svg>
            </button>
            <button class="asset-action-btn asset-action-trash" title="Trash image" onClick={onTrashAsset}>
              <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M3 6h18M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2m2 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" />
              </svg>
            </button>
          </span>
        </Show>
        <Show when={props.blockId}>
          <span class="img-resize-grip" title="Drag to resize" onPointerDown={onGripDown} />
        </Show>
      </span>
    </Show>
  );
}

// `{{img url}}`, `{{img url W H}}`, `{{img url W H class}}`, or
// `{{img url left|right|center}}` (OG). Numeric W/H → px; left/right/center wrap
// the image in an alignment span (float / centered block); a 4th non-align token is
// treated as a user CSS class. Reuses AssetImage (graph assets + external URLs).
function renderImgMacro(body: string): JSX.Element {
  const toks = body.replace(/^img\s*/i, "").trim().split(/\s+/).filter(Boolean);
  const url = toks[0] ?? "";
  const ALIGN = new Set(["left", "right", "center"]);
  let width: string | undefined;
  let height: string | undefined;
  let align: string | undefined;
  let cls: string | undefined;
  if (toks.length === 2 && ALIGN.has(toks[1].toLowerCase())) {
    align = toks[1].toLowerCase();
  } else {
    width = toks[1];
    height = toks[2];
    if (toks[3]) (ALIGN.has(toks[3].toLowerCase()) ? (align = toks[3].toLowerCase()) : (cls = toks[3]));
  }
  const px = (v?: string) => (v ? (/^\d+$/.test(v) ? `${v}px` : v) : undefined);
  const img = <AssetImage url={url} alt="" width={px(width)} height={px(height)} />;
  if (!align && !cls) return img;
  const classes: Record<string, boolean> = {};
  if (align) classes[`img-align-${align}`] = true;
  if (cls) classes[cls] = true;
  return (
    <span class="img-macro" classList={classes}>
      {img}
    </span>
  );
}

// Video/audio asset: try inline playback through Tauri's range-aware native asset
// controls>), and on a media error — typically WebKitGTK lacking the mp4/h264
// codec — fall back to a click-to-open chip that launches the OS default player.
// Matches the user's "inline when it works, otherwise open externally" intent.
function MediaEmbed(props: {
  url: string;
  kind: "video" | "audio";
  alt?: string;
  width?: string;
  blockId?: string;
  spanAttrs?: SpanDomAttrs;
}): JSX.Element {
  const [failed, setFailed] = createSignal(false);
  const [blobFallback, setBlobFallback] = createSignal("");
  // Audio has no fullscreen; the "Expand" button (below) opens a wide overlay
  // player with a waveform scrubber instead of stretching the inline control.
  const external = /^(https?:|data:|blob:)/.test(props.url);
  const rel = () => assetRelPath(props.url);
  // Graph media uses native range requests; never copy a multi-GB file into a Blob.
  const [blob] = createResource(
    () => (external ? null : rel()),
    async (r) => (r ? await backend().streamAsset(r) : "")
  );
  const src = () => blobFallback() || (external ? props.url : blob());
  const label = () =>
    decodeURIComponent((rel() || props.url).split("/").pop() || props.url);
  const open = (e: MouseEvent) => {
    e.stopPropagation();
    const r = rel();
    if (r && !external) void backend().openAsset(r);
    else void backend().openExternal(props.url);
  };
  let tryingBlobFallback = false;
  let ownedBlobUrl = "";
  onCleanup(() => {
    if (ownedBlobUrl) URL.revokeObjectURL(ownedBlobUrl);
  });
  const onMediaError = () => {
    const r = rel();
    // WebKitGTK's media pipeline rejects Tauri custom-scheme Matroska URLs even
    // when the same supported MKV bytes play from a Blob. Retry that one known
    // scheme/container mismatch with a graph-scoped, size-bounded read. Very
    // large files still fall back to the system player instead of consuming
    // unbounded WebView memory.
    if (!external && r?.toLowerCase().endsWith(".mkv") && !tryingBlobFallback && !blobFallback()) {
      tryingBlobFallback = true;
      void backend().readAsset(r, 512 * 1024 * 1024).then((bytes) => {
        ownedBlobUrl = URL.createObjectURL(new Blob([Uint8Array.from(bytes).buffer], { type: "video/x-matroska" }));
        setBlobFallback(ownedBlobUrl);
      }).catch(() => setFailed(true));
      return;
    }
    setFailed(true);
  };

  // Video drag-resize: identical mechanic to images (size the WRAPPER, persist a
  // width %). Audio uses the widen toggle instead, so no grip there.
  let wrapEl: HTMLSpanElement | undefined;
  let mediaEl: HTMLVideoElement | HTMLAudioElement | undefined;
  const onGripDown = (e: PointerEvent) => {
    if (!wrapEl || !props.blockId) return;
    e.preventDefault();
    e.stopPropagation();
    const refW = blockRefWidth(wrapEl);
    const startX = e.clientX;
    const startW = wrapEl.getBoundingClientRect().width;
    if (mediaEl) mediaEl.style.width = "100%";
    const move = (me: PointerEvent) => {
      const w = Math.max(80, Math.min(refW, startW + (me.clientX - startX)));
      if (wrapEl) wrapEl.style.width = `${w}px`;
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      const w = wrapEl ? wrapEl.getBoundingClientRect().width : startW;
      const pct = Math.max(5, Math.min(100, Math.round((w / refW) * 100)));
      writeMediaWidth(props.blockId!, props.alt ?? "", props.url, pct);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  // A persisted `{:width N%}` sizes the video wrapper (image fills it at 100%).
  const wrapStyle = () =>
    props.kind === "video" && props.width ? { width: props.width } : {};
  const mediaStyle = () =>
    props.kind === "video" && props.width ? { width: "100%" } : {};

  return (
    <Show
      when={!failed() && src()}
      fallback={
        <a class="media-chip" classList={{ audio: props.kind === "audio" }} onClick={open} title="Open in the default player">
          <span class="media-chip-icon">{props.kind === "audio" ? "♪" : "▶"}</span>
          {label()}
        </a>
      }
    >
      {/* An EXPLICIT "open in the default player" button is always available (shown
          on hover), not just the onError fallback: WebKit sometimes renders the
          <video> as playable but the codec is actually broken, so the user needs
          a guaranteed escape hatch to the OS player even when no media error fires. */}
      <span
        ref={wrapEl}
        class="media-embed-wrap"
        classList={{ "media-audio-wrap": props.kind === "audio" }}
        style={wrapStyle()}
        {...(props.spanAttrs ?? {})}
      >
        <Dynamic
          component={props.kind}
          ref={(el: HTMLVideoElement | HTMLAudioElement) => (mediaEl = el)}
          class="media-embed"
          classList={{ "media-audio": props.kind === "audio" }}
          controls={true}
          src={src()!}
          style={mediaStyle()}
          onError={onMediaError}
          onClick={(e: MouseEvent) => e.stopPropagation()}
        />
        <button class="media-open-external" onClick={open} title="Open in the default player (if playback here is broken)">
          <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true">
            <path d="M14 4h6v6M20 4l-8 8M18 13v6a1 1 0 01-1 1H5a1 1 0 01-1-1V7a1 1 0 011-1h6"
              fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
        <Show when={props.kind === "video" && props.blockId}>
          <span class="img-resize-grip media-resize-grip" title="Drag to resize" onPointerDown={onGripDown} />
        </Show>
        <Show when={props.kind === "audio"}>
          <button
            class="media-audio-widen"
            onClick={(e) => { e.stopPropagation(); setAudioPlayer({ url: props.url, name: label() }); }}
            title="Open the expanded player — waveform + precise seeking"
          >
            ⤢ Expand
          </button>
        </Show>
      </span>
    </Show>
  );
}

/** The inline run of a parsed block: the concatenated `inline` of its inline-flow
 *  blocks (paragraph/bullet/heading). `InlineText` callers pass a single line of
 *  inline markup, so this is normally one block's inlines. */
function blockInlines(blocks: AstBlock[]): Inline[] {
  const out: Inline[] = [];
  for (const b of blocks) {
    if (b.kind === "paragraph" || b.kind === "bullet" || b.kind === "heading") out.push(...b.inline);
  }
  return out;
}

/** Render a single line of inline markup (a property value, breadcrumb, ref-block
 *  preview line, …) — anything NOT a full block body. Parses via the in-browser
 *  wasm parser (src/render/parse.ts) and renders the inline run; `blockId` is
 *  threaded to inline `{{query}}` macros so they can rewrite the owning block. */
export function InlineText(props: { text: string; blockId?: string; format?: Format }): JSX.Element {
  // Only parse once the wasm parser is ready — `parseBlock` THROWS otherwise, and
  // unlike AstBody these callers (property values, breadcrumbs, ref previews, PDF
  // annotations) have no error boundary. When the parser isn't ready, OR when the
  // line is a block construct that yields no inline-flow content (`> quote`, `---`,
  // `| a | b |`, `[^1]: …`, `$$…$$`, …), fall back to the literal text so the
  // content is never dropped — matching the old inline-only renderer.
  const inlines = createMemo(() =>
    parserReady() ? blockInlines(parseBlock(props.text, props.format === "org")) : null,
  );
  return (
    <Show when={inlines() && inlines()!.length > 0} fallback={<EmojiText text={props.text} />}>
      {renderInlines(inlines()!, props.blockId, false)}
    </Show>
  );
}

// Recursion depth guard for user `:macros` expansion. renderSegs uses <For>, which
// evaluates synchronously on creation, so a macro that expands to itself (or a
// mutually-recursive pair) would render forever. We cap nesting and bail to a grey
// literal past the cap — cheap and bulletproof against both direct and mutual loops.
let userMacroDepth = 0;
const MAX_USER_MACRO_DEPTH = 12;

export function expandTemplate(template: string, args: string[]): string {
  return template.replace(/\$(\d+)/g, (m, d) => args[Number(d) - 1] ?? m);
}

function isSingleParagraphExpansionBlock(b: AstBlock): boolean {
  // parseBlock re-bullets raw Markdown/Org block bodies, so a plain paragraph
  // arrives as the synthetic header bullet. A heading expansion has `size`.
  return b.kind === "paragraph" || (b.kind === "bullet" && b.size == null);
}

export function expansionIsBlockLevel(expanded: string, fmt?: Format): boolean {
  const blocks = parseBlock(expanded, fmt === "org");
  return !(blocks.length === 1 && isSingleParagraphExpansionBlock(blocks[0]));
}

function expansionHeadingLevel(expanded: string, fmt?: Format): number | null {
  const first = parseBlock(expanded, fmt === "org")[0];
  if (!first) return null;
  if (first.kind === "heading") return first.size ?? first.level ?? null;
  if (first.kind === "bullet") return first.size ?? null;
  return null;
}

// `{{name a, b}}` where `name` is a user-defined `:macros` key: fill `$1..$N` with
// the args. Single-paragraph expansions stay inline; block-level expansions render
// through the block renderer, matching OG's macro parse/render split.
function UserMacroView(props: { name: string; template: string; args: string[]; blockId?: string }): JSX.Element {
  if (userMacroDepth >= MAX_USER_MACRO_DEPTH) {
    return <span class="macro">{`{{${props.name}}}`}</span>;
  }
  const expanded = expandTemplate(props.template, props.args);
  userMacroDepth++;
  try {
    const fmt = formatForBlock(props.blockId);
    if (parserReady() && expansionIsBlockLevel(expanded, fmt)) {
      return (
        <div class="macro-blocks">
          <AstBody raw={expanded} blockId={props.blockId} format={fmt} headingLevel={expansionHeadingLevel(expanded, fmt)} />
        </div>
      );
    }
    return <InlineText text={expanded} blockId={props.blockId} format={fmt} />;
  } finally {
    userMacroDepth--;
  }
}

// Inline block reference. Bare `((uuid))` shows the referenced block's first
// line; the labeled form `[label](((uuid)))` shows the label instead. Both
// navigate to the source page on click and show a hover preview of the full
// referenced block (mirrors OG); a missing target falls back to a short id.
function BlockRefView(props: { id: string; label?: string; spanAttrs?: SpanDomAttrs }): JSX.Element {
  const pane = useContext(PaneContext);
  const insidePeek = useContext(PeekContext);
  let anchorEl: HTMLSpanElement | undefined;
  const [grp] = createResource(
    () => `${props.id}\0${graphEpoch()}`, // resolve once per open graph; batched + cached
    () => resolveBlockBatched(props.id)
  );
  const peek = createPeekBridge(() => insidePeek);
  // Visible text: an explicit label wins; otherwise the target's first line.
  const text = () => props.label ?? (grp() ? visibleBody(grp()!.blocks[0].raw)[0] : undefined);
  // Parse the referenced block's text with ITS page's format (org refs render org).
  const fmt = () => formatForPage(grp()?.page);
  const annotation = () => {
    const block = grp()?.blocks[0];
    return block ? annotationInfoForBlock(block) : null;
  };
  const capped = createMemo(() => capBlockTree(grp()?.blocks ?? [], PEEK_BLOCK_CAP));
  return (
    <>
      <span
        ref={anchorEl}
        class="block-ref"
        classList={{ "block-ref-missing": !grp() }}
        {...(props.spanAttrs ?? {})}
        title={annotation()
          ? "Click to open the highlight in its PDF; shift-click → sidebar; right-click for more"
          : "Click to go to the block; shift-click → sidebar; right-click for more"}
        // Suppress native shift-range-selection when shift+click opens the sidebar (GH #42).
        onMouseDown={(e) => { if (e.shiftKey) e.preventDefault(); }}
        onMouseEnter={peek.anchorEnter}
        onMouseLeave={peek.anchorLeave}
        onContextMenu={(e) => {
          const g = grp();
          if (!g) return; // missing target → let the default menu through
          e.preventDefault();
          e.stopPropagation();
          openBlockRefContextMenu(e.clientX, e.clientY, props.id, g.page, g.kind);
        }}
        onClick={(e) => {
          e.stopPropagation();
          const g = grp();
          if (!g) return;
          const ann = annotation();
          // OG opens a referenced PDF annotation at its source page. Modifier
          // clicks retain Tine's existing pane/sidebar navigation semantics.
          if (ann && !e.ctrlKey && !e.metaKey && !e.shiftKey) {
            void backend()
              .getPage(g.page, g.kind)
              .then((page) => {
                const file = pdfFileFromPreBlock(page?.pre_block);
                if (file) openPdf(file, file, ann.hlPage);
                else pushToast("Couldn't find the PDF for this highlight", "error");
              })
              .catch(() => pushToast("Couldn't open the PDF for this highlight", "error"));
            return;
          }
          // Shift-click opens the referenced block in the right sidebar. Plain click:
          // Tine scrolls + flashes the block in context (default); the OG behavior —
          // zoom into the block as its own page — is opt-in (Settings → ref-click-zoom).
          if (e.ctrlKey || e.metaKey)
            openRouteInOtherPane(
              { kind: "page", name: g.page, pageKind: g.kind, block: props.id },
              pane?.paneId ?? focusedPaneId()
            );
          else if (e.shiftKey) openBlockInSidebar({ uuid: props.id, page: g.page, pageKind: g.kind });
          else if (refClickZoom()) focusBlock(props.id);
          else openPageAtBlock(g.page, g.kind, props.id);
        }}
      >
        <Show when={text() !== undefined} fallback={<>(({props.id.slice(0, 8)}))</>}>
          <InlineText text={text()!} format={fmt()} />
        </Show>
      </span>
      <Show when={peek.open() && grp() && capped().blocks.length > 0}>
        <PeekPopup
          anchor={() => anchorEl}
          title={<span class="peek-popup-title-name">{grp()!.page}</span>}
          blocks={() => capped().blocks}
          page={grp()!.page}
          pageKind={grp()!.kind}
          truncatedCount={() => capped().truncated}
          onPointerEnter={peek.popupEnter}
          onPointerLeave={peek.popupLeave}
        />
      </Show>
    </>
  );
}
