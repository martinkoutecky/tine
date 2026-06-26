// Inline markdown -> Solid components. Produces real interactive DOM (clickable
// [[links]] and #tags), not an innerHTML string. Used to render a block when it
// is not being edited.

import { For, Show, createMemo, createResource, createSignal, type JSX } from "solid-js";
import { Dynamic } from "solid-js/web";
import { mediaKind } from "../media";
import { openPage, openPageInNewTab } from "../router";
import { openPdf, openPageInSidebar, openPageContextMenu, setLightbox, graphEpoch } from "../ui";
import { parseInline, type Seg, type Format } from "./parseInline";
import { blockView } from "./block";
import { backend } from "../backend";
import { loadAssetBlob } from "../assetCache";
import { resolveBlockBatched } from "../resolveBatch";
import { QueryMacro, EmbedMacro, VideoMacro, TweetMacro } from "../components/Macro";

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
            if (e.shiftKey) openPageInSidebar(s.name);
            else openPage(s.name);
          }}
          onAuxClick={(e) => {
            if (e.button === 1) {
              e.preventDefault();
              e.stopPropagation();
              openPageInNewTab(s.name);
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
            if (e.shiftKey) openPageInSidebar(s.name);
            else openPage(s.name);
          }}
          onAuxClick={(e) => {
            if (e.button === 1) {
              e.preventDefault();
              e.stopPropagation();
              openPageInNewTab(s.name);
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
      return <BlockRefView id={s.id} />;
    case "macro": {
      // Render {{query}} / {{embed}} wherever they appear — including inline
      // after a label, e.g. `All todos {{query (task TODO)}}` (Logseq dashboards).
      const body = s.body.trimStart();
      if (/^query\b/i.test(body)) return <QueryMacro body={body} blockId={blockId} />;
      if (/^embed\b/i.test(body)) return <EmbedMacro body={body} />;
      if (/^(video|youtube)\b/i.test(body)) return <VideoMacro body={body} />;
      if (/^tweet\b/i.test(body)) return <TweetMacro body={body} />;
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
      return <AssetImage url={s.url} alt={s.alt} width={s.width} height={s.height} />;
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

// Image embed: external URLs load directly; graph assets (`../assets/x.png`)
// are read from disk via the backend and shown as a blob URL.
function AssetImage(props: { url: string; alt: string; width?: string; height?: string }): JSX.Element {
  const dim = () => ({
    ...(props.width ? { width: props.width } : {}),
    ...(props.height ? { height: props.height } : {}),
  });
  if (/^(https?:|data:|blob:)/.test(props.url)) {
    return (
      <img
        class="inline-image"
        src={props.url}
        alt={props.alt}
        style={dim()}
        onClick={(e) => { e.stopPropagation(); setLightbox(props.url); }}
      />
    );
  }
  // Served from a shared, graph-scoped blob cache so repeated references and
  // re-mounts don't re-read the file or mint duplicate blob URLs. The cache owns
  // the URL's lifetime (cleared on graph switch), so we don't revoke on unmount.
  const [src] = createResource(
    () => props.url,
    async (url) => {
      const rel = assetRelPath(url);
      if (!rel) return "";
      return loadAssetBlob(rel);
    }
  );
  return (
    <Show
      when={src()}
      fallback={<span class="inline-image-missing">🖼 {props.alt || assetRelPath(props.url)}</span>}
    >
      <img
        class="inline-image"
        src={src()!}
        alt={props.alt}
        style={dim()}
        onClick={(e) => { e.stopPropagation(); setLightbox(src()!); }}
      />
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
      <Dynamic
        component={props.kind}
        class="media-embed"
        classList={{ "media-audio": props.kind === "audio" }}
        controls={true}
        src={src()!}
        onError={() => setFailed(true)}
        onClick={(e: MouseEvent) => e.stopPropagation()}
      />
    </Show>
  );
}

/** Render a block's body text (already stripped of marker/heading prefix).
 *  `blockId` (the owning block) is threaded to inline `{{query}}` macros so they
 *  can show the editable builder + rewrite that block. */
export function InlineText(props: { text: string; blockId?: string; format?: Format }): JSX.Element {
  return <>{renderSegs(parseInline(props.text, props.format ?? "md"), props.blockId)}</>;
}

// Inline ((block reference)): resolves to the referenced block's first line,
// navigates to its source page on click, and shows a hover preview of the full
// referenced block (mirrors OG's block-ref tooltip).
function BlockRefView(props: { id: string }): JSX.Element {
  const [grp] = createResource(
    () => `${props.id} ${graphEpoch()}`, // resolve once per open graph; batched + cached
    () => resolveBlockBatched(props.id)
  );
  const [hover, setHover] = createSignal(false);
  return (
    <span
      class="block-ref"
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      onClick={(e) => {
        e.stopPropagation();
        const g = grp();
        if (g) openPage(g.page, g.kind);
      }}
    >
      <Show when={grp()} fallback={<>(({props.id.slice(0, 8)}))</>}>
        <InlineText text={blockView(grp()!.blocks[0].raw).lines[0]} />
        <Show when={hover()}>
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
      </Show>
    </span>
  );
}
