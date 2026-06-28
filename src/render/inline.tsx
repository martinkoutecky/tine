// Inline markdown -> Solid components. Produces real interactive DOM (clickable
// [[links]] and #tags), not an innerHTML string. Used to render a block when it
// is not being edited.

import { For, Show, createMemo, createResource, createSignal, type JSX } from "solid-js";
import { Dynamic } from "solid-js/web";
import { mediaKind } from "../media";
import { openPage, openPageInNewTab, openPageAtBlock, focusBlock } from "../router";
import { refClickZoom } from "../copySettings";
import { isJournalTitle } from "../journal";
import { openPdf, openPageInSidebar, openBlockInSidebar, openPageContextMenu, openBlockRefContextMenu, setLightbox, setAudioPlayer, graphEpoch, graphMeta } from "../ui";
import { parseInline, type Seg, type Format } from "./parseInline";
import type { Inline, Url, MacroInline, TimestampInline, TimestampPoint, EmailValue } from "./ast";
import { EmojiText } from "./emoji";
import { blockView } from "./block";
import { backend } from "../backend";
import { loadAssetBlob } from "../assetCache";
import { resolveBlockBatched } from "../resolveBatch";
import { doc, setRaw } from "../store";
import { QueryMacro, EmbedMacro, VideoMacro, TweetMacro, YoutubeTimestamp, ClozeMacro, ZoteroMacro } from "../components/Macro";
import { NamespaceMacro } from "../components/Namespace";

function renderSegs(segs: Seg[], blockId?: string): JSX.Element {
  return <For each={segs}>{(s) => renderSeg(s, blockId)}</For>;
}

function renderSeg(s: Seg, blockId?: string): JSX.Element {
  switch (s.t) {
    case "text":
      return <EmojiText text={s.v} />;
    case "bold":
      return <strong>{renderSegs(s.v)}</strong>;
    case "italic":
      return <em>{renderSegs(s.v)}</em>;
    case "underline":
      return <u>{renderSegs(s.v)}</u>;
    case "strike":
      return <del>{renderSegs(s.v)}</del>;
    case "highlight":
      return <mark>{renderSegs(s.v)}</mark>;
    case "code":
      return <code class="inline-code">{s.v}</code>;
    case "pageref":
      return (
        <a
          class="page-ref"
          onClick={(e) => {
            e.stopPropagation();
            // A journal-date name (in the graph's title format) routes to the
            // journal page; otherwise it's a regular page. Without this a
            // `[[Fri, 26-06-2026]]` link/tag opens as an empty page.
            const kind = isJournalTitle(s.name) ? "journal" : "page";
            if (e.shiftKey) openPageInSidebar(s.name, kind);
            else openPage(s.name, kind);
          }}
          onAuxClick={(e) => {
            if (e.button === 1) {
              e.preventDefault();
              e.stopPropagation();
              openPageInNewTab(s.name, isJournalTitle(s.name) ? "journal" : "page");
            }
          }}
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            openPageContextMenu(e.clientX, e.clientY, s.name);
          }}
        >
          <Show when={s.alias} fallback={<><span class="bracket">[[</span>{s.name}<span class="bracket">]]</span></>}>
            {s.alias}
          </Show>
        </a>
      );
    case "tag":
      return (
        <a
          class="tag"
          onClick={(e) => {
            e.stopPropagation();
            // A journal-date name (in the graph's title format) routes to the
            // journal page; otherwise it's a regular page. Without this a
            // `[[Fri, 26-06-2026]]` link/tag opens as an empty page.
            const kind = isJournalTitle(s.name) ? "journal" : "page";
            if (e.shiftKey) openPageInSidebar(s.name, kind);
            else openPage(s.name, kind);
          }}
          onAuxClick={(e) => {
            if (e.button === 1) {
              e.preventDefault();
              e.stopPropagation();
              openPageInNewTab(s.name, isJournalTitle(s.name) ? "journal" : "page");
            }
          }}
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            openPageContextMenu(e.clientX, e.clientY, s.name);
          }}
        >
          #{s.name}
        </a>
      );
    case "blockref":
      return <BlockRefView id={s.id} label={s.label} />;
    case "macro":
      return renderMacroBody(s.body, blockId);
    case "math":
      return <MathView tex={s.tex} display={s.display} />;
    case "link": {
      // PDF assets open in the side viewer instead of navigating away.
      if (/\.pdf$/i.test(s.url)) {
        const filename = s.url.split("/").pop() ?? s.url;
        return (
          <a
            class="external-link pdf-link"
            onClick={(e) => {
              e.stopPropagation();
              openPdf(filename, s.label || filename);
            }}
          >
            📄 {s.label || filename}
          </a>
        );
      }
      return (
        <a
          class="external-link"
          href={s.url}
          onClick={(e) => {
            // Open externally; don't navigate the webview or fall through to the
            // row's click-to-edit.
            e.preventDefault();
            e.stopPropagation();
            void backend().openExternal(s.url);
          }}
        >
          {s.label || s.url}
        </a>
      );
    }
    case "image": {
      // `![](…)` is reused for video/audio (like OG) — route those to a player.
      const k = mediaKind(s.url);
      if (k === "video" || k === "audio")
        return <MediaEmbed url={s.url} kind={k} alt={s.alt} width={s.width} blockId={blockId} />;
      return (
        <AssetImage url={s.url} alt={s.alt} width={s.width} height={s.height} blockId={blockId} />
      );
    }
    case "footnote":
      return <sup class="footnote-ref">{s.id}</sup>;
    case "timestamp":
      // Org date/time stamp: active `<…>` vs inactive `[…]` (styled, like OG;
      // inline Date stamps aren't journal links in OG, so neither are these).
      return (
        <span class="org-timestamp" classList={{ inactive: !s.active }}>
          {s.active ? "<" : "["}
          {s.raw}
          {s.active ? ">" : "]"}
        </span>
      );
    case "iframe":
      return (
        <span class="embed-iframe-wrap" style={{ ...(s.width ? { width: s.width } : {}), ...(s.height ? { "aspect-ratio": "auto", height: s.height } : {}) }}>
          <iframe
            class="embed-iframe"
            src={s.src}
            sandbox="allow-scripts allow-same-origin allow-popups allow-forms"
            referrerpolicy="no-referrer"
            title="embed"
          />
        </span>
      );
  }
}

// ===========================================================================
// AST renderer (lsdoc). Renders an `Inline[]` produced by the Rust parser to
// interactive DOM — the replacement for the parseInline → renderSeg path.
// Reuses every component above (EmojiText, MathView, BlockRefView, AssetImage,
// MediaEmbed, the macros). See subagent-tasks/notes/ast-render-contract.md.
// ===========================================================================

// Shared `{{macro}}` dispatch, keyed off a reconstructed body string so the
// legacy Seg path and the AST path render macros identically.
function renderMacroBody(raw: string, blockId?: string): JSX.Element {
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
    const args = um[2].trim() ? um[2].split(",").map((a) => a.trim()) : [];
    return <UserMacroView name={um[1]} template={userMacros[um[1]]} args={args} blockId={blockId} />;
  }
  return <span class="macro">{`{{${raw}}}`}</span>;
}

/** Render a parsed inline run (lsdoc `Inline[]`) to interactive DOM. */
export function renderInlines(inlines: Inline[], blockId?: string): JSX.Element {
  return <For each={inlines}>{(s) => renderInline(s, blockId)}</For>;
}

function renderInline(s: Inline, blockId?: string): JSX.Element {
  switch (s.k) {
    case "plain":
      return <EmojiText text={s.text} />;
    case "code":
    case "verbatim":
      return <code class="inline-code">{s.text}</code>;
    case "break":
    case "hardbreak":
      // Both render as <br>: today body.tsx joins every in-block line with <br>,
      // and a soft `break` is exactly such an in-block newline — match that look.
      return <br />;
    case "emphasis": {
      const inner = renderInlines(s.children, blockId);
      switch (s.emph) {
        case "Bold": return <strong>{inner}</strong>;
        case "Italic": return <em>{inner}</em>;
        case "Strike_through": return <del>{inner}</del>;
        case "Highlight": return <mark>{inner}</mark>;
        case "Underline": return <u>{inner}</u>;
      }
      return inner;
    }
    case "subscript":
      return <sub>{renderInlines(s.children, blockId)}</sub>;
    case "superscript":
      return <sup>{renderInlines(s.children, blockId)}</sup>;
    case "link":
      return renderLink(s, blockId);
    case "nested_link":
      // Logseq `[[a [[b]] c]]` — best-effort: route the whole inner as a page ref.
      return <PageRef name={s.content} />;
    case "target":
      return <span class="org-target">{s.text}</span>;
    case "tag":
      return <PageRef name={astText(s.children)} tag />;
    case "macro":
      return renderMacroBody(macroBody(s), blockId);
    case "latex":
      return <MathView tex={s.body} display={s.mode === "Displayed"} />;
    case "timestamp":
      return renderTimestamp(s);
    case "fnref":
      return <sup class="footnote-ref">{s.name}</sup>;
    case "inline_html":
      return renderRawHtml(s.text);
    case "email":
      return renderEmail(s.text);
    case "entity":
      return <>{s.unicode}</>;
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
      return url.v;
    case "complex":
      return url.protocol && url.link != null ? `${url.protocol}://${url.link}` : url.link ?? "";
  }
}

function macroBody(s: MacroInline): string {
  return s.args.length ? `${s.name} ${s.args.join(", ")}` : s.name;
}

// A `[[page]]` / `#tag` anchor — shared by page_ref links, bare refs, and #tags.
function PageRef(props: { name: string; alias?: JSX.Element; tag?: boolean }): JSX.Element {
  const open = (e: MouseEvent) => {
    e.stopPropagation();
    const kind = isJournalTitle(props.name) ? "journal" : "page";
    if (e.shiftKey) openPageInSidebar(props.name, kind);
    else openPage(props.name, kind);
  };
  return (
    <a
      class={props.tag ? "tag" : "page-ref"}
      onClick={open}
      onAuxClick={(e) => {
        if (e.button === 1) {
          e.preventDefault();
          e.stopPropagation();
          openPageInNewTab(props.name, isJournalTitle(props.name) ? "journal" : "page");
        }
      }}
      onContextMenu={(e) => {
        e.preventDefault();
        e.stopPropagation();
        openPageContextMenu(e.clientX, e.clientY, props.name);
      }}
    >
      <Show when={props.tag} fallback={
        <Show when={props.alias} fallback={<><span class="bracket">[[</span>{props.name}<span class="bracket">]]</span></>}>
          {props.alias}
        </Show>
      }>
        #{props.name}
      </Show>
    </a>
  );
}

function renderLink(s: Extract<Inline, { k: "link" }>, blockId?: string): JSX.Element {
  const url = s.url;
  if (url.type === "page_ref") {
    const alias = s.label && s.label.length ? renderInlines(s.label, blockId) : undefined;
    return <PageRef name={url.v} alias={alias} />;
  }
  if (url.type === "block_ref") {
    const label = s.label && s.label.length ? astText(s.label) : undefined;
    return <BlockRefView id={url.v} label={label} />;
  }
  const dest = urlDest(url);
  if (s.image) {
    const { width, height } = parseImageMetaBrace(s.metadata);
    const alt = s.label && s.label.length ? astText(s.label) : "";
    const k = mediaKind(dest);
    if (k === "video" || k === "audio")
      return <MediaEmbed url={dest} kind={k} alt={alt} width={width} blockId={blockId} />;
    return <AssetImage url={dest} alt={alt} width={width} height={height} blockId={blockId} />;
  }
  if (/\.pdf$/i.test(dest)) {
    const filename = dest.split("/").pop() ?? dest;
    const labelStr = s.label && s.label.length ? astText(s.label) : filename;
    return (
      <a class="external-link pdf-link" onClick={(e) => { e.stopPropagation(); openPdf(filename, labelStr || filename); }}>
        📄 {labelStr || filename}
      </a>
    );
  }
  return (
    <a
      class="external-link"
      href={dest}
      onClick={(e) => { e.preventDefault(); e.stopPropagation(); void backend().openExternal(dest); }}
    >
      <Show when={s.label && s.label.length} fallback={dest}>{renderInlines(s.label!, blockId)}</Show>
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

// Org timestamp inline → the styled `<…>`(active)/`[…]`(inactive) badge. The AST
// carries only the structured date, so format the display string ourselves.
function pad2(n: number): string { return String(n).padStart(2, "0"); }
function fmtTsPoint(p: TimestampPoint): string {
  const d = p.date;
  let s = `${d.year}-${pad2(d.month)}-${pad2(d.day)}`;
  if (p.wday) s += ` ${p.wday}`;
  if (p.time) s += ` ${pad2(p.time.hour)}:${pad2(p.time.min)}`;
  return s;
}
function renderTimestamp(s: TimestampInline): JSX.Element {
  const v = s.date as Record<string, unknown>;
  let active = true;
  let text: string;
  if (s.ts === "Range" && v.start && v.stop) {
    const start = v.start as TimestampPoint;
    active = start.active ?? true;
    text = `${fmtTsPoint(start)}--${fmtTsPoint(v.stop as TimestampPoint)}`;
  } else {
    const p = v as unknown as TimestampPoint;
    active = p.active ?? true;
    text = fmtTsPoint(p);
  }
  return (
    <span class="org-timestamp" classList={{ inactive: !active }}>
      {active ? "<" : "["}{text}{active ? ">" : "]"}
    </span>
  );
}

// Sandboxed-iframe rendering, reused for inline `inline_html` and block `raw_html`.
function renderIframe(src: string, width?: string, height?: string): JSX.Element {
  return (
    <span class="embed-iframe-wrap" style={{ ...(width ? { width } : {}), ...(height ? { "aspect-ratio": "auto", height } : {}) }}>
      <iframe class="embed-iframe" src={src} sandbox="allow-scripts allow-same-origin allow-popups allow-forms" referrerpolicy="no-referrer" title="embed" />
    </span>
  );
}
// Raw HTML (inline_html / raw_html): honour ONLY the https `<iframe>` subset
// (sandboxed); any OTHER raw HTML renders as PLAIN TEXT (no innerHTML — XSS-safe,
// matches today's behavior).
export function renderRawHtml(text: string): JSX.Element {
  const m = /<iframe\b([^>]*)>/i.exec(text);
  if (m) {
    const attrs = m[1];
    const src = /src\s*=\s*["']([^"']+)["']/i.exec(attrs)?.[1];
    if (src && /^https?:\/\//i.test(src)) {
      const width = /width\s*=\s*["']?(\d+%?|\d+px)["']?/i.exec(attrs)?.[1];
      const height = /height\s*=\s*["']?(\d+%?|\d+px)["']?/i.exec(attrs)?.[1];
      return renderIframe(src, width, height);
    }
  }
  return <EmojiText text={text} />;
}

function renderEmail(text: EmailValue): JSX.Element {
  let addr = "";
  if (typeof text === "string") addr = text;
  else if (text && typeof text === "object") {
    const lp = (text as Record<string, unknown>).local_part;
    const dom = (text as Record<string, unknown>).domain;
    if (typeof lp === "string" && typeof dom === "string") addr = `${lp}@${dom}`;
  }
  const href = `mailto:${addr}`;
  return (
    <a class="external-link" href={href} onClick={(e) => { e.preventDefault(); e.stopPropagation(); void backend().openExternal(href); }}>
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
export function MathView(props: { tex: string; display: boolean }): JSX.Element {
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
    <span class="math" classList={{ "math-display": props.display }}>
      <Show when={html()} fallback={<span class="math-raw">{props.tex}</span>}>
        <span innerHTML={html()!} />
      </Show>
    </span>
  );
}

// Resolve the path of a graph asset relative to the `assets/` dir.
function assetRelPath(url: string): string | null {
  const i = url.indexOf("assets/");
  return i === -1 ? null : url.slice(i + "assets/".length);
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
  // Served from a shared, graph-scoped blob cache so repeated references and
  // re-mounts don't re-read the file or mint duplicate blob URLs. The cache owns
  // the URL's lifetime (cleared on graph switch), so we don't revoke on unmount.
  const [diskSrc] = createResource(
    () => (external ? null : props.url),
    async (url) => {
      const rel = assetRelPath(url);
      return rel ? await loadAssetBlob(rel) : "";
    }
  );
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

  return (
    <Show
      when={external || src()}
      fallback={<span class="inline-image-missing">🖼 {props.alt || assetRelPath(props.url)}</span>}
    >
      <span ref={wrapEl} class="inline-image-wrap" style={wrapStyle()}>
        <img
          ref={imgEl}
          class="inline-image"
          src={src()!}
          alt={props.alt}
          style={imgStyle()}
          onClick={(e) => { e.stopPropagation(); setLightbox(src()!); }}
        />
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

// Video/audio asset: try inline playback (a streaming-ish blob-URL <video>/<audio
// controls>), and on a media error — typically WebKitGTK lacking the mp4/h264
// codec — fall back to a click-to-open chip that launches the OS default player.
// Matches the user's "inline when it works, otherwise open externally" intent.
function MediaEmbed(props: {
  url: string;
  kind: "video" | "audio";
  alt?: string;
  width?: string;
  blockId?: string;
}): JSX.Element {
  const [failed, setFailed] = createSignal(false);
  // Audio has no fullscreen; the "Expand" button (below) opens a wide overlay
  // player with a waveform scrubber instead of stretching the inline control.
  const external = /^(https?:|data:|blob:)/.test(props.url);
  const rel = () => assetRelPath(props.url);
  // Graph assets load over IPC into a blob URL (lazy); external URLs play directly.
  const [blob] = createResource(
    () => (external ? null : rel()),
    async (r) => (r ? await loadAssetBlob(r) : "")
  );
  const src = () => (external ? props.url : blob());
  const label = () =>
    decodeURIComponent((rel() || props.url).split("/").pop() || props.url);
  const open = (e: MouseEvent) => {
    e.stopPropagation();
    const r = rel();
    if (r && !external) void backend().openAsset(r);
    else void backend().openExternal(props.url);
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
      >
        <Dynamic
          component={props.kind}
          ref={(el: HTMLVideoElement | HTMLAudioElement) => (mediaEl = el)}
          class="media-embed"
          classList={{ "media-audio": props.kind === "audio" }}
          controls={true}
          src={src()!}
          style={mediaStyle()}
          onError={() => setFailed(true)}
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

/** Render a block's body text (already stripped of marker/heading prefix).
 *  `blockId` (the owning block) is threaded to inline `{{query}}` macros so they
 *  can show the editable builder + rewrite that block. */
export function InlineText(props: { text: string; blockId?: string; format?: Format }): JSX.Element {
  return <>{renderSegs(parseInline(props.text, props.format ?? "md"), props.blockId)}</>;
}

// Recursion depth guard for user `:macros` expansion. renderSegs uses <For>, which
// evaluates synchronously on creation, so a macro that expands to itself (or a
// mutually-recursive pair) would render forever. We cap nesting and bail to a grey
// literal past the cap — cheap and bulletproof against both direct and mutual loops.
let userMacroDepth = 0;
const MAX_USER_MACRO_DEPTH = 12;

// `{{name a, b}}` where `name` is a user-defined `:macros` key: fill `$1..$N` with
// the args, render the result as inline markdown. Block-level expansions degrade to
// inline (we only have an inline renderer here) — fine for the common link/format
// macros; a documented limitation for multi-line templates.
function UserMacroView(props: { name: string; template: string; args: string[]; blockId?: string }): JSX.Element {
  if (userMacroDepth >= MAX_USER_MACRO_DEPTH) {
    return <span class="macro">{`{{${props.name}}}`}</span>;
  }
  const expanded = props.template.replace(/\$(\d+)/g, (_, d) => props.args[Number(d) - 1] ?? "");
  userMacroDepth++;
  try {
    return <InlineText text={expanded} blockId={props.blockId} />;
  } finally {
    userMacroDepth--;
  }
}

// Inline block reference. Bare `((uuid))` shows the referenced block's first
// line; the labeled form `[label](((uuid)))` shows the label instead. Both
// navigate to the source page on click and show a hover preview of the full
// referenced block (mirrors OG); a missing target falls back to a short id.
function BlockRefView(props: { id: string; label?: string }): JSX.Element {
  const [grp] = createResource(
    () => `${props.id} ${graphEpoch()}`, // resolve once per open graph; batched + cached
    () => resolveBlockBatched(props.id)
  );
  const [hover, setHover] = createSignal(false);
  // Visible text: an explicit label wins; otherwise the target's first line.
  const text = () => props.label ?? (grp() ? blockView(grp()!.blocks[0].raw).lines[0] : undefined);
  return (
    <span
      class="block-ref"
      classList={{ "block-ref-missing": !grp() }}
      title="Click to go to the block; shift-click → sidebar; right-click for more"
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
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
        // Shift-click opens the referenced block in the right sidebar. Plain click:
        // Tine scrolls + flashes the block in context (default); the OG behavior —
        // zoom into the block as its own page — is opt-in (Settings → ref-click-zoom).
        if (e.shiftKey) openBlockInSidebar({ uuid: props.id, page: g.page, pageKind: g.kind });
        else if (refClickZoom()) focusBlock(props.id);
        else openPageAtBlock(g.page, g.kind, props.id);
      }}
    >
      <Show when={text() !== undefined} fallback={<>(({props.id.slice(0, 8)}))</>}>
        <InlineText text={text()!} />
      </Show>
      <Show when={hover() && grp()}>
        <span class="block-ref-preview">
          <span class="block-ref-preview-page">{grp()!.page}</span>
          <For each={grp()!.blocks}>
            {(b) => (
              <span class="block-ref-preview-line">
                <InlineText text={blockView(b.raw).lines.join(" ")} />
              </span>
            )}
          </For>
        </span>
      </Show>
    </span>
  );
}
