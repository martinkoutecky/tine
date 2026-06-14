import { For, Show, createSignal, createResource, createEffect, type JSX } from "solid-js";
import { backend } from "../backend";
import { switcherOpen, closeSwitcher } from "../ui";
import { openPage } from "../router";
import type { PageEntry } from "../types";

// Ctrl-K quick switcher: fuzzy page jump + a few full-text hits.
export function QuickSwitcher(): JSX.Element {
  const [query, setQuery] = createSignal("");
  const [sel, setSel] = createSignal(0);
  let inputRef: HTMLInputElement | undefined;

  const [pages] = createResource(query, (q) => backend().quickSwitch(q, 8));

  type Row = { kind: "page"; entry: PageEntry };
  const rows = (): Row[] => (pages() ?? []).map((entry) => ({ kind: "page" as const, entry }));

  createEffect(() => {
    if (switcherOpen()) {
      setQuery("");
      setSel(0);
      queueMicrotask(() => inputRef?.focus());
    }
  });

  const choose = (r: Row) => {
    openPage(r.entry.name, r.entry.kind);
    closeSwitcher();
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const list = rows();
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSel(list.length ? (sel() + 1) % list.length : 0);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSel(list.length ? (sel() - 1 + list.length) % list.length : 0);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const r = list[sel()];
      if (r) choose(r);
    } else if (e.key === "Escape") {
      e.preventDefault();
      closeSwitcher();
    }
  };

  return (
    <Show when={switcherOpen()}>
      <div class="switcher-overlay" onClick={closeSwitcher}>
        <div class="switcher" onClick={(e) => e.stopPropagation()}>
          <input
            ref={inputRef}
            class="switcher-input"
            type="text"
            placeholder="Jump to page…"
            value={query()}
            onInput={(e) => {
              setQuery(e.currentTarget.value);
              setSel(0);
            }}
            onKeyDown={onKeyDown}
          />
          <div class="switcher-results">
            <For each={rows()}>
              {(r, i) => (
                <div
                  class="switcher-row"
                  classList={{ active: i() === sel() }}
                  onMouseDown={(e) => {
                    e.preventDefault();
                    choose(r);
                  }}
                >
                  <span class="switcher-kind">{r.entry.kind === "journal" ? "journal" : "page"}</span>
                  <span class="switcher-name">{r.entry.name}</span>
                </div>
              )}
            </For>
            <Show when={rows().length === 0 && query()}>
              <div class="switcher-empty">No matching pages</div>
            </Show>
          </div>
        </div>
      </div>
    </Show>
  );
}
