// Block and page embeds ({{embed ((uuid))}} / {{embed [[page]]}}), split out of Macro.tsx (size ratchet).
import { For, Show, useContext, type JSX } from "solid-js";
import { backend } from "../backend";
import { pageIdentityKey } from "../ui";
import { blocksContainEdit, createMembershipResource } from "../resultMembership";
import { graphOwner, readOwned } from "../owned";
import { pageByName, resolveGuidePageDto, node as docNode } from "../document";
import { resolveBlockBatched } from "../resolveBatch";
import { LiveRefGroup } from "./LiveRefGroup";
import { PageEmbedHeader } from "./PageEmbedHeader";
import { InlineText, PageRef } from "../render/inline";
import { styleKnobValues } from "../styleKnobs";
import { PeekContext } from "../render/PeekPopup";
import { RefBlocks } from "./RefBlocks";
import type { PageKind } from "../types";
import { LinkDepthContext, LinkDepthWarning, MAX_DEPTH_OF_LINKS } from "./linkDepth";
import { readOr } from "../resourceRead";

export function EmbedMacro(props: { body: string; blockId?: string }): JSX.Element {
  const insidePeek = useContext(PeekContext);
  const linkDepth = useContext(LinkDepthContext);
  if (linkDepth > MAX_DEPTH_OF_LINKS) return <LinkDepthWarning />;

  const target = () => props.body.replace(/^embed\s*/i, "").trim();
  const pageTarget = () => /^\[\[([^\]]+)\]\]$/.exec(target())?.[1];
  const selfPageEmbed = () => {
    const sourcePage = props.blockId ? docNode(props.blockId)?.page : undefined;
    const targetPage = pageTarget();
    return !!sourcePage
      && pageByName(sourcePage)?.kind === "page"
      && !!targetPage
      && pageIdentityKey(sourcePage) === pageIdentityKey(targetPage);
  };

  const [data] = createMembershipResource(
    () => selfPageEmbed() ? null : target(),
    target,
    async () => {
    const t = target();
    const blockRef = /^\(\(([^)]+)\)\)$/.exec(t);
    if (blockRef) {
      const g = await resolveBlockBatched(blockRef[1]);
      // embedId = the embedded block's own id, so its ref-count badge is hidden
      // inside the embed (OG hide-block-refs-count?); its children keep theirs.
      return g ? { page: g.page, kind: g.kind, blocks: g.blocks, embedId: g.blocks[0]?.id } : null;
    }
    const pageRef = /^\[\[([^\]]+)\]\]$/.exec(t);
    if (pageRef) {
      // Backend miss → the virtual in-app Guide, matched by bare title (the embed
      // carries no source context to remap the name). No-op for real graphs.
      const result = await readOwned(graphOwner(), backend().getPage(pageRef[1], "page"));
      if (result.kind === "stale") return null;
      const p = result.value ?? resolveGuidePageDto(pageRef[1]);
      return p ? { page: p.name, kind: "page" as PageKind, blocks: p.blocks, embedId: undefined } : null;
    }
    return null;
  }, (value, id) => !!value && blocksContainEdit(value.blocks, id));

  const embedded = () => readOr(data, undefined, "embed");
  return (
    <div class="embed-block" classList={{ "embed-page": !!pageTarget() }}>
      {/* GH #439: a page embed names its page (OG block.cljs `page-embed` header). */}
      <Show when={pageTarget() && (selfPageEmbed() || embedded())}>
        <PageEmbedHeader name={embedded()?.page ?? pageTarget()!} blockId={props.blockId} />
      </Show>
      <Show when={!selfPageEmbed() && !(pageTarget() && props.blockId && docNode(props.blockId)?.collapsed)}>
        <Show when={embedded()} fallback={<div class="embed-missing">{`{{${props.body}}}`}</div>}>
          <Show when={!pageTarget() && styleKnobValues()["embed-breadcrumbs"] === true}>
            <div class="embed-source-breadcrumb">
              <PageRef name={embedded()!.page} />
              <For each={embedded()!.blocks[0]?.breadcrumb ?? []}>{label => <><span class="ref-crumb-sep">›</span><InlineText text={label} /></>}</For>
            </div>
          </Show>
          <Show when={!insidePeek} fallback={
            <LinkDepthContext.Provider value={linkDepth + 1}>
              <RefBlocks page={embedded()!.page} pageKind={embedded()!.kind} blocks={embedded()!.blocks} readOnly />
            </LinkDepthContext.Provider>
          }>
            <LiveRefGroup page={embedded()!.page} kind={embedded()!.kind} blocks={embedded()!.blocks} embedId={embedded()!.embedId} hostBlockId={props.blockId} surface="embed" />
          </Show>
        </Show>
      </Show>
    </div>
  );
}
