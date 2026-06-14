// Inline markdown -> Solid components. Produces real interactive DOM (clickable
// [[links]] and #tags), not an innerHTML string. Used to render a block when it
// is not being edited.

import { For, Show, createResource, type JSX } from "solid-js";
import { openPage } from "../router";
import { parseInline, type Seg } from "./parseInline";
import { blockView } from "./block";
import { backend } from "../backend";

function renderSegs(segs: Seg[]): JSX.Element {
  return <For each={segs}>{(s) => renderSeg(s)}</For>;
}

function renderSeg(s: Seg): JSX.Element {
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
            openPage(s.name);
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
            openPage(s.name);
          }}
        >
          #{s.name}
        </a>
      );
    case "blockref":
      return <BlockRefView id={s.id} />;
    case "macro":
      return <span class="macro">{`{{${s.body}}}`}</span>;
    case "math":
      return <span class="math">{s.tex}</span>;
    case "link":
      return (
        <a class="external-link" href={s.url} target="_blank" rel="noreferrer">
          {s.label || s.url}
        </a>
      );
    case "image":
      return <img class="inline-image" src={s.url} alt={s.alt} />;
  }
}

/** Render a block's body text (already stripped of marker/heading prefix). */
export function InlineText(props: { text: string }): JSX.Element {
  return <>{renderSegs(parseInline(props.text))}</>;
}

// Inline ((block reference)): resolves to the referenced block's first line and
// navigates to its source page on click.
function BlockRefView(props: { id: string }): JSX.Element {
  const [grp] = createResource(
    () => props.id,
    (id) => backend().resolveBlock(id)
  );
  return (
    <span
      class="block-ref"
      onClick={(e) => {
        e.stopPropagation();
        const g = grp();
        if (g) openPage(g.page, g.kind);
      }}
    >
      <Show when={grp()} fallback={<>(({props.id.slice(0, 8)}))</>}>
        <InlineText text={blockView(grp()!.blocks[0].raw).lines[0]} />
      </Show>
    </span>
  );
}
