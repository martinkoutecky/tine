// Block and page embeds ({{embed ((uuid))}} / {{embed [[page]]}}), split out of Macro.tsx (size ratchet).
import { For, Show, useContext, type JSX } from "solid-js";
import { backend } from "../backend";
import { pageIdentityKey } from "../ui";
import { blocksContainEdit, createMembershipResource } from "../resultMembership";
import { graphOwner, readOwned } from "../owned";
import { blockHasPageEmbed, formatForBlock, pageByName, resolveGuidePageDto, node as docNode } from "../document";
import { nearestAncestors } from "../render/blockTrail";
import { visibleBody } from "../render/block";
import { CollapseSurfaceContext } from "./Block";
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
import { t } from "../i18n";
import type { Span } from "../render/ast";
import { blockRefTarget } from "../resolveBatch";
import { PaneContext, focusedPaneId } from "../panes";
import { internalLinkDest } from "../linkGesture";
import { openRefTarget } from "../blockRefActions";
import { editOccurrence, occurrenceEditable, removeOccurrence, useOccurrenceOwner, type RefOccurrence } from "../refOccurrence";

/** `span` is the macro's parser source span in its host block (`blockId`); with
 *  it, the embed offers Edit link and Remove for this one occurrence (GH #417). */
export function EmbedMacro(props: { body: string; blockId?: string; span?: Span }): JSX.Element {
  const insidePeek = useContext(PeekContext);
  const linkDepth = useContext(LinkDepthContext);
  const collapseSurface = useContext(CollapseSurfaceContext);
  const pane = useContext(PaneContext);
  const owner = useOccurrenceOwner(() => props.blockId);
  if (linkDepth > MAX_DEPTH_OF_LINKS) return <LinkDepthWarning />;

  const target = () => props.body.replace(/^embed\s*/i, "").trim();
  const pageTarget = () => /^\[\[([^\]]+)\]\]$/.exec(target())?.[1];
  const blockTarget = () => /^\(\(([^)]+)\)\)$/.exec(target())?.[1];
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
    const uuid = blockTarget();
    if (uuid) {
      const g = await resolveBlockBatched(uuid);
      // embedId = the embedded block's own id, so its ref-count badge is hidden
      // inside the embed (OG hide-block-refs-count?); its children keep theirs.
      return g ? { page: g.page, kind: g.kind, blocks: g.blocks, embedId: g.blocks[0]?.id } : null;
    }
    const pageName = pageTarget();
    if (pageName) {
      // Backend miss → the virtual in-app Guide, matched by bare title (the embed
      // carries no source context to remap the name). No-op for real graphs.
      const result = await readOwned(graphOwner(), backend().getPage(pageName, "page"));
      if (result.kind === "stale") return null;
      const p = result.value ?? resolveGuidePageDto(pageName);
      return p ? { page: p.name, kind: "page" as PageKind, blocks: p.blocks, embedId: undefined } : null;
    }
    return null;
  }, (value, id) => !!value && blocksContainEdit(value.blocks, id));

  const embedded = () => readOr(data, undefined, "embed");
  const foldedPageBody = () => {
    if (!pageTarget() || !props.blockId || !blockHasPageEmbed(props.blockId)) return false;
    const stored = docNode(props.blockId)?.collapsed ?? false;
    return collapseSurface?.collapsed(props.blockId, stored) ?? stored;
  };
  // GH #417: actions on this occurrence, in a toolbar above the embed's top-right
  // corner (desktop hover; on touch while a block inside the embed is edited).
  const occurrence = (): RefOccurrence | undefined =>
    props.blockId && props.span && !insidePeek
      ? { host: props.blockId, span: props.span, kind: "embed", target: target() }
      : undefined;
  const source = () => {
    const e = embedded();
    if (!e) return null;
    const uuid = blockTarget();
    return uuid ? blockRefTarget(uuid, e) : { page: e.page, pageKind: e.kind };
  };
  const sourceTrail = () => {
    const id = embedded()?.embedId;
    return id && docNode(id) ? nearestAncestors(id) : null;
  };
  return (
    <div class="embed-block" classList={{ "embed-page": !!pageTarget() }}>
      <Show when={!insidePeek && (occurrenceEditable(occurrence()) || source())}>
        {/* mousedown: not the host's edit gesture, and keep a touch editor's focus. */}
        <div class="embed-actions" role="toolbar" aria-label={t("ref-occurrence-actions")}
          onMouseDown={(e) => { e.preventDefault(); e.stopPropagation(); }}>
          <Show when={occurrenceEditable(occurrence())}>
            <button type="button" class="embed-action-edit" title={t("ref-occurrence-edit-title")}
              onClick={() => editOccurrence(occurrence()!, owner())}>{t("ref-occurrence-edit")}</button>
          </Show>
          <Show when={source()}>
            <button type="button" class="embed-action-source"
              onClick={(e) => openRefTarget(source()!, internalLinkDest(e), pane?.paneId ?? focusedPaneId())}>{t("preview-open-source")}</button>
          </Show>
          <Show when={occurrenceEditable(occurrence())}>
            <button type="button" class="embed-action-remove" title={t("ref-occurrence-remove-embed-title")}
              onClick={() => removeOccurrence(occurrence()!)}>{t("ref-occurrence-remove")}</button>
          </Show>
        </div>
      </Show>
      {/* GH #439: a page embed names its page (OG block.cljs `page-embed` header). */}
      <Show when={pageTarget() && (selfPageEmbed() || embedded() || foldedPageBody())}>
        <PageEmbedHeader name={embedded()?.page ?? pageTarget()!} blockId={props.blockId} />
      </Show>
      <Show when={!selfPageEmbed() && !foldedPageBody()}>
        <Show when={embedded()} fallback={<div class="embed-missing">{`{{${props.body}}}`}</div>}>
          <Show when={!pageTarget() && styleKnobValues()["embed-breadcrumbs"] === true}>
            {/* GH #440: the page, then the embedded block's nearest ancestors
                (OG `breadcrumb`, as Linked References shows them); the backend
                row carries no trail, so it is read from the loaded source. */}
            <div class="embed-source-breadcrumb">
              {/* OG breadcrumb names the page via page-cp: no link brackets. */}
              <PageRef name={embedded()!.page} alias={embedded()!.page} />
              <Show when={sourceTrail()?.more || (!sourceTrail() && (embedded()!.blocks[0]?.breadcrumb?.length ?? 0) > 3)}><span class="ref-crumb-sep">›</span><span class="ref-crumb">…</span></Show>
              <Show when={sourceTrail()} fallback={
                <For each={(embedded()!.blocks[0]?.breadcrumb ?? []).slice(-3)}>{(label) => <><span class="ref-crumb-sep">›</span><span class="ref-crumb"><InlineText text={label} /></span></>}</For>
              }>
                <For each={sourceTrail()!.ids}>{(id) => <><span class="ref-crumb-sep">›</span>
                  <span class="ref-crumb"><InlineText text={visibleBody(docNode(id)?.raw ?? "")[0] || "…"} format={formatForBlock(id)} /></span></>}</For>
              </Show>
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
