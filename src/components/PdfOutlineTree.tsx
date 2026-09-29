import { For, Show, type JSX } from "solid-js";
import type { PdfOutlineItem } from "./pdfOutline";

/** Render the bounded document outline without loading PDF pages. */
export function PdfOutlineTree(props: {
  items: PdfOutlineItem[];
  nested?: boolean;
  expanded: (id: string) => boolean;
  toggle: (id: string) => void;
  activate: (item: PdfOutlineItem) => void;
}): JSX.Element {
  return (
    <ul class={props.nested ? "pdf-outline-children" : "pdf-outline-list"}>
      <For each={props.items}>
        {(item) => (
          <li class="pdf-outline-item">
            <div class="pdf-outline-row">
              <Show
                when={item.children.length}
                fallback={<span class="pdf-outline-disclosure-spacer" aria-hidden="true" />}
              >
                <button
                  type="button"
                  class="pdf-outline-disclosure"
                  aria-label={`${props.expanded(item.id) ? "Collapse" : "Expand"} ${item.label}`}
                  aria-expanded={props.expanded(item.id)}
                  onClick={() => props.toggle(item.id)}
                >
                  {props.expanded(item.id) ? "▾" : "▸"}
                </button>
              </Show>
              <button
                type="button"
                class="pdf-outline-label"
                disabled={item.destination === null}
                onClick={() => props.activate(item)}
              >
                {item.label}
              </button>
            </div>
            <Show when={item.children.length && props.expanded(item.id)}>
              <PdfOutlineTree
                items={item.children}
                nested
                expanded={props.expanded}
                toggle={props.toggle}
                activate={props.activate}
              />
            </Show>
          </li>
        )}
      </For>
    </ul>
  );
}

