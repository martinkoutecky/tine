import { For, Show, createSignal, createResource, createEffect, createMemo, onCleanup, type JSX } from "solid-js";
import { backend } from "../backend";
import { switcherOpen, closeSwitcher, switcherMode, switcherEmbryo, recentPages, graphMeta } from "../ui";
import { openPage, openPageAtBlock, openPageInNewTab, openFile, openInNewTab, openQueryInNewTab, route } from "../router";
import { paletteCommands } from "../keybindings";
import { closePane, focusPane, openRouteInOtherPane, paneRouter } from "../panes";
import { fuzzyScore } from "../editor/autocomplete";
import { SEARCH_SYNTAX } from "../editor/searchQuery";
import { EmojiText } from "../render/emoji";
import { SearchResultRow } from "./SearchResultRow";
import type { MatchSpan, ObjectiveMatchClass, PageKind } from "../types";
import { rankLauncherItems, recordLauncherActivation } from "../launcherRanking";

// One selectable result row.
type Item =
  | { t: "page"; name: string; pageKind: PageKind; path?: string; spans?: MatchSpan[]; adaptiveClass: ObjectiveMatchClass; adaptiveIdentity: string }
  | { t: "create"; name: string }
  | { t: "command"; label: string; binding: string; run: () => void }
  | { t: "block"; page: string; pageKind: PageKind; blockId: string; text: string; crumb: string[]; spans: MatchSpan[]; adaptiveClass: ObjectiveMatchClass; adaptiveIdentity: string };

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
  const [syntaxOpen, setSyntaxOpen] = createSignal(false);
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
  const [graphResults] = createResource(
    () => (commandsOnly() ? null : { q: debouncedQuery(), pages: pageLimit(), blocks: blockLimit() }),
    (s) => s && s.q.trim()
      ? backend().runGraphSearch(s.q, s.pages + 1, s.blocks + 1, "quick-switch", false)
      : Promise.resolve({ hits: [], diagnostics: [], explanation: { branches: [] }, cancelled: false })
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
    const embryoPane = switcherEmbryo()?.paneId ?? null;
    const out: Section[] = [];

    if (commandsOnly()) {
      out.push({ header: "Commands", items: commandItems(q) });
      return out;
    }

    // Empty query → recents.
    if (!q) {
      const recents: Item[] = recentPages().map((r) => ({
        t: "page",
        name: r.name,
        pageKind: r.kind,
        adaptiveClass: "exact",
        adaptiveIdentity: `page:${r.kind}:${r.name.toLocaleLowerCase()}`,
      }));
      if (recents.length) out.push({ header: "Recent", items: recents });
      return out;
    }

    const ql = q.toLowerCase();
    // Page and block membership, diagnostics, and match evidence now come from
    // one Rust QueryPlan execution. Commands/create remain launcher providers.
    const allPages: Item[] = (graphResults()?.hits ?? [])
      .filter((hit) => hit.entity === "page")
      .map((hit) => ({
        t: "page",
        name: hit.page.name,
        pageKind: hit.page.kind,
        path: hit.page.path,
        spans: hit.evidence.flatMap((evidence) => evidence.spans),
        adaptiveClass: hit.match_class ?? "substring",
        adaptiveIdentity: `page:${hit.page.kind}:${hit.page.path || hit.page.name.toLocaleLowerCase()}`,
      }));
    const rankedPages = rankLauncherItems(
      graphMeta()?.root ?? "",
      q,
      allPages as Extract<Item, { t: "page" }>[],
    );
    const morePages = rankedPages.length > pageLimit();
    const pageItems = morePages ? rankedPages.slice(0, pageLimit()) : rankedPages;
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
    const cmds = embryoPane ? [] : commandItems(q);
    if (cmds.length) out.push({ header: "Commands", items: cmds });

    // Blocks. Gather every match (backend order), then page the whole set so a
    // user who can't narrow the query can still reach all of it via "Load more".
    const cur = currentPageName();
    const all: { item: Item; onCur: boolean }[] = [];
    for (const hit of graphResults()?.hits ?? []) {
      if (hit.entity !== "block" || !hit.display_text.trim()) continue;
      all.push({
        item: {
          t: "block",
          page: hit.page,
          pageKind: hit.kind,
          blockId: hit.block.id,
          text: hit.display_text,
          crumb: hit.block.breadcrumb ?? [],
          spans: hit.evidence.flatMap((evidence) => evidence.spans),
          adaptiveClass: hit.match_class ?? "body_evidence",
          adaptiveIdentity: `block:${hit.kind}:${hit.page.toLocaleLowerCase()}:${hit.block.id}`,
        },
        onCur: !!(cur && hit.page === cur),
      });
    }
    // We asked for blockLimit + 1; getting past the limit means more remain.
    const rankedBlocks = rankLauncherItems(
      graphMeta()?.root ?? "",
      q,
      all.map((entry) => entry.item).filter((item): item is Extract<Item, { t: "block" }> => item.t === "block"),
    );
    const onCurrent = new Map(all.map((entry) => [
      entry.item.t === "block" ? entry.item.adaptiveIdentity : "",
      entry.onCur,
    ]));
    const ranked = rankedBlocks.map((item) => ({
      item,
      onCur: onCurrent.get(item.adaptiveIdentity) ?? false,
    }));
    const moreBlocks = ranked.length > blockLimit();
    const shown = moreBlocks ? ranked.slice(0, blockLimit()) : ranked;
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
      setQuery(switcherEmbryo()?.prefill ?? "");
      setSel(0);
      setSyntaxOpen(false);
      queueMicrotask(() => inputRef?.focus());
    }
  });
  // Keep the highlight in range as results change.
  createEffect(() => {
    const n = flat().length;
    if (sel() >= n) setSel(0);
  });

  const choose = (it: Item) => {
    const embryo = switcherEmbryo();
    if (embryo) {
      void chooseEmbryo(it, embryo.paneId);
      return;
    }
    recordChoice(it);
    switch (it.t) {
      case "page":
        it.path ? openFile(it.path, it.name, it.pageKind) : openPage(it.name, it.pageKind);
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

  const recordChoice = (it: Item) => {
    if (it.t !== "page" && it.t !== "block") return;
    recordLauncherActivation(graphMeta()?.root ?? "", query(), it.adaptiveIdentity);
  };

  const chooseEmbryo = async (it: Item, paneId: string) => {
    recordChoice(it);
    const router = paneRouter(paneId);
    switch (it.t) {
      case "page":
        it.path ? router.openFile(it.path, it.name, it.pageKind) : router.openPage(it.name, it.pageKind);
        break;
      case "create":
        await createPageFile(it.name);
        router.openPage(it.name, "page");
        break;
      case "block":
        router.openPageAtBlock(it.page, it.pageKind, it.blockId);
        break;
      case "command":
        it.run();
        break;
    }
    focusPane(paneId);
    closeSwitcher();
  };

  const chooseOther = async (it: Item) => {
    recordChoice(it);
    switch (it.t) {
      case "page":
        openRouteInOtherPane({ kind: "page", name: it.name, pageKind: it.pageKind, path: it.path });
        break;
      case "create":
        await createPageFile(it.name);
        openRouteInOtherPane({ kind: "page", name: it.name, pageKind: "page" });
        break;
      case "command":
        it.run();
        break;
      case "block":
        openRouteInOtherPane({ kind: "page", name: it.page, pageKind: it.pageKind, block: it.blockId });
        break;
    }
    closeSwitcher();
  };

  // Middle-click: open in a background tab and KEEP the switcher open, so you can
  // fan several results out without re-searching. A block opens zoomed into
  // itself (self-contained and durable — the tab shows exactly what you found);
  // create/command have no background-tab meaning, so they're ignored.
  const openInBackground = (it: Item) => {
    recordChoice(it);
    if (it.t === "page")
      it.path
        ? openInNewTab({ kind: "page", name: it.name, pageKind: it.pageKind, path: it.path })
        : openPageInNewTab(it.name, it.pageKind);
    else if (it.t === "block") openPageInNewTab(it.page, it.pageKind, it.blockId);
  };

  const createPageFile = async (name: string) => {
    try {
      await backend().savePage(
        { name, kind: "page", title: name, pre_block: null, blocks: [{ id: "", raw: "", collapsed: false, children: [] }] },
        null, // brand-new page — no baseline
        false
      );
    } catch {
      // ignore — still navigate; the page will be created on first edit
    }
  };

  const createPage = async (name: string) => {
    await createPageFile(name);
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
      if (it) {
        if (e.altKey && !switcherEmbryo()) void chooseOther(it);
        else choose(it);
      }
    } else if (e.key === "Escape") {
      e.preventDefault();
      if (syntaxOpen()) {
        setSyntaxOpen(false);
        queueMicrotask(() => inputRef?.focus());
      } else {
        cancelSwitcher();
      }
    }
  };

  const cancelSwitcher = () => {
    const embryo = switcherEmbryo();
    closeSwitcher();
    if (embryo) closePane(embryo.paneId);
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
      <div class="switcher-overlay" onClick={cancelSwitcher}>
        <div class="switcher" onClick={(e) => e.stopPropagation()}>
          <input
            ref={inputRef}
            class="switcher-input"
            type="text"
            placeholder={commandsOnly() ? "Run a command…" : "Jump to page, search, or run a command…"}
            value={query()}
            role="combobox"
            aria-autocomplete="list"
            aria-expanded="true"
            aria-controls="switcher-results"
            aria-activedescendant={flat().length ? `switcher-option-${sel()}` : undefined}
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
          <div id="switcher-results" class="switcher-results" role="listbox" ref={resultsRef}>
            <For each={sections()}>
              {(section, sIdx) => (
                <div class="switcher-section" role="group" aria-labelledby={`switcher-group-${sIdx()}`}>
                  <div id={`switcher-group-${sIdx()}`} class="switcher-group-header">
                    <span>{section.header}</span>
                    <span class="switcher-count">{section.items.length}{section.more ? "+" : ""}</span>
                  </div>
                  <For each={section.items}>
                    {(it, iIdx) => {
                      const idx = () => flatIndex(sIdx(), iIdx());
                      return (
                        <div
                          class="switcher-row"
                          classList={{ active: idx() === sel(), "block-result": it.t === "block" }}
                          id={`switcher-option-${idx()}`}
                          role="option"
                          aria-selected={idx() === sel()}
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
                          <Row item={it} />
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
            <Show when={(graphResults()?.diagnostics.length ?? 0) > 0}>
              <div class="switcher-empty switcher-error">
                {graphResults()!.diagnostics.map((diagnostic) => diagnostic.message).join(" · ")}
              </div>
            </Show>
            <Show when={query().trim() && flat().length === 0 && !(graphResults()?.diagnostics.length ?? 0)}>
              <div class="switcher-empty">No matched results</div>
            </Show>
          </div>
          <Show when={syntaxOpen() && !commandsOnly()}>
            <div id="switcher-search-syntax" class="switcher-syntax" role="region" aria-label="Search syntax">
              <For each={SEARCH_SYNTAX}>
                {(rule) => (
                  <div class="switcher-syntax-row">
                    <code>{rule.example}</code>
                    <span>{rule.description}</span>
                  </div>
                )}
              </For>
            </div>
          </Show>
          <div class="switcher-footer">
            <span><kbd>↑↓</kbd> navigate</span>
            <span><kbd>↵</kbd> open</span>
            <span><kbd>⌘⇧P</kbd> commands</span>
            <Show when={!commandsOnly()}>
              <button
                type="button"
                class="switcher-syntax-toggle"
                aria-expanded={syntaxOpen()}
                aria-controls="switcher-search-syntax"
                onClick={() => setSyntaxOpen((open) => !open)}
              >
                Search syntax
              </button>
              <button
                type="button"
                class="switcher-syntax-toggle"
                data-open-search-tab
                disabled={!query().trim() || !!graphResults()?.diagnostics.length}
                onClick={() => {
                  openQueryInNewTab(query().trim(), "search", true);
                  closeSwitcher();
                }}
              >
                Open results in tab
              </button>
            </Show>
          </div>
        </div>
      </div>
    </Show>
  );
}

// A single result row body, with an icon tag, label, and (for blocks) a
// highlighted snippet windowed around the match.
function Row(props: { item: Item }): JSX.Element {
  const it = props.item;
  switch (it.t) {
    case "page":
      return (
        <>
          <span class="switcher-kind">{it.pageKind === "journal" ? "journal" : "page"}</span>
          <span class="switcher-name"><EmojiText text={it.name} /></span>
        </>
      );
    case "create":
      return (
        <>
          <span class="switcher-kind create">new</span>
          <span class="switcher-name">Create page: <strong><EmojiText text={it.name} /></strong></span>
        </>
      );
    case "command":
      return (
        <>
          <span class="switcher-kind cmd">cmd</span>
          <span class="switcher-name"><EmojiText text={it.label} /></span>
          <Show when={it.binding}>
            <span class="switcher-shortcut">{it.binding}</span>
          </Show>
        </>
      );
    case "block":
      return (
        <SearchResultRow page={it.page} breadcrumb={it.crumb} text={it.text} spans={it.spans} />
      );
  }
}
