import { For, Show, createSignal, createResource, createEffect, createMemo, onCleanup, type JSX } from "solid-js";
import { backend } from "../backend";
import { switcherOpen, closeSwitcher, switcherMode, recentPages } from "../ui";
import { openPage, openPageAtBlock, openPageInNewTab, route } from "../router";
import { paletteCommands } from "../keybindings";
import { fuzzyScore } from "../editor/autocomplete";
import { blockView } from "../render/block";
import type { PageEntry, PageKind } from "../types";

// One selectable result row.
type Item =
  | { t: "page"; name: string; pageKind: PageKind }
  | { t: "create"; name: string }
  | { t: "command"; label: string; binding: string; run: () => void }
  | { t: "block"; page: string; pageKind: PageKind; blockId: string; text: string; crumb: string[] };

interface Section {
  header: string;
  items: Item[];
  // True when there are more matches than we currently show — the count badge
  // reads "N+" (not a wrong exact total) and a "Load more" row is offered.
  more?: boolean;
  // Fetch the next chunk for this section (grows its limit). Set iff `more`.
  loadMore?: () => void;
}

// Initial page size per backend-capped section, and how many more each "Load
// more" pulls in. We fetch one MORE than the current limit; getting it back
// means "there are still more" (the search stops at the limit, so this never
// walks the whole graph just to count) → show the limit, offer "Load more".
const PAGE_CAP = 12;
const BLOCK_CAP = 50;

// Ctrl-K: grouped search (Pages / Create / Commands / Blocks), command palette
// (⌘⇧P, commands-only), and recents on an empty query — mirroring OG's cmdk.
export function QuickSwitcher(): JSX.Element {
  const [query, setQuery] = createSignal("");
  const [sel, setSel] = createSignal(0);
  // How many page/block results to show right now; "Load more" grows these. They
  // reset to the base size whenever the query text changes (a fresh search).
  const [pageLimit, setPageLimit] = createSignal(PAGE_CAP);
  const [blockLimit, setBlockLimit] = createSignal(BLOCK_CAP);
  createEffect(() => {
    query();
    setPageLimit(PAGE_CAP);
    setBlockLimit(BLOCK_CAP);
  });
  let inputRef: HTMLInputElement | undefined;
  let resultsRef: HTMLDivElement | undefined;
  // X11/WebKitGTK pastes the PRIMARY selection into the focused input on ANY
  // middle-click (not cancelable from the row's mousedown). We want that paste
  // only when the user middle-clicks the input itself — so a middle-click on a
  // result row arms this, and the input's onPaste swallows the one that follows.
  let swallowNextPaste = false;
  // Scroll the highlighted row into view as the cursor moves across sections.
  createEffect(() => {
    sel();
    queueMicrotask(() =>
      resultsRef?.querySelector(".switcher-row.active")?.scrollIntoView({ block: "nearest" })
    );
  });

  const commandsOnly = () => switcherMode() === "commands";
  // The backend search/quick-switch IPC keys off a debounced query so holding
  // keys doesn't fire a whole-graph scan per character; local sections (recents,
  // commands, create-option) still react to `query()` instantly.
  const [debouncedQuery, setDebouncedQuery] = createSignal("");
  let qTimer: ReturnType<typeof setTimeout> | undefined;
  createEffect(() => {
    const q = query();
    clearTimeout(qTimer);
    qTimer = setTimeout(() => setDebouncedQuery(q), 110);
  });
  onCleanup(() => clearTimeout(qTimer));
  // Fetch limit+1 so we can tell "there are more" without counting the whole
  // graph. The +1 row is never displayed. Re-fetches when the query OR the
  // (Load-more-grown) limit changes; the early-stop scan keeps each fetch cheap.
  const [pages] = createResource(
    () => (commandsOnly() ? null : { q: debouncedQuery(), limit: pageLimit() }),
    (s) => (s && s.q.trim() ? backend().quickSwitch(s.q, s.limit + 1) : Promise.resolve([] as PageEntry[]))
  );
  const [hits] = createResource(
    () => (commandsOnly() ? null : { q: debouncedQuery(), limit: blockLimit() }),
    (s) => (s && s.q.trim() ? backend().search(s.q, s.limit + 1) : Promise.resolve([]))
  );

  const currentPageName = () => {
    const r = route();
    return r.kind === "page" ? r.name : null;
  };

  const commandItems = (q: string): Item[] => {
    const ql = q.trim();
    // Rank the command palette with the same fuzzy score as the slash menu
    // (empty query → all, in defined order); a stable sort keeps ties ordered.
    const ranked = !ql
      ? paletteCommands()
      : paletteCommands()
          .map((c) => ({ c, s: fuzzyScore(ql, c.label) }))
          .filter((x) => x.s > 0)
          .sort((a, b) => b.s - a.s)
          .map((x) => x.c);
    return ranked.map((c) => ({ t: "command" as const, label: c.label, binding: c.binding, run: c.run }));
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
    const allPages: Item[] = (pages() ?? [])
      .filter((e) => e.name.toLowerCase().includes(ql))
      .map((e) => ({ t: "page", name: e.name, pageKind: e.kind }));
    const morePages = allPages.length > pageLimit();
    const pageItems = morePages ? allPages.slice(0, pageLimit()) : allPages;
    if (pageItems.length)
      out.push({
        header: "Pages",
        items: pageItems,
        more: morePages,
        loadMore: morePages ? () => setPageLimit((n) => n + PAGE_CAP) : undefined,
      });

    // Create page (when no exact match exists).
    const exact = pageItems.some((p) => p.t === "page" && p.name.toLowerCase() === ql);
    if (!exact) out.push({ header: "Create", items: [{ t: "create", name: q }] });

    // Commands matching the query.
    const cmds = commandItems(q);
    if (cmds.length) out.push({ header: "Commands", items: cmds });

    // Blocks. Gather every match (backend order), then page the whole set so a
    // user who can't narrow the query can still reach all of it via "Load more".
    const cur = currentPageName();
    const all: { item: Item; onCur: boolean }[] = [];
    for (const g of hits() ?? []) {
      for (const b of g.blocks) {
        const text = blockView(b.raw).lines.join(" ").trim();
        if (!text) continue;
        all.push({
          item: { t: "block", page: g.page, pageKind: g.kind, blockId: b.id, text, crumb: b.breadcrumb ?? [] },
          onCur: !!(cur && g.page === cur),
        });
      }
    }
    // We asked for blockLimit + 1; getting past the limit means more remain.
    const moreBlocks = all.length > blockLimit();
    const shown = moreBlocks ? all.slice(0, blockLimit()) : all;
    // Surface current-page hits first (own section), the rest under "Blocks".
    const curItems = shown.filter((x) => x.onCur).map((x) => x.item);
    const otherItems = shown.filter((x) => !x.onCur).map((x) => x.item);
    const blockSecs: Section[] = [];
    if (curItems.length) blockSecs.push({ header: "Current page", items: curItems });
    if (otherItems.length) blockSecs.push({ header: "Blocks", items: otherItems });
    if (blockSecs.length) {
      // Hang the overflow badge + loader on the last block section, so "Load
      // more" sits at the very bottom of the results.
      const last = blockSecs[blockSecs.length - 1];
      last.more = moreBlocks;
      if (moreBlocks) last.loadMore = () => setBlockLimit((n) => n + BLOCK_CAP);
      out.push(...blockSecs);
    }

    return out;
  });

  // Flattened item list for the single highlight cursor. Every rendered row is
  // reachable — the section count badge and what you can scroll/arrow through
  // now agree (the result set is already bounded by the backend search caps).
  const flat = createMemo<Item[]>(() => sections().flatMap((s) => s.items));

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

  // Middle-click: open in a background tab and KEEP the switcher open, so you can
  // fan several results out without re-searching. A block opens zoomed into
  // itself (self-contained and durable — the tab shows exactly what you found);
  // create/command have no background-tab meaning, so they're ignored.
  const openInBackground = (it: Item) => {
    if (it.t === "page") openPageInNewTab(it.name, it.pageKind);
    else if (it.t === "block") openPageInNewTab(it.page, it.pageKind, it.blockId);
  };

  const createPage = async (name: string) => {
    try {
      await backend().savePage(
        { name, kind: "page", title: name, pre_block: null, blocks: [{ id: "", raw: "", collapsed: false, children: [] }] },
        null, // brand-new page — no baseline
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
    for (let s = 0; s < sIdx; s++) n += secs[s].items.length;
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
            onPaste={(e) => {
              // Middle-click-paste into the search bar stays allowed; only the
              // paste triggered by middle-clicking a result row is swallowed.
              if (swallowNextPaste) {
                e.preventDefault();
                swallowNextPaste = false;
              }
            }}
          />
          <div class="switcher-results" ref={resultsRef}>
            <For each={sections()}>
              {(section, sIdx) => (
                <div class="switcher-section">
                  <div class="switcher-group-header">
                    <span>{section.header}</span>
                    <span class="switcher-count">{section.items.length}{section.more ? "+" : ""}</span>
                  </div>
                  <For each={section.items}>
                    {(it, iIdx) => {
                      const idx = () => flatIndex(sIdx(), iIdx());
                      return (
                        <div
                          class="switcher-row"
                          classList={{ active: idx() === sel() }}
                          onMouseMove={() => setSel(idx())}
                          onMouseDown={(e) => {
                            // preventDefault keeps input focus (and kills the
                            // middle-click autoscroll). Left opens + closes;
                            // middle opens a background tab, switcher stays open.
                            e.preventDefault();
                            if (e.button === 1) {
                              // Swallow the PRIMARY-selection paste this click
                              // triggers into the (still-focused) search input.
                              swallowNextPaste = true;
                              setTimeout(() => (swallowNextPaste = false), 200);
                              openInBackground(it);
                            } else if (e.button === 0) choose(it);
                          }}
                        >
                          <Row item={it} query={query()} />
                        </div>
                      );
                    }}
                  </For>
                  <Show when={section.loadMore}>
                    <div
                      class="switcher-more"
                      onMouseDown={(e) => {
                        if (e.button !== 0) return;
                        e.preventDefault(); // keep input focus; don't close
                        section.loadMore!();
                      }}
                    >
                      Load more results…
                    </div>
                  </Show>
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
            <span class="switcher-page">
              {it.page}
              {it.crumb.length ? ` › ${it.crumb.join(" › ")} › ` : ": "}
            </span>
            {snippet(it.text, props.query)}
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
