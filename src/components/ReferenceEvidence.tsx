import { t } from "../i18n";
import { For, Show, createMemo, createSignal, type JSX } from "solid-js";
import { openInNewTab, openPageAtBlock } from "../router";
import { openRouteInOtherPane } from "../panes";
import { internalLinkAuxClick, internalLinkDest, internalLinkMouseDown } from "../linkGesture";
import { openBlockInSidebar, openContextMenu } from "../ui";
import { shouldOpenBlockContextMenu } from "../contextMenuPolicy";
import { focusedSurfaceOwner } from "../focusedSurface";
import { formatForPage, resolveBlockRef, node as docNode } from "../document";
import { startEditing, type EditorSelection } from "../editorController";
import type { BlockDto, MatchSpan, PageKind, ReferenceBlockEvidence } from "../types";
import { blockDtoExternalId } from "../blockIdentity";
import { EmojiText } from "../render/emoji";
import { buildSearchExcerpt, type Segment } from "./SearchResultRow";
import { BUILTIN_HIDE, rawOffsetToVisibleOffset } from "../editor/properties";

/**
 * Land on the occurrence as a SELECTION, not a collapsed caret. Every jump for
 * one block opens the same block; only where the caret ended up told jump 1 from
 * jump 4, and iOS/iPadOS paints no caret for a programmatic focus (master GH
 * #200). A selection is drawn on every platform. LiveRefGroup uses this too.
 */
export function occurrenceSelection(raw: string, span: MatchSpan, page: string): EditorSelection {
  const format = formatForPage(page);
  const start = rawOffsetToVisibleOffset(raw, span.start, BUILTIN_HIDE, format);
  const end = rawOffsetToVisibleOffset(raw, span.end, BUILTIN_HIDE, format);
  return { start, end: Math.max(start, end), direction: "forward" };
}

function focusMainOccurrence(
  page: string,
  kind: PageKind,
  blockId: string,
  span: MatchSpan,
  path?: string,
) {
  openPageAtBlock(page, kind, blockId, path);
  // The retry loop below waits for the target page to load. It belongs to the
  // surface the click opened, so a later navigation, tab or pane change, or a
  // graph switch, ends it instead of grabbing the editor from wherever the user
  // went (I-20). Captured AFTER the navigation it performs.
  const owner = focusedSurfaceOwner();
  let attempts = 0;
  const focus = () => {
    if (!owner()) return;
    const runtimeId = resolveBlockRef({ uuid: blockId, page, pageKind: kind, ...(path ? { path } : {}) });
    if (runtimeId && docNode(runtimeId)) {
      const block = docNode(runtimeId);
      startEditing(runtimeId, occurrenceSelection(block.raw, span, page), null, "main");
    } else if (attempts++ < 30) {
      setTimeout(focus, 50);
    }
  };
  setTimeout(focus, 60);
}

export function OccurrenceControls(props: {
  evidence: ReferenceBlockEvidence;
  /** Jump mode (Linked References' live block has no excerpt to mark): each
   *  numbered circle opens the mention in the editor. */
  onOccurrence?: (span: MatchSpan) => void;
  /** Select mode (Unlinked References' excerpt): the circles choose WHICH mention
   *  the excerpt highlights, in place; nothing navigates (GH #556 item 3). */
  selected?: number;
  onSelect?: (index: number) => void;
}): JSX.Element {
  const total = () => props.evidence.total ?? props.evidence.occurrences.length;
  const selecting = () => props.onSelect !== undefined;
  // Logseq shows no per-block mention count or occurrence-jump controls in
  // Linked References. For a single mention the "1 mention" label + lone "1" jump
  // button are redundant and confusing (GH #200), so surface these controls only
  // when a block mentions the page more than once — where jumping to a specific
  // occurrence is actually useful. Gate on the true total (GH #137), not the
  // capped occurrence list, so an honest ">1" is what shows the controls.
  return (
    <Show when={total() > 1}>
    <span class="reference-occurrence-controls">
      <span class="reference-mention-count">
        {t("reference-evidence-mention-count", { count: total() })}
      </span>
      <For each={props.evidence.occurrences}>
        {(occurrence, index) => (
          <button
            type="button"
            class="reference-occurrence-jump"
            classList={{ selected: selecting() && props.selected === index() }}
            title={selecting() ? t("reference-evidence-highlight-mention", { kind: String(occurrence.kind), value: String(index() + 1) }) : t("reference-evidence-jump-to-mention", { kind: String(occurrence.kind), value: String(index() + 1) })}
            aria-label={t("reference-evidence-occurrence-label", { selecting: selecting() ? 1 : 0, index: String(index() + 1), total: String(total()) })}
            aria-pressed={selecting() ? props.selected === index() : undefined}
            onClick={() => (selecting() ? props.onSelect!(index()) : props.onOccurrence?.(occurrence.span))}
          >
            {index() + 1}
          </button>
        )}
      </For>
    </span>
    </Show>
  );
}

/**
 * Every span marked over the whole block, with no excerpt window. "Show full
 * block" is how a reader reaches mentions past the excerpt's windows, which is
 * exactly when the marking matters most.
 */
export function buildFullMarkedSegments(text: string, spans: MatchSpan[]): Segment[] {
  const ordered = [...spans].filter((span) => span.end > span.start).sort((a, b) => a.start - b.start);
  const segments: Segment[] = [];
  let cursor = 0;
  for (const span of ordered) {
    const start = Math.max(cursor, Math.min(text.length, span.start));
    const end = Math.max(start, Math.min(text.length, span.end));
    if (end <= start) continue;
    if (start > cursor) segments.push({ text: text.slice(cursor, start), marked: false });
    segments.push({ text: text.slice(start, end), marked: true, span });
    cursor = end;
  }
  if (cursor < text.length) segments.push({ text: text.slice(cursor), marked: false });
  return segments;
}

export function ReferenceExcerptBlocks(props: {
  blocks: BlockDto[];
  evidence: ReferenceBlockEvidence[];
  page: string;
  kind: PageKind;
  path?: string;
}): JSX.Element {
  const evidenceById = createMemo(() => new Map(props.evidence.map((item) => [item.block_id, item])));
  const [full, setFull] = createSignal<Record<string, boolean>>({});
  // GH #556 item 1/3: ONE mention is highlighted per block (the first until the
  // reader picks another with the numbered circles), not every mention at once:
  // all-occurrence marking cluttered long blocks and fought the user's own
  // highlighting. Picking a circle stays in this view; the excerpt re-centres on it.
  const [selected, setSelected] = createSignal<Record<string, number>>({});
  return (
    <For each={props.blocks}>
      {(block) => {
        const evidence = () => evidenceById().get(block.id);
        const spans = () => evidence()?.occurrences.map((occurrence) => occurrence.span) ?? [];
        const selectedIndex = () => Math.min(selected()[block.id] ?? 0, Math.max(0, spans().length - 1));
        const markedSpans = () => (spans().length ? [spans()[selectedIndex()]] : []);
        const segments = (): Segment[] => {
          if (!evidence()) return [{ text: block.raw, marked: false }];
          return full()[block.id] ? buildFullMarkedSegments(block.raw, markedSpans()) : buildSearchExcerpt(block.raw, markedSpans());
        };
        // The label names the MENTION's ordinal, not the excerpt segment's.
        const ordinalOf = (span: MatchSpan) => {
          const index = spans().findIndex((candidate) => candidate.start === span.start && candidate.end === span.end);
          return index < 0 ? 1 : index + 1;
        };
        const sourceRoute = () => ({
          kind: "page" as const, name: props.page, pageKind: props.kind, block: blockDtoExternalId(block),
          ...(props.path ? { path: props.path } : {}),
        });
        const jumpTo = (span: MatchSpan) => focusMainOccurrence(props.page, props.kind, blockDtoExternalId(block), span, props.path);
        return (
          <div
            class="reference-excerpt-row"
            data-reference-block={block.id}
            onContextMenu={(event) => {
              // The block's own context menu, once its page is loaded (the row
              // is a bounded excerpt until then and has no runtime block).
              if (!docNode(block.id) || !shouldOpenBlockContextMenu(event.target)) return;
              event.preventDefault();
              openContextMenu(event.clientX, event.clientY, block.id);
            }}
          >
            {/* GH #556 item 7: the bullet is the block-level way into the source
                block, as in Linked References and the outline; the highlighted
                text locates a mention. Same modified-click contract as every
                internal link (linkGesture.ts). */}
            <button
              type="button"
              class="reference-excerpt-bullet"
              title={t("reference-evidence-open-this-block-in", { page: String(props.page) })}
              aria-label={t("reference-evidence-open-this-block-in", { page: String(props.page) })}
              onMouseDown={internalLinkMouseDown}
              onClick={(event) => {
                switch (internalLinkDest(event)) {
                  case "sidebar":
                    openBlockInSidebar({ page: props.page, pageKind: props.kind, uuid: blockDtoExternalId(block), ...(props.path ? { path: props.path } : {}) });
                    break;
                  case "background": openInNewTab(sourceRoute()); break;
                  case "pane": openRouteInOtherPane(sourceRoute()); break;
                  default: openPageAtBlock(props.page, props.kind, blockDtoExternalId(block), props.path);
                }
              }}
              onAuxClick={(event) => internalLinkAuxClick(event, () => openInNewTab(sourceRoute()))}
            >•</button>
            <div class="reference-excerpt-content">
              <div class="reference-excerpt-text" aria-label={block.raw}>
                <For each={segments()}>
                  {(segment) => segment.marked && segment.span
                    ? (
                      <button
                        type="button"
                        class="reference-excerpt-mark"
                        // A control, but its text is the excerpt's text: in-page
                        // find must still see it (src/inpageFind.ts).
                        data-inpage-find-text=""
                        title={t("reference-evidence-open-this-mention-in", { page: String(props.page) })}
                        aria-label={t("reference-evidence-open-mention-in", { span: String(ordinalOf(segment.span!)), page: String(props.page) })}
                        onClick={() => jumpTo(segment.span!)}
                      >
                        <EmojiText text={segment.text} />
                      </button>
                    )
                    : segment.marked
                      ? <mark><EmojiText text={segment.text} /></mark>
                      : <EmojiText text={segment.text} />}
                </For>
              </div>
              <div class="reference-excerpt-actions">
                <Show when={evidence()}>
                  {(item) => (
                    <OccurrenceControls
                      evidence={item()}
                      selected={selectedIndex()}
                      onSelect={(index) => setSelected((state) => ({ ...state, [block.id]: index }))}
                    />
                  )}
                </Show>
                <button
                  type="button"
                  class="reference-show-full"
                  aria-expanded={!!full()[block.id]}
                  onClick={() => setFull((state) => ({ ...state, [block.id]: !state[block.id] }))}
                >
                  {full()[block.id] ? t("reference-evidence-show-excerpt") : t("reference-evidence-show-full-block")}
                </button>
              </div>
            </div>
          </div>
        );
      }}
    </For>
  );
}
