// Inline markdown -> Solid components. Produces real interactive DOM (clickable
// [[links]] and #tags), not an innerHTML string. Used to render a block when it
// is not being edited.

import { For, Show, createResource, createSignal, onCleanup, type JSX } from "solid-js";
import katex from "katex";
// mhchem extends KaTeX with \ce{…} chemistry support (registers globally on import).
import "katex/contrib/mhchem";
import { openPage, openPageInNewTab } from "../router";
import { openPdf, openPageInSidebar, openPageContextMenu } from "../ui";
import { parseInline, type Seg } from "./parseInline";
import { blockView } from "./block";
import { backend } from "../backend";
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
          <span class="bracket">[[</span>
          {s.name}
          <span class="bracket">]]</span>
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
        <a class="external-link" href={s.url} target="_blank" rel="noreferrer">
          {s.label || s.url}
        </a>
      );
    }
    case "image":
      return <AssetImage url={s.url} alt={s.alt} width={s.width} height={s.height} />;
    case "footnote":
      return <sup class="footnote-ref">{s.id}</sup>;
  }
}

// KaTeX-typeset math. KaTeX output is trusted HTML, so innerHTML is safe here.
function MathView(props: { tex: string; display: boolean }): JSX.Element {
  const html = () => {
    try {
      return katex.renderToString(props.tex, {
        throwOnError: false,
        displayMode: props.display,
      });
    } catch {
      return props.tex;
    }
  };
  return <span class="math" classList={{ "math-display": props.display }} innerHTML={html()} />;
}

// Resolve the path of a graph asset relative to the `assets/` dir.
function assetRelPath(url: string): string | null {
  const i = url.indexOf("assets/");
  return i === -1 ? null : url.slice(i + "assets/".length);
}

function mimeFromExt(path: string): string {
  const ext = path.split(".").pop()?.toLowerCase();
  switch (ext) {
    case "png":
      return "image/png";
    case "jpg":
    case "jpeg":
      return "image/jpeg";
    case "gif":
      return "image/gif";
    case "svg":
      return "image/svg+xml";
    case "webp":
      return "image/webp";
    default:
      return "application/octet-stream";
  }
}

// Image embed: external URLs load directly; graph assets (`../assets/x.png`)
// are read from disk via the backend and shown as a blob URL.
function AssetImage(props: { url: string; alt: string; width?: string; height?: string }): JSX.Element {
  const dim = () => ({
    ...(props.width ? { width: props.width } : {}),
    ...(props.height ? { height: props.height } : {}),
  });
  if (/^(https?:|data:|blob:)/.test(props.url)) {
    return <img class="inline-image" src={props.url} alt={props.alt} style={dim()} />;
  }
  let objectUrl = "";
  const [src] = createResource(
    () => props.url,
    async (url) => {
      const rel = assetRelPath(url);
      if (!rel) return "";
      try {
        const bytes = await backend().readAsset(rel);
        if (!bytes.length) return "";
        objectUrl = URL.createObjectURL(
          new Blob([bytes as unknown as BlobPart], { type: mimeFromExt(rel) })
        );
        return objectUrl;
      } catch {
        return "";
      }
    }
  );
  onCleanup(() => objectUrl && URL.revokeObjectURL(objectUrl));
  return (
    <Show
      when={src()}
      fallback={<span class="inline-image-missing">🖼 {props.alt || assetRelPath(props.url)}</span>}
    >
      <img class="inline-image" src={src()!} alt={props.alt} style={dim()} />
    </Show>
  );
}

/** Render a block's body text (already stripped of marker/heading prefix).
 *  `blockId` (the owning block) is threaded to inline `{{query}}` macros so they
 *  can show the editable builder + rewrite that block. */
export function InlineText(props: { text: string; blockId?: string }): JSX.Element {
  return <>{renderSegs(parseInline(props.text), props.blockId)}</>;
}

// Inline ((block reference)): resolves to the referenced block's first line,
// navigates to its source page on click, and shows a hover preview of the full
// referenced block (mirrors OG's block-ref tooltip).
function BlockRefView(props: { id: string }): JSX.Element {
  const [grp] = createResource(
    () => props.id,
    (id) => backend().resolveBlock(id)
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
