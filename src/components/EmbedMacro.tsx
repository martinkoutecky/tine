// Block and page embeds ({{embed ((uuid))}} / {{embed [[page]]}}), split out of Macro.tsx (size ratchet).
import { For, Show, createSignal, useContext, type JSX } from "solid-js";
import { backend } from "../backend";
import { pageIdentityKey } from "../ui";
import { blocksContainEdit, createMembershipResource } from "../resultMembership";
import { graphOwner, readOwned } from "../owned";
import { blockHasPageEmbed, blockRef, formatForBlock, pageByName, resolveBlockRef, resolveGuidePageDto, node as docNode } from "../document";
import { blockDtoExternalId } from "../blockIdentity";
import { nearestAncestors } from "../render/blockTrail";
import { visibleBody } from "../render/block";
import { CollapseSurfaceContext } from "./Block";
import { hostFolded } from "./macroFold";
import { resolveBlockBatched } from "../resolveBatch";
import { changed, pageChangeKey } from "../changeKeys";
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
import { editOccurrence, occurrenceEditable, removeOccurrence, createShownOccurrence, useOccurrenceOwner } from "../refOccurrence";

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

  // The host may answer a page embed under another spelling (journal day, case,
  // a Guide page); the summary names the page it answered, so key on both.
  const [answeredPage, setAnsweredPage] = createSignal<string>();
  const pageRevision = () => {
    const requested = pageChangeKey(pageTarget() ?? "");
    const answered = answeredPage();
    const answeredKey = answered === undefined ? requested : pageChangeKey(answered);
    return answeredKey === requested ? changed(requested) : `${changed(requested)}\0${changed(answeredKey)}`;
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
      return g ? { page: g.page, kind: g.kind, path: g.path, blocks: g.blocks, embedId: g.blocks[0]?.id } : null;
    }
    const pageName = pageTarget();
    if (pageName) {
      // Backend miss → the virtual in-app Guide, matched by bare title (the embed
      // carries no source context to remap the name). No-op for real graphs.
      const result = await readOwned(graphOwner(), backend().getPage(pageName, "page"));
      if (result.kind === "stale") return null;
      const p = result.value ?? resolveGuidePageDto(pageName);
      setAnsweredPage(p?.name);
      return p ? { page: p.name, kind: "page" as PageKind, path: result.value?.id, blocks: p.blocks, embedId: undefined } : null;
    }
    return null;
  }, (value, id) => !!value && blocksContainEdit(value.blocks, id), () => true, false,
  () => blockTarget() ? changed(`block:${blockTarget()}`) : pageRevision());

  const embedded = () => readOr(data, undefined, "embed");
  const foldedPageBody = () => {
    if (!pageTarget() || !props.blockId || !blockHasPageEmbed(props.blockId)) return false;
    return hostFolded(collapseSurface, props.blockId);
  };
  // GH #417: actions on this occurrence, in a toolbar above the embed's top-right
  // corner (desktop hover; on touch while a block inside the embed is edited).
  const occurrence = createShownOccurrence(() =>
    insidePeek ? undefined : { host: props.blockId, span: props.span, kind: "embed", target: target() });
  const editable = () => (occurrenceEditable(occurrence()) ? occurrence() : undefined);
  const source = () => {
    const e = embedded();
    if (!e) return null;
    const uuid = blockTarget();
    return uuid ? blockRefTarget(uuid, e) : { page: e.page, pageKind: e.kind };
  };
  const sourceTrail = () => {
    const e = embedded();
    const dto = e?.embedId ? e.blocks[0] : undefined;
    const id = e && dto ? resolveBlockRef({ uuid: blockDtoExternalId(dto), page: e.page, pageKind: e.kind, ...(e.path ? { path: e.path } : {}) }) : null;
    return id && docNode(id) ? nearestAncestors(id) : null;
  };
  return (
    <div class="embed-block" data-foreign-source="embed" classList={{ "embed-page": !!pageTarget() }}>
      <Show when={!insidePeek && (editable() || source())}>
        {/* mousedown: not the host's edit gesture, and keep a touch editor's focus.
            click: an embed rendered inside a block reference must not also run
            that reference's navigation after Open source. */}
        <div class="embed-actions" role="toolbar" aria-label={t("ref-occurrence-actions")}
          onMouseDown={(e) => { e.preventDefault(); e.stopPropagation(); }}
          onClick={(e) => e.stopPropagation()}>
          {/* keyed: each shown host text gets fresh buttons bound to it. */}
          <Show when={editable()} keyed>
            {(o) => <button type="button" class="embed-action-edit" title={t("ref-occurrence-edit-title")}
              onClick={() => editOccurrence(o, owner())}>{t("ref-occurrence-edit")}</button>}
          </Show>
          <Show when={source()}>
            <button type="button" class="embed-action-source"
              onClick={(e) => openRefTarget(source()!, internalLinkDest(e), pane?.paneId ?? focusedPaneId())}>{t("preview-open-source")}</button>
          </Show>
          <Show when={editable()} keyed>
            {(o) => <button type="button" class="embed-action-remove" title={t("ref-occurrence-remove-embed-title")}
              onClick={() => removeOccurrence(o)}>{t("ref-occurrence-remove")}</button>}
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
                {/* GH #779: every ancestor crumb opens that block, at the shared link
                    destination (plain: here/zoom, Shift: sidebar, Ctrl/Cmd: tab, Alt:
                    other pane), like the Open source button. Linked References' plain
                    click grows its own group's context root, which an embed (fixed to
                    its one block) has no counterpart for. */}
                <For each={sourceTrail()!.ids}>{(id) => <><span class="ref-crumb-sep">›</span>
                  <span role="button" tabIndex={0} class="ref-crumb" onKeyDown={(event) => {
                    if (event.target !== event.currentTarget || (event.key !== "Enter" && event.key !== " ")) return;
                    event.preventDefault();
                    event.currentTarget.click();
                  }} onClick={(event) => {
                    event.stopPropagation();
                    if (!docNode(id)) return;
                    openRefTarget(blockRef(id), internalLinkDest(event), pane?.paneId ?? focusedPaneId());
                  }}><InlineText text={visibleBody(docNode(id)?.raw ?? "", formatForBlock(id))[0] || "…"} format={formatForBlock(id)} /></span></>}</For>
              </Show>
            </div>
          </Show>
          <Show when={!insidePeek} fallback={
            <LinkDepthContext.Provider value={linkDepth + 1}>
              <RefBlocks page={embedded()!.page} pageKind={embedded()!.kind} path={embedded()!.path} blocks={embedded()!.blocks} readOnly />
            </LinkDepthContext.Provider>
          }>
            <LiveRefGroup page={embedded()!.page} kind={embedded()!.kind} path={embedded()!.path} blocks={embedded()!.blocks} embedId={embedded()!.embedId} hostBlockId={props.blockId} surface="embed" />
          </Show>
        </Show>
      </Show>
    </div>
  );
}
