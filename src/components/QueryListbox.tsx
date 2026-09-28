import { For, Show, createEffect, createMemo, createSignal, type JSX } from "solid-js";

// **The ONE listbox keyboard/ARIA controller the query sheet uses (§7.7, D-14).** It was a private `Listbox` …

/** Stops a click inside the builder from bubbling to the block's `onClick`, which would drop the block into … */
export const stop = (e: MouseEvent) => e.stopPropagation();

/** **A row's key → the tail of its DOM id (§7.5, I-22).** The keys are the user's own vocabulary: a property … */
export function encodeOptionKey(key: string): string {
  let out = "";
  for (let i = 0; i < key.length; i += 1) {
    out += key.charCodeAt(i).toString(16).padStart(4, "0");
  }
  return out;
}

export interface ListboxOption {
  /** Stable row identity. The option's DOM id is derived from THIS. */
  key: string;
  label: string;
  hint?: string;
  active?: boolean;
  /** A non-selectable section label: drawn, skipped by the arrow keys. */
  header?: boolean;
  /** A richer second line for the row (the vocabulary picker's type/count). */
  detail?: JSX.Element;
}

/** What a replacement list body is handed. */
export interface ListboxBody {
  /** The options to draw, headers included, in order. */
  shown: () => ListboxOption[];
  /** The key the keyboard is on, or `null` for an empty list. */
  activeKey: () => string | null;
  setActiveKey: (key: string) => void;
  /** The DOM id for a row, so `aria-activedescendant` and the row agree. */
  optionId: (key: string) => string;
  pick: (key: string) => void;
  listId: string;
  label: string;
}

const selectable = (options: ListboxOption[]) => options.filter((option) => !option.header);

/** One `role="listbox"` with `aria-activedescendant`, arrow keys and scroll-follow — the pattern … */
export function Listbox(props: {
  id: string;
  label: string;
  options: ListboxOption[];
  filterable?: boolean;
  placeholder?: string;
  /** Controlled needle. */
  query?: string;
  onQuery?: (query: string) => void;
  onPick: (key: string) => void;
  onEmptyBackspace?: () => void;
  rootRef?: (element: HTMLDivElement) => void;
  /** An extra class on the popover root, for a caller whose list needs a different width. */
  class?: string;
  /** Drawn between the filter and the list: a compact line about the LIST itself (the vocabulary picker's "the … */
  status?: JSX.Element;
  /** Draw the rows some other way (virtualized). The keyboard stays here. */
  body?: (context: ListboxBody) => JSX.Element;
  /** Drawn under the list, inside the popover (the "use it anyway" row). */
  footer?: JSX.Element;
  inputRef?: (element: HTMLInputElement) => void;
}): JSX.Element {
  const controlled = () => props.query !== undefined;
  const [ownQuery, setOwnQuery] = createSignal("");
  const query = () => props.query ?? ownQuery();
  const shown = createMemo(() => {
    if (controlled()) return props.options;
    const needle = query().trim().toLowerCase();
    if (!needle) return props.options;
    return props.options.filter(
      (option) => option.header || option.label.toLowerCase().includes(needle),
    );
  });
  const [activeKey, setActiveKey] = createSignal<string | null>(null);
  // A filter that shrinks the list, or empties it, must not leave the keyboard pointing at a row that is no …
  createEffect(() => {
    const rows = selectable(shown());
    const current = activeKey();
    if (current && rows.some((option) => option.key === current)) return;
    setActiveKey(rows[0]?.key ?? null);
  });
  const optionId = (key: string) => `${props.id}-option-${encodeOptionKey(key)}`;
  const move = (delta: number) => {
    const rows = selectable(shown());
    if (!rows.length) return;
    const current = rows.findIndex((option) => option.key === activeKey());
    const next = (current + delta + rows.length) % rows.length;
    setActiveKey(rows[next].key);
  };
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === "ArrowDown" || (event.ctrlKey && event.key === "n")) {
      event.preventDefault();
      move(1);
    } else if (event.key === "ArrowUp" || (event.ctrlKey && event.key === "p")) {
      event.preventDefault();
      move(-1);
    } else if (event.key === "Enter") {
      const key = activeKey();
      if (key) {
        event.preventDefault();
        props.onPick(key);
      }
    } else if (event.key === "Backspace" && !query() && props.onEmptyBackspace) {
      event.preventDefault();
      props.onEmptyBackspace();
    }
  };
  const setQuery = (next: string) => {
    if (!controlled()) setOwnQuery(next);
    props.onQuery?.(next);
  };
  const context: ListboxBody = {
    shown,
    activeKey,
    setActiveKey,
    optionId,
    pick: (key) => props.onPick(key),
    listId: props.id,
    label: props.label,
  };
  return (
    <div
      ref={(element) => props.rootRef?.(element)}
      class="qs-menu"
      classList={{ [props.class ?? ""]: !!props.class }}
      onClick={stop}
      onKeyDown={onKeyDown}
    >
      <Show when={props.filterable}>
        <input
          ref={(element) => props.inputRef?.(element)}
          class="qs-menu-filter"
          autofocus
          role="combobox"
          aria-expanded="true"
          aria-controls={props.id}
          aria-activedescendant={activeKey() ? optionId(activeKey()!) : undefined}
          aria-label={props.label}
          placeholder={props.placeholder ?? "Type to filter"}
          value={query()}
          onInput={(e) => setQuery(e.currentTarget.value)}
        />
      </Show>
      <Show when={props.status}>{props.status}</Show>
      <Show when={props.body} fallback={<PlainList context={context} />}>
        {(body) => body()(context)}
      </Show>
      <Show when={props.footer}>{props.footer}</Show>
    </div>
  );
}

/** The default body: every option, as a button. */
function PlainList(props: { context: ListboxBody }): JSX.Element {
  let listRef: HTMLDivElement | undefined;
  const context = props.context;
  createEffect(() => {
    context.activeKey();
    listRef?.querySelector(".qs-option.active")?.scrollIntoView({ block: "nearest" });
  });
  return (
    <div ref={listRef} id={context.listId} class="qs-options" role="listbox" aria-label={context.label}>
      <For each={context.shown()}>
        {(option) => (
          <Show
            when={!option.header}
            fallback={
              <div class="qs-option-section" role="presentation">
                {option.label}
              </div>
            }
          >
            <button
              type="button"
              id={context.optionId(option.key)}
              class="qs-option"
              classList={{ active: option.key === context.activeKey(), current: option.active }}
              role="option"
              aria-selected={option.key === context.activeKey()}
              title={option.hint}
              onMouseEnter={() => context.setActiveKey(option.key)}
              onClick={() => context.pick(option.key)}
            >
              {option.label}
              <Show when={option.hint}>
                <span class="qs-option-hint">{option.hint}</span>
              </Show>
              <Show when={option.detail}>{option.detail}</Show>
            </button>
          </Show>
        )}
      </For>
    </div>
  );
}
