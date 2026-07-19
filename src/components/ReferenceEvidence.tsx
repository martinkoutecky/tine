import { For, Show, createMemo, createSignal, type JSX } from "solid-js";
import { openPageAtBlock } from "../router";
import { doc, formatForPage } from "../store";
import { startEditing } from "../editorController";
import type { BlockDto, PageKind, ReferenceBlockEvidence } from "../types";
import { EmojiText } from "../render/emoji";
import { buildSearchExcerpt } from "./SearchResultRow";
import { isBuiltinHidden, rawOffsetToVisibleOffset } from "../editor/properties";

function focusMainOccurrence(
  page: string,
  kind: PageKind,
  blockId: string,
  offset: number,
) {
  openPageAtBlock(page, kind, blockId);
  let attempts = 0;
  const focus = () => {
    if (doc.byId[blockId]) {
      const block = doc.byId[blockId];
      startEditing(
        blockId,
        rawOffsetToVisibleOffset(block.raw, offset, isBuiltinHidden, formatForPage(page)),
        null,
        "main",
      );
    } else if (attempts++ < 30) {
      setTimeout(focus, 50);
    }
  };
  setTimeout(focus, 60);
}

export function OccurrenceControls(props: {
  evidence: ReferenceBlockEvidence;
  onOccurrence: (offset: number) => void;
}): JSX.Element {
  const total = () => props.evidence.total ?? props.evidence.occurrences.length;
  return (
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
            onClick={() => props.onOccurrence(occurrence.span.start)}
          >
            {index() + 1}
          </button>
        )}
      </For>
    </span>
  );
}

export function ReferenceExcerptBlocks(props: {
  blocks: BlockDto[];
  evidence: ReferenceBlockEvidence[];
  page: string;
  kind: PageKind;
}): JSX.Element {
  const evidenceById = createMemo(() => new Map(props.evidence.map((item) => [item.block_id, item])));
  const [full, setFull] = createSignal<Record<string, boolean>>({});
  return (
    <For each={props.blocks}>
      {(block) => {
        const evidence = () => evidenceById().get(block.id);
        const segments = () => {
          const item = evidence();
          return item
            ? buildSearchExcerpt(block.raw, item.occurrences.map((occurrence) => occurrence.span))
            : [{ text: block.raw, marked: false }];
        };
        return (
          <div class="reference-excerpt-row" data-reference-block={block.id}>
            <span class="reference-excerpt-bullet" aria-hidden="true">•</span>
            <div class="reference-excerpt-content">
              <div class="reference-excerpt-text" aria-label={block.raw}>
                <Show
                  when={!full()[block.id]}
                  fallback={<EmojiText text={block.raw} />}
                >
                  <For each={segments()}>
                    {(segment) => segment.marked
                      ? <mark><EmojiText text={segment.text} /></mark>
                      : <EmojiText text={segment.text} />}
                  </For>
                </Show>
              </div>
              <div class="reference-excerpt-actions">
                <Show when={evidence()}>
                  {(item) => (
                    <OccurrenceControls
                      evidence={item()}
                      onOccurrence={(offset) => focusMainOccurrence(props.page, props.kind, block.id, offset)}
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
