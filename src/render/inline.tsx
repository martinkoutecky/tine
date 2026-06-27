// Inline markdown -> Solid components. Produces real interactive DOM (clickable
// [[links]] and #tags), not an innerHTML string. Used to render a block when it
// is not being edited.

import { For, Show, createMemo, createResource, createSignal, type JSX } from "solid-js";
import { Dynamic } from "solid-js/web";
import { mediaKind } from "../media";
import { openPage, openPageInNewTab } from "../router";
import { isJournalTitle } from "../journal";
import { openPdf, openPageInSidebar, openPageContextMenu, setLightbox, graphEpoch } from "../ui";
import { parseInline, type Seg, type Format } from "./parseInline";
import { blockView } from "./block";
import { backend } from "../backend";
import { loadAssetBlob } from "../assetCache";
import { resolveBlockBatched } from "../resolveBatch";
import { doc, setRaw } from "../store";
import { QueryMacro, EmbedMacro, VideoMacro, TweetMacro } from "../components/Macro";
import { NamespaceMacro } from "../components/Namespace";

function renderSegs(segs: Seg[], blockId?: string): JSX.Element {
  return <For each={segs}>{(s) => renderSeg(s, blockId)}</For>;
}

function renderSeg(s: Seg, blockId?: string): JSX.Element {
  switch (s.t) {
    case "text":
      return <>{s.v}</>;
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
    case "macro": {
      // Render {{query}} / {{embed}} wherever they appear — including inline
      // after a label, e.g. `All todos {{query (task TODO)}}` (Logseq dashboards).
      const body = s.body.trimStart();
      if (/^query\b/i.test(body)) return <QueryMacro body={body} blockId={blockId} />;
      if (/^embed\b/i.test(body)) return <EmbedMacro body={body} />;
      if (/^(video|youtube)\b/i.test(body)) return <VideoMacro body={body} />;
      if (/^tweet\b/i.test(body)) return <TweetMacro body={body} />;
      if (/^namespace\b/i.test(body)) {
        const root = body.replace(/^namespace\s+/i, "").trim();
        if (root) return <NamespaceMacro root={root} />;
      }
      return <span class="macro">{`{{${s.body}}}`}</span>;
    }
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
      if (k === "video" || k === "audio") return <MediaEmbed url={s.url} kind={k} />;
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
function MathView(props: { tex: string; display: boolean }): JSX.Element {
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

// Persist a resized image width back into its block's raw text as the OG-native
// `{:width "N%"}` brace. We rewrite THIS image token's trailing `{...}` only
// (matched by its exact alt+url), so other text/images in the block are
// untouched; width is stored as a percentage (Martin's choice — survives column
// width changes) and as a quoted string so it stays valid EDN OG can also read.
function writeImageWidth(blockId: string, alt: string, url: string, pct: number) {
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
      writeImageWidth(props.blockId!, props.alt, props.url, pct);
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

// Video/audio asset: try inline playback (a streaming-ish blob-URL <video>/<audio
// controls>), and on a media error — typically WebKitGTK lacking the mp4/h264
// codec — fall back to a click-to-open chip that launches the OS default player.
// Matches the user's "inline when it works, otherwise open externally" intent.
function MediaEmbed(props: { url: string; kind: "video" | "audio" }): JSX.Element {
  const [failed, setFailed] = createSignal(false);
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
      <span class="media-embed-wrap" classList={{ "media-audio-wrap": props.kind === "audio" }}>
        <Dynamic
          component={props.kind}
          class="media-embed"
          classList={{ "media-audio": props.kind === "audio" }}
          controls={true}
          src={src()!}
          onError={() => setFailed(true)}
          onClick={(e: MouseEvent) => e.stopPropagation()}
        />
        <button class="media-open-external" onClick={open} title="Open in the default player (if playback here is broken)">
          <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true">
            <path d="M14 4h6v6M20 4l-8 8M18 13v6a1 1 0 01-1 1H5a1 1 0 01-1-1V7a1 1 0 011-1h6"
              fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </button>
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
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      onClick={(e) => {
        e.stopPropagation();
        const g = grp();
        if (g) openPage(g.page, g.kind);
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
