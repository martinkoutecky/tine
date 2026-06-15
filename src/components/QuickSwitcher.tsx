import { For, Show, createSignal, createResource, createEffect, type JSX } from "solid-js";
import { backend } from "../backend";
import { switcherOpen, closeSwitcher } from "../ui";
import { openPage } from "../router";
import { blockView } from "../render/block";
import type { PageEntry, PageKind } from "../types";

type Row =
  | { kind: "page"; entry: PageEntry }
  | { kind: "block"; page: string; pageKind: PageKind; text: string };

// Ctrl-K quick switcher: fuzzy page jump plus full-text content hits.
export function QuickSwitcher(): JSX.Element {
  const [query, setQuery] = createSignal("");
  const [sel, setSel] = createSignal(0);
  let inputRef: HTMLInputElement | undefined;

  const [pages] = createResource(query, (q) => backend().quickSwitch(q, 8));
  const [hits] = createResource(query, (q) => (q.trim() ? backend().search(q, 12) : Promise.resolve([])));

  const rows = (): Row[] => {
    // Page matches: require the query as a contiguous substring of the title.
    // The backend's quick_switch is fuzzy (subsequence) for `[[` autocomplete,
    // but in a search box subsequence matches read as false positives (e.g.
    // "banka" matching "BP téma Milan Wikarski").
    const q = query().trim().toLowerCase();
    const pageRows: Row[] = (pages() ?? [])
      .filter((entry) => !q || entry.name.toLowerCase().includes(q))
      .map((entry) => ({ kind: "page" as const, entry }));
    const blockRows: Row[] = [];
    for (const g of hits() ?? []) {
      for (const b of g.blocks) {
        const text = blockView(b.raw).lines.join(" ").trim();
        if (!text) continue;
        blockRows.push({ kind: "block", page: g.page, pageKind: g.kind, text });
      }
    }
    return [...pageRows, ...blockRows];
  };

  createEffect(() => {
    if (switcherOpen()) {
      setQuery("");
      setSel(0);
      queueMicrotask(() => inputRef?.focus());
    }
  });

  const choose = (r: Row) => {
    if (r.kind === "page") openPage(r.entry.name, r.entry.kind);
    else openPage(r.page, r.pageKind);
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
            placeholder="Jump to page or search…"
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
                  <Show
                    when={r.kind === "page"}
                    fallback={
                      <>
                        <span class="switcher-kind">text</span>
                        <span class="switcher-name">
                          <span class="switcher-page">{(r as { page: string }).page}:</span>{" "}
                          {(r as { text: string }).text}
                        </span>
                      </>
                    }
                  >
                    <span class="switcher-kind">
                      {(r as { entry: PageEntry }).entry.kind === "journal" ? "journal" : "page"}
                    </span>
                    <span class="switcher-name">{(r as { entry: PageEntry }).entry.name}</span>
                  </Show>
                </div>
              )}
            </For>
            <Show when={rows().length === 0 && query()}>
              <div class="switcher-empty">No matches</div>
            </Show>
          </div>
        </div>
      </div>
    </Show>
  );
}
