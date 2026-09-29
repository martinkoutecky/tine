import { For, Show, createMemo, createSignal, type JSX } from "solid-js";
import { openPageAtBlock } from "../router";
import { formatForPage, resolveBlockRef, node as docNode } from "../document";
import { startEditing, type EditorSelection } from "../editorController";
import type { BlockDto, MatchSpan, PageKind, ReferenceBlockEvidence } from "../types";
import { blockDtoExternalId } from "../blockIdentity";
import { EmojiText } from "../render/emoji";
import { buildSearchExcerpt, type Segment } from "./SearchResultRow";
import { isBuiltinHidden, rawOffsetToVisibleOffset } from "../editor/properties";

/**
 * Land on the occurrence as a SELECTION, not a collapsed caret. Every jump for
 * one block opens the same block; only where the caret ended up told jump 1 from
 * jump 4, and iOS/iPadOS paints no caret for a programmatic focus (master GH
 * #200). A selection is drawn on every platform. LiveRefGroup uses this too.
 */
export function occurrenceSelection(raw: string, span: MatchSpan, page: string): EditorSelection {
  const format = formatForPage(page);
  const start = rawOffsetToVisibleOffset(raw, span.start, isBuiltinHidden, format);
  const end = rawOffsetToVisibleOffset(raw, span.end, isBuiltinHidden, format);
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
  let attempts = 0;
  const focus = () => {
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
  onOccurrence: (span: MatchSpan) => void;
  /**
   * Occurrences the surrounding surface already shows the user. Unlinked
   * References renders a highlighted excerpt, so a mention it marked is one the
   * reader can already see and click; numbering it again is redundant. Linked
   * References renders the live block and marks nothing, so it passes 0 and
   * keeps the full control.
   */
  visible?: number;
}): JSX.Element {
  const total = () => props.evidence.total ?? props.evidence.occurrences.length;
  // Logseq shows no per-block mention count or occurrence-jump controls in
  // Linked References. For a single mention the "1 mention" label + lone "1" jump
  // button are redundant and confusing (GH #200), so surface these controls only
  // when a block mentions the page more than once — where jumping to a specific
  // occurrence is actually useful. Gate on the true total (GH #137), not the
  // capped occurrence list, so an honest ">1" is what shows the controls.
  //
  // Second gate (master GH #200): only when the surface cannot already show the
  // occurrences itself. Derived from the excerpt, not a tuned constant: only the
  // mentions beyond its window need a numbered jump.
  return (
    <Show when={total() > 1 && total() > (props.visible ?? 0)}>
    <span class="reference-occurrence-controls">
      <span class="reference-mention-count">
        {total()} {total() === 1 ? "mention" : "mentions"}
      </span>
      <For each={props.evidence.occurrences}>
        {(occurrence, index) => (
          <button
            type="button"
            class="reference-occurrence-jump"
            title={`Jump to ${occurrence.kind} mention ${index() + 1}`}
            aria-label={`Jump to mention ${index() + 1} of ${total()}`}
            onClick={() => props.onOccurrence(occurrence.span)}
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
  return (
    <For each={props.blocks}>
      {(block) => {
        const evidence = () => evidenceById().get(block.id);
        const spans = () => evidence()?.occurrences.map((occurrence) => occurrence.span) ?? [];
        const segments = (): Segment[] => {
          if (!evidence()) return [{ text: block.raw, marked: false }];
          return full()[block.id] ? buildFullMarkedSegments(block.raw, spans()) : buildSearchExcerpt(block.raw, spans());
        };
        // What the reader can already see and click; drives the jump row's second
        // gate, so expanding a block retires the row on its own.
        const shownOccurrences = () => new Set(
          segments().filter((segment) => segment.marked && segment.span).map((segment) => `${segment.span!.start}:${segment.span!.end}`),
        ).size;
        // The label names the MENTION's ordinal, not the excerpt segment's.
        const ordinalOf = (span: MatchSpan) => {
          const index = spans().findIndex((candidate) => candidate.start === span.start && candidate.end === span.end);
          return index < 0 ? 1 : index + 1;
        };
        const jumpTo = (span: MatchSpan) => focusMainOccurrence(props.page, props.kind, blockDtoExternalId(block), span, props.path);
        return (
          <div class="reference-excerpt-row" data-reference-block={block.id}>
            <span class="reference-excerpt-bullet" aria-hidden="true">•</span>
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
                        title={`Open this mention in ${props.page}`}
                        aria-label={`Open mention ${ordinalOf(segment.span!)} in ${props.page}`}
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
                      visible={shownOccurrences()}
                      onOccurrence={jumpTo}
                    />
                  )}
                </Show>
                <button
                  type="button"
                  class="reference-show-full"
                  aria-expanded={!!full()[block.id]}
                  onClick={() => setFull((state) => ({ ...state, [block.id]: !state[block.id] }))}
                >
                  {full()[block.id] ? "Show excerpt" : "Show full block"}
                </button>
              </div>
            </div>
          </div>
        );
      }}
    </For>
  );
}
