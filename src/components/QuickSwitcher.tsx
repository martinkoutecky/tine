import { For, Show, createSignal, createResource, createEffect, createMemo, type JSX } from "solid-js";
import { backend } from "../backend";
import { switcherOpen, closeSwitcher, switcherMode, recentPages } from "../ui";
import { openPage, openPageAtBlock, route } from "../router";
import { paletteCommands } from "../keybindings";
import { blockView } from "../render/block";
import type { PageEntry, PageKind } from "../types";

// One selectable result row.
type Item =
  | { t: "page"; name: string; pageKind: PageKind }
  | { t: "create"; name: string }
  | { t: "command"; label: string; binding: string; run: () => void }
  | { t: "block"; page: string; pageKind: PageKind; blockId: string; text: string };

interface Section {
  header: string;
  items: Item[];
}

const GROUP_LIMIT = 5;

// Ctrl-K: grouped search (Pages / Create / Commands / Blocks), command palette
// (⌘⇧P, commands-only), and recents on an empty query — mirroring OG's cmdk.
export function QuickSwitcher(): JSX.Element {
  const [query, setQuery] = createSignal("");
  const [sel, setSel] = createSignal(0);
  let inputRef: HTMLInputElement | undefined;
  let resultsRef: HTMLDivElement | undefined;
  // Scroll the highlighted row into view as the cursor moves across sections.
  createEffect(() => {
    sel();
    queueMicrotask(() =>
      resultsRef?.querySelector(".switcher-row.active")?.scrollIntoView({ block: "nearest" })
    );
  });

  const commandsOnly = () => switcherMode() === "commands";
  const [pages] = createResource(
    () => (commandsOnly() ? "" : query()),
    (q) => (q.trim() && !commandsOnly() ? backend().quickSwitch(q, 8) : Promise.resolve([] as PageEntry[]))
  );
  const [hits] = createResource(
    () => (commandsOnly() ? "" : query()),
    (q) => (q.trim() && !commandsOnly() ? backend().search(q, 20) : Promise.resolve([]))
  );

  const currentPageName = () => {
    const r = route();
    return r.kind === "page" ? r.name : null;
  };

  const commandItems = (q: string): Item[] => {
    const ql = q.trim().toLowerCase();
    return paletteCommands()
      .filter((c) => !ql || c.label.toLowerCase().includes(ql))
      .map((c) => ({ t: "command" as const, label: c.label, binding: c.binding, run: c.run }));
  };

  const sections = createMemo<Section[]>(() => {
    const q = query().trim();
    const out: Section[] = [];

    if (commandsOnly()) {
      out.push({ header: "Commands", items: commandItems(q) });
      return out;
    }

    // Empty query → recents.
    if (!q) {
      const recents: Item[] = recentPages().map((r) => ({ t: "page", name: r.name, pageKind: r.kind }));
      if (recents.length) out.push({ header: "Recent", items: recents });
      return out;
    }

    const ql = q.toLowerCase();
    // Pages (require contiguous substring — fuzzy subsequence reads as noise here).
    const pageItems: Item[] = (pages() ?? [])
      .filter((e) => e.name.toLowerCase().includes(ql))
      .map((e) => ({ t: "page", name: e.name, pageKind: e.kind }));
    if (pageItems.length) out.push({ header: "Pages", items: pageItems });

    // Create page (when no exact match exists).
    const exact = pageItems.some((p) => p.t === "page" && p.name.toLowerCase() === ql);
    if (!exact) out.push({ header: "Create", items: [{ t: "create", name: q }] });

    // Commands matching the query.
    const cmds = commandItems(q);
    if (cmds.length) out.push({ header: "Commands", items: cmds });

    // Blocks, split into current-page vs the rest.
    const cur = currentPageName();
    const curItems: Item[] = [];
    const otherItems: Item[] = [];
    for (const g of hits() ?? []) {
      for (const b of g.blocks) {
        const text = blockView(b.raw).lines.join(" ").trim();
        if (!text) continue;
        const item: Item = { t: "block", page: g.page, pageKind: g.kind, blockId: b.id, text };
        (cur && g.page === cur ? curItems : otherItems).push(item);
      }
    }
    if (curItems.length) out.push({ header: "Current page", items: curItems });
    if (otherItems.length) out.push({ header: "Blocks", items: otherItems });

    return out;
  });

  // Flattened item list (capped per section) for the single highlight cursor.
  const flat = createMemo<Item[]>(() => sections().flatMap((s) => s.items.slice(0, GROUP_LIMIT)));

  createEffect(() => {
    if (switcherOpen()) {
      setQuery("");
      setSel(0);
      queueMicrotask(() => inputRef?.focus());
    }
  });
  // Keep the highlight in range as results change.
  createEffect(() => {
    const n = flat().length;
    if (sel() >= n) setSel(0);
  });

  const choose = (it: Item) => {
    switch (it.t) {
      case "page":
        openPage(it.name, it.pageKind);
        break;
      case "create":
        void createPage(it.name);
        break;
      case "command":
        closeSwitcher();
        it.run();
        return;
      case "block":
        openPageAtBlock(it.page, it.pageKind, it.blockId);
        break;
    }
    closeSwitcher();
  };

  const createPage = async (name: string) => {
    try {
      await backend().savePage(
        { name, kind: "page", title: name, pre_block: null, blocks: [{ id: "", raw: "", collapsed: false, children: [] }] },
        false
      );
    } catch {
      // ignore — still navigate; the page will be created on first edit
    }
    openPage(name, "page");
  };

  const move = (d: number) => {
    const n = flat().length;
    if (n) setSel((sel() + d + n) % n);
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const ctrl = e.ctrlKey;
    if (e.key === "ArrowDown" || (ctrl && e.key.toLowerCase() === "n")) {
      e.preventDefault();
      move(1);
    } else if (e.key === "ArrowUp" || (ctrl && e.key.toLowerCase() === "p")) {
      e.preventDefault();
      move(-1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      const it = flat()[sel()];
      if (it) choose(it);
    } else if (e.key === "Escape") {
      e.preventDefault();
      closeSwitcher();
    }
  };

  // Running flat index for a given (section, itemIndex), to match the cursor.
  const flatIndex = (sIdx: number, iIdx: number): number => {
    let n = 0;
    const secs = sections();
    for (let s = 0; s < sIdx; s++) n += Math.min(secs[s].items.length, GROUP_LIMIT);
    return n + iIdx;
  };

  return (
    <Show when={switcherOpen()}>
      <div class="switcher-overlay" onClick={closeSwitcher}>
        <div class="switcher" onClick={(e) => e.stopPropagation()}>
          <input
            ref={inputRef}
            class="switcher-input"
            type="text"
            placeholder={commandsOnly() ? "Run a command…" : "Jump to page, search, or run a command…"}
            value={query()}
            onInput={(e) => {
              setQuery(e.currentTarget.value);
              setSel(0);
            }}
            onKeyDown={onKeyDown}
          />
          <div class="switcher-results" ref={resultsRef}>
            <For each={sections()}>
              {(section, sIdx) => (
                <div class="switcher-section">
                  <div class="switcher-group-header">
                    <span>{section.header}</span>
                    <span class="switcher-count">{section.items.length}</span>
                  </div>
                  <For each={section.items.slice(0, GROUP_LIMIT)}>
                    {(it, iIdx) => {
                      const idx = () => flatIndex(sIdx(), iIdx());
                      return (
                        <div
                          class="switcher-row"
                          classList={{ active: idx() === sel() }}
                          onMouseMove={() => setSel(idx())}
                          onMouseDown={(e) => {
                            e.preventDefault();
                            choose(it);
                          }}
                        >
                          <Row item={it} query={query()} />
                        </div>
                      );
                    }}
                  </For>
                </div>
              )}
            </For>
            <Show when={query().trim() && flat().length === 0}>
              <div class="switcher-empty">No matched results</div>
            </Show>
          </div>
          <div class="switcher-footer">
            <span><kbd>↑↓</kbd> navigate</span>
            <span><kbd>↵</kbd> open</span>
            <span><kbd>⌘⇧P</kbd> commands</span>
          </div>
        </div>
      </div>
    </Show>
  );
}

// A single result row body, with an icon tag, label, and (for blocks) a
// highlighted snippet windowed around the match.
function Row(props: { item: Item; query: string }): JSX.Element {
  const it = props.item;
  switch (it.t) {
    case "page":
      return (
        <>
          <span class="switcher-kind">{it.pageKind === "journal" ? "journal" : "page"}</span>
          <span class="switcher-name">{it.name}</span>
        </>
      );
    case "create":
      return (
        <>
          <span class="switcher-kind create">new</span>
          <span class="switcher-name">Create page: <strong>{it.name}</strong></span>
        </>
      );
    case "command":
      return (
        <>
          <span class="switcher-kind cmd">cmd</span>
          <span class="switcher-name">{it.label}</span>
          <Show when={it.binding}>
            <span class="switcher-shortcut">{it.binding}</span>
          </Show>
        </>
      );
    case "block":
      return (
        <>
          <span class="switcher-kind">block</span>
          <span class="switcher-name">
            <span class="switcher-page">{it.page}:</span> {snippet(it.text, props.query)}
          </span>
        </>
      );
  }
}

// Window the text around the first case-insensitive match and wrap it in <mark>.
function snippet(text: string, query: string): JSX.Element {
  const q = query.trim();
  if (!q) return <>{text.slice(0, 120)}</>;
  const i = text.toLowerCase().indexOf(q.toLowerCase());
  if (i === -1) return <>{text.slice(0, 120)}</>;
  const start = Math.max(0, i - 30);
  const pre = (start > 0 ? "…" : "") + text.slice(start, i);
  const match = text.slice(i, i + q.length);
  const post = text.slice(i + q.length, i + q.length + 60);
  return (
    <>
      {pre}
      <mark>{match}</mark>
      {post}
      {i + q.length + 60 < text.length ? "…" : ""}
    </>
  );
}
