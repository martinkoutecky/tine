// GH #943 / #941: every selected parent copies with its whole subtree, whether
// expanded, folded or shown as a Sheet view (D-20261112-SU10).
// Assertions are on the public clipboard text (what a plain-text paste receives).
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { ContextMenu } from "./components/ContextMenu";
import { backend } from "./backend";
import { initParser } from "./render/parse";
import { loadSingle } from "./document/workingSet";
import { extendSelectionTo, resetStore, selectBlock, selectionMarkdown } from "./document";
import { clearClipboardPayload, peekClipboardPayload } from "./clipboard";
import { openContextMenu, closeContextMenu, closeExportModal } from "./ui";
import { setGraphMeta } from "./graphSession";
import { clearTransientLayersForTest } from "./transientLayers";
import { installKeybindings } from "./keybindings";
import { setCopyStripCollapsed } from "./copySettings";
import type { BlockDto } from "./types";
import { bindTestHost } from "./document/host/wiring.test.support";

const block = (id: string, raw: string, children: BlockDto[] = [], collapsed = false): BlockDto => ({ id, raw, children, collapsed });

// The issue's outline: A, B (collapsed, child C), D. C has an expanded child E, and
// a collapsed child F with its own child G (nested collapse inside the hidden subtree).
function load(bCollapsed = true) {
  loadSingle({
    name: "P", kind: "page", title: "P", format: "md", pre_block: null,
    blocks: [
      block("A", "A"),
      block("B", bCollapsed ? "B\ncollapsed:: true" : "B", [
        block("C", "C", [block("E", "E"), block("F", "F\ncollapsed:: true", [block("G", "G")], true)]),
      ], bCollapsed),
      block("D", "D"),
    ],
  });
  setGraphMeta({ root: "/fixture" } as any);
}

const WHOLE_B_STRIPPED = "- B\n\t- C\n\t\t- E\n\t\t- F\n\t\t\t- G";
const WHOLE_B_KEPT = "- B\n  collapsed:: true\n\t- C\n\t\t- E\n\t\t- F\n\t\t  collapsed:: true\n\t\t\t- G";

let dispose: (() => void) | undefined;
let disposeKeys: (() => void) | undefined;
beforeAll(() => initParser());
beforeEach(async () => {
  await bindTestHost();
  vi.spyOn(backend(), "writeRich").mockResolvedValue();
  vi.spyOn(backend(), "writeText").mockResolvedValue();
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) => ids.map(() => null));
  setCopyStripCollapsed(true);
});
afterEach(() => {
  dispose?.(); dispose = undefined;
  disposeKeys?.(); disposeKeys = undefined;
  closeContextMenu(); closeExportModal(); clearClipboardPayload(); resetStore();
  setGraphMeta(null); clearTransientLayersForTest(); vi.restoreAllMocks();
  setCopyStripCollapsed(true);
  document.body.innerHTML = "";
});

function clipboardText(): string {
  return vi.mocked(backend().writeRich).mock.calls.at(-1)![0];
}

async function ctrlC(): Promise<string> {
  disposeKeys = installKeybindings();
  window.dispatchEvent(new KeyboardEvent("keydown", { key: "c", code: "KeyC", ctrlKey: true, bubbles: true }));
  await vi.waitFor(() => expect(backend().writeRich).toHaveBeenCalled());
  return clipboardText();
}

async function menuCopy(on: string): Promise<string> {
  const root = document.createElement("div"); document.body.append(root);
  dispose = render(() => <ContextMenu />, root);
  openContextMenu(10, 10, on);
  const item = [...document.querySelectorAll<HTMLElement>(".ctx-item")].find(el => /^Copy blocks?$/.test(el.textContent?.trim() ?? ""));
  expect(item).toBeDefined();
  item!.click();
  await vi.waitFor(() => expect(backend().writeRich).toHaveBeenCalled());
  return clipboardText();
}

type Entry = { name: string; copy: () => Promise<string> };
const select = (from: string, to = from) => { selectBlock(from); if (to !== from) extendSelectionTo(to); };

describe("GH #943: a collapsed block copies with the children it hides", () => {
  const entries: Entry[] = [
    { name: "Ctrl+C, collapsed B alone", copy: () => { select("B"); return ctrlC(); } },
    { name: "block menu Copy, collapsed B alone", copy: () => { select("B"); return menuCopy("B"); } },
    { name: "block menu Copy on an unselected collapsed B", copy: () => menuCopy("B") },
  ];
  for (const entry of entries) {
    it(`${entry.name}: default copies B's whole hidden subtree, nested collapse included`, async () => {
      load();
      expect(await entry.copy()).toBe(WHOLE_B_STRIPPED);
    });
    it(`${entry.name}: an expanded parent also copies the whole subtree`, async () => {
      load(false);
      expect(await entry.copy()).toBe(WHOLE_B_STRIPPED);
    });
    it(`${entry.name}: collapsed:: follows the strip setting`, async () => {
      setCopyStripCollapsed(false);
      load();
      expect(await entry.copy()).toBe(WHOLE_B_KEPT);
    });
  }

  it.each([
    ["Ctrl+C", () => ctrlC()],
    ["block menu Copy", () => menuCopy("B")],
  ])("%s on A through D (the issue's shift-click range) keeps C under B", async (_name, copy) => {
    load();
    select("A", "D");
    expect(await copy()).toBe(`- A\n${WHOLE_B_STRIPPED}\n- D`);
  });

  it.each([
    ["Ctrl+C", () => ctrlC()],
    ["block menu Copy", () => menuCopy("B")],
  ])("%s: expanded A through D copies the same complete forest", async (_name, copy) => {
    load(false);
    select("A", "D");
    expect(await copy()).toBe(`- A\n${WHOLE_B_STRIPPED}\n- D`);
  });

  it("an expanded B selected alone copies its descendants (Ctrl+C)", async () => {
    load(false);
    select("B");
    expect(await ctrlC()).toBe(WHOLE_B_STRIPPED);
  });

  it("an expanded B and its visible children up to C: the folded F inside carries G", async () => {
    load(false);
    // B expanded: the visible rows are A B C E F D (F is collapsed, G hidden).
    select("B", "F");
    expect(selectionMarkdown()).toBe(WHOLE_B_STRIPPED);
  });

  it("an expanded B through C completes all descendants of both endpoints", async () => {
    load(false);
    select("B", "C");
    expect(selectionMarkdown()).toBe(WHOLE_B_STRIPPED);
  });

  it("block-menu Copy of an expanded parent includes its subtree", async () => {
    load(false);
    select("B");
    expect(await menuCopy("B")).toBe(WHOLE_B_STRIPPED);
  });

  it("a zoom that force-expands collapsed B: B's visible children are not hidden by it", () => {
    load();
    // Zoomed into B: B's own fold is overridden for this view, so its children show.
    selectBlock("B", { roots: ["B"], forceExpandedRoot: "B" });
    expect(selectionMarkdown()).toBe(WHOLE_B_STRIPPED);
  });

  it("a secondary surface that folds an expanded block: its hidden children are copied", () => {
    load(false);
    selectBlock("B", { roots: ["B"], collapsed: (id, stored) => id === "B" ? true : stored });
    expect(selectionMarkdown()).toBe(WHOLE_B_STRIPPED);
  });

  // A Sheet view (grid/table/board) shows its host's children as cells, not rows, so
  // they cannot be selected either: the same silent loss, the same repair.
  function loadSheet(view: "grid" | "table" | "board") {
    loadSingle({
      name: "S", kind: "page", title: "S", format: "md", pre_block: null,
      blocks: [
        // The backend ships header facets with each block (BlockDto.properties); the view reads them.
        { ...block("H", `Sheet\ntine.view:: ${view}`, [block("R1", "row one", [block("R1a", "cell")]), block("R2", "row two")]), properties: [["tine.view", view]] },
        block("Z", "after"),
      ],
    });
    setGraphMeta({ root: "/fixture" } as any);
  }
  const SHEET = (view: string) => `- Sheet\n  tine.view:: ${view}\n\t- row one\n\t\t- cell\n\t- row two`;

  it.each(["grid", "table", "board"] as const)("Ctrl+C on a selected %s-view block copies the rows its view hides", async (view) => {
    loadSheet(view);
    select("H");
    expect(await ctrlC()).toBe(SHEET(view));
  });

  it("block menu Copy on a Sheet host and the next block keeps the rows", async () => {
    loadSheet("grid");
    select("H", "Z");
    expect(await menuCopy("H")).toBe(`${SHEET("grid")}\n- after`);
  });

  it("block-menu Copy of a Sheet host includes all rows", async () => {
    loadSheet("grid");
    select("H");
    expect(await menuCopy("H")).toBe(SHEET("grid"));
  });

  it("the in-app paste payload agrees with the text for a collapsed block", async () => {
    load();
    select("B");
    await ctrlC();
    const tree = (b: { raw: string; children: any[] }): string[] => [b.raw.split("\n")[0], ...b.children.flatMap(tree)];
    expect(peekClipboardPayload()!.blocks.flatMap(tree)).toEqual(["B", "C", "E", "F", "G"]);
  });
});
