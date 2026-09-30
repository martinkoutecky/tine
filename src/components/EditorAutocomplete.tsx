import { For, Show, createEffect, type JSX } from "solid-js";
import { Portal } from "solid-js/web";

/** The editor's portaled completion surface. Pending and failed block reads
 * stay local to this popup, preserving the text and offering an explicit retry.
 * Rendering costs O(supplied completion items); no graph reads or writes. */
export function EditorAutocomplete<T extends { label: string; sub?: string }>(props: {
  items: readonly T[];
  index: number;
  style: Record<string, string>;
  listRef: (element: HTMLDivElement) => void;
  select: (item: T) => void;
  blockState: "pending" | "error" | "ready" | null;
  retry: () => void;
}): JSX.Element {
  let list: HTMLDivElement | undefined;
  createEffect(() => {
    props.index;
    queueMicrotask(() => list?.querySelector(".ac-item.active")?.scrollIntoView({ block: "nearest" }));
  });
  return <Portal>
    <div class="autocomplete" ref={(element) => { list = element; props.listRef(element); }} data-lenis-prevent style={props.style}>
      <Show when={props.blockState === "pending"}><div class="ac-item" role="status">Searching…</div></Show>
      <Show when={props.blockState === "error"}>
        <div class="ac-item" role="alert">Couldn’t load blocks. <button type="button"
          onMouseDown={(event) => event.preventDefault()} onClick={props.retry}>Retry</button></div>
      </Show>
      <Show when={props.blockState === "ready" && props.items.length === 0}>
        <div class="ac-item" role="status">No matched blocks</div>
      </Show>
      <For each={props.items}>{(item, index) => <div class="ac-item" classList={{ active: index() === props.index }}
        onMouseDown={(event) => { event.preventDefault(); props.select(item); }}>
        <span class="ac-label">{item.label}</span>
        <Show when={item.sub}><span class="ac-sub">{item.sub}</span></Show>
      </div>}</For>
    </div>
  </Portal>;
}
