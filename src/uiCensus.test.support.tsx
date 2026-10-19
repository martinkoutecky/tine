// Shared helpers for the UI census budget tests (src/uiCensus*.budget.test.tsx).
// Not a test file. The mobile test sets its platform in vi.hoisted, which runs
// before this module (and every other app module) is evaluated.
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { ContextMenu } from "./components/ContextMenu";
import { doc, setDoc, type Node as StoreNode } from "./document/model";
import { clearSelection, extendSelectionTo, resetStore, selectBlock, setPageProperty } from "./document";
import { closeContextMenu, openBlockRefContextMenu, openContextMenu, openPageContextMenu } from "./ui";
import type { OwnedOccurrence } from "./refOccurrence";
import { activeDocument, bodyOf, createRangeIn, selectionOf } from "./windowRealm";

export const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/** Re-read until the value stops changing: async loads (platform probe, lists)
 *  must land before a surface is counted, or the budget would flake. */
export async function settled<T>(read: () => T): Promise<T> {
  let last = JSON.stringify(read());
  for (let stable = 0, spins = 0; stable < 3; spins++) {
    if (spins > 200) throw new Error("UI census: a surface never settled");
    await sleep(15);
    const next = JSON.stringify(read());
    stable = next === last ? stable + 1 : 0;
    last = next;
  }
  return read();
}

export function mount(view: () => JSX.Element): { root: HTMLElement; dispose: () => void } {
  const root = activeDocument().createElement("div");
  bodyOf().append(root);
  const dispose = render(view, root);
  return { root, dispose: () => { dispose(); root.remove(); } };
}

export const CONTROLS = "input:not([type=hidden]), select, textarea, button, summary, a[href]";
export const controlsIn = (scope: ParentNode, selector = CONTROLS) => [...scope.querySelectorAll<HTMLElement>(selector)];
export const describeControl = (el: Element) =>
  `${el.tagName.toLowerCase()}:${el.getAttribute("aria-label") ?? el.getAttribute("title") ?? el.textContent?.trim().slice(0, 24) ?? ""}`;

/** Reach is part of the measurement: a surface is counted only if its anchor exists. */
export function required<T extends Element>(found: T | null | undefined, what: string): T {
  if (!found) throw new Error(`UI census: ${what} was not reached`);
  return found;
}

/** Top-level rows of an open context menu (submenu entries counted apart). */
export function menuRows(menu: HTMLElement) {
  const items = controlsIn(menu, ".ctx-item");
  const top = items.filter((el) => !el.closest(".ctx-submenu-menu"));
  return { top, nested: items.length - top.length };
}

/** Select the words "Hello" in a rendered block, the way a reader would before
 *  right-clicking, so the menu's "Comment on selection" row is reachable. */
export function selectRenderedWords(blockId: string): () => void {
  const row = activeDocument().createElement("div");
  row.setAttribute("data-block-id", blockId);
  const content = activeDocument().createElement("div");
  content.className = "block-content";
  content.textContent = "Hello world";
  row.append(content);
  bodyOf().append(row);
  const range = createRangeIn();
  range.setStart(content.firstChild!, 0);
  range.setEnd(content.firstChild!, 5);
  const selection = selectionOf()!;
  selection.removeAllRanges();
  selection.addRange(range);
  return () => { selection.removeAllRanges(); row.remove(); };
}

// ---- Context menus: the same measurement on both platforms ----------------
const node = (id: string, raw: string, parent: string | null, children: string[]): StoreNode => ({ id, raw, collapsed: false, parent, page: "P", children });
function loadMenuFixtures(kind: "page" | "journal", readOnly = false) {
  const name = kind === "journal" ? "2000-01-01" : "P";
  setDoc({
    byId: {
      leaf: node("leaf", "Leaf", null, []),
      leaf2: node("leaf2", "Second leaf", null, []),
      // Every conditional block-menu row at once: children (Show children as),
      // a template property (Use for new journals), a trailing pipe table
      // (Convert to grid); the test adds a rendered text selection (Comment on selection).
      full: node("full", "Hello world\ntemplate:: tpl\n| a | b |\n|---|---|\n| 1 | 2 |", null, ["kid"]),
      kid: node("kid", "Kid", "full", []),
    },
    pages: [{ name, kind, title: name, preBlock: null, roots: ["leaf", "leaf2", "full"], format: "md", readOnly, guide: false }],
    feed: [name],
    loaded: true,
  });
  for (const id of Object.keys(doc.byId)) setDoc("byId", id, "page", name);
}
export function measureContextMenus(counts: Record<string, number>, labels: Record<string, string[]>, prefix: string, withButtonRows = true) {
  const { root, dispose } = mount(() => <ContextMenu />);
  const menu = () => required(root.querySelector<HTMLElement>(".ctx-menu"), "a context menu");
  const record = (key: string, nestedKey?: string) => {
    const { top, nested } = menuRows(menu());
    counts[key] = top.length;
    labels[key] = top.map((el) => el.textContent?.trim() ?? "");
    if (nestedKey) counts[nestedKey] = nested;
  };
  try {
    // Block bullet menu.
    loadMenuFixtures("page");
    openContextMenu(1, 1, "leaf");
    record(`${prefix}.block.plain`);
    if (withButtonRows) {
      counts[`${prefix}.block.colors`] = menu().querySelectorAll(".ctx-color").length;
      counts[`${prefix}.block.headings`] = menu().querySelectorAll(".ctx-h").length;
    }
    closeContextMenu();
    const unselect = selectRenderedWords("full");
    openContextMenu(1, 1, "full");
    record(`${prefix}.block.full`, `${prefix}.block.full.nested`);
    for (const row of ["Comment on selection", "Properties…", "Convert to grid", "Use for new journals", "Show children as →", "Make a template…"]) {
      if (!labels[`${prefix}.block.full`].some((label) => label.startsWith(row))) throw new Error(`UI census: the full block menu fixture did not reach "${row}"`);
    }
    unselect();
    closeContextMenu();
    selectBlock("leaf");
    extendSelectionTo("leaf2");
    openContextMenu(1, 1, "leaf");
    record(`${prefix}.block.multi`);
    closeContextMenu();
    clearSelection();
    loadMenuFixtures("page", true);
    openContextMenu(1, 1, "leaf");
    record(`${prefix}.block.readonly`);
    closeContextMenu();
    // Page menu: title/link entry points (no file actions) and the page header.
    loadMenuFixtures("page");
    setPageProperty("P", "tine.icon", "AB");
    openPageContextMenu(1, 1, "P", "page", false);
    record(`${prefix}.page.link`);
    closeContextMenu();
    openPageContextMenu(1, 1, "P", "page", true);
    record(`${prefix}.page.header.page`, `${prefix}.page.header.nested`);
    closeContextMenu();
    loadMenuFixtures("journal");
    setPageProperty("2000-01-01", "tine.icon", "AB");
    openPageContextMenu(1, 1, "2000-01-01", "journal", true);
    record(`${prefix}.page.header.journal`);
    closeContextMenu();
    // Block reference menu, with the occurrence rows (edit / remove reference).
    openBlockRefContextMenu(1, 1, "leaf", "P", "page", undefined, {} as OwnedOccurrence);
    record(`${prefix}.blockref`);
    closeContextMenu();
  } finally { dispose(); closeContextMenu(); clearSelection(); resetStore(); }
}

