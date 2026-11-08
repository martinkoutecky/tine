import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { Block } from "./Block";
import { backend } from "../backend";
import type { BlockLastChanged } from "../backendTypes";
import { node, setRaw, insertSiblingAfterSubtree } from "../document";
import { RefBlocks } from "./RefBlocks";

beforeAll(() => initParser());
afterEach(() => { vi.restoreAllMocks(); resetStore(); document.body.innerHTML = ""; });
const tooltipText = (bullet: HTMLElement) => bullet.querySelector('[role="tooltip"]')?.textContent ?? "";

it("GH #831: hovering a bullet in a non-git graph says Unknown and retains gesture help", async () => {
  loadSingle({ name: "History", title: "History", kind: "page", id: "pages/History.md", format: "md", pre_block: null,
    blocks: [{ id: "a", raw: "A note", collapsed: false, children: [] }] });
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <Block id="a" />, host);
  try {
    const bullet = host.querySelector<HTMLElement>(".bullet-container")!;
    bullet.dispatchEvent(new MouseEvent("mouseenter"));
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(tooltipText(bullet)).toContain("Unknown");
    expect(tooltipText(bullet)).toContain("drag to move");
    expect(bullet.title).toBe(""); // one visible tooltip, without a second native popup
  } finally { dispose(); }
});

it.each<[BlockLastChanged, string]>([
  [{ kind: "committed", time: 1_700_086_400 }, `Last changed ${new Date(1_700_086_400_000).toLocaleString()}`],
  [{ kind: "uncommitted" }, "Changed, not yet committed"],
  [{ kind: "history_limit" }, "Unknown — history limit reached"],
])("GH #831: a hovered source block displays %j", async (changed, expected) => {
  const read = vi.spyOn(backend(), "getPageLastChanged").mockResolvedValue([{ block_path: [0], changed }]);
  loadSingle({ name: "History", title: "History", kind: "page", id: "pages/History.md", format: "md", pre_block: null,
    blocks: [{ id: "a", raw: "A note", collapsed: false, children: [] }] });
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <Block id="a" />, host);
  try {
    expect(read).not.toHaveBeenCalled();
    expect(host.querySelector('[role="tooltip"]')).toBeNull();
    const bullet = host.querySelector<HTMLElement>(".bullet-container")!;
    bullet.dispatchEvent(new MouseEvent("mouseenter"));
    await vi.waitFor(() => expect(tooltipText(bullet)).toContain(expected));
    expect(read).toHaveBeenCalledOnce();
    expect(read.mock.calls[0][0]).toBe("pages/History.md");
    expect(read.mock.calls[0][1]).toContain("A note");
    expect(tooltipText(bullet)).toContain("drag to move");
    expect(bullet.querySelectorAll('[role="tooltip"]')).toHaveLength(1);
    bullet.dispatchEvent(new MouseEvent("mouseleave"));
    expect(bullet.querySelector('[role="tooltip"]')).toBeNull();
  } finally { dispose(); }
});

it.each(["text", "outline", "reload"])("drops a date if its source changed during the read (%s)", async change => {
  let finish!: (value: { block_path: number[]; changed: BlockLastChanged }[]) => void;
  vi.spyOn(backend(), "getPageLastChanged").mockImplementation(() => new Promise(resolve => { finish = resolve; }));
  loadSingle({ name: "History", title: "History", kind: "page", id: "pages/History.md", format: "md", pre_block: null,
    blocks: [{ id: "a", raw: "A note", collapsed: false, children: [] }] });
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <Block id="a" />, host);
  try {
    const bullet = host.querySelector<HTMLElement>(".bullet-container")!;
    bullet.dispatchEvent(new MouseEvent("mouseenter"));
    if (change === "text") {
      setRaw("a", "A newer note");
      expect(node("a").raw).toBe("A newer note");
    } else if (change === "outline") {
      expect(insertSiblingAfterSubtree("a", () => {})).toBeTruthy();
    } else {
      loadSingle({ name: "History", title: "History", kind: "page", id: "pages/History.md", format: "md", pre_block: null,
        blocks: [{ id: "a", raw: "A note", collapsed: false, children: [] },
          { id: "b", raw: "An externally added block", collapsed: false, children: [] }] });
    }
    finish([{ block_path: [0], changed: { kind: "committed", time: 100 } }]);
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(tooltipText(bullet)).not.toContain("Last changed");
  } finally { dispose(); }
});

it.each([false, true])("fallback reference bullets use a current source or say Unknown (loaded=%s)", async loaded => {
  const read = vi.spyOn(backend(), "getPageLastChanged").mockResolvedValue([{ block_path: [0], changed: { kind: "committed", time: 100 } }]);
  if (loaded) loadSingle({ name: "History", title: "History", kind: "page", id: "pages/History.md", format: "md", pre_block: null,
    blocks: [{ id: "a", raw: "Current source", collapsed: false, children: [] }] });
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <RefBlocks blocks={[{ id: "a", raw: "DTO excerpt", collapsed: false, children: [] }]}
    page="History" pageKind="page" path="pages/History.md" />, host);
  try {
    expect(read).not.toHaveBeenCalled();
    const bullet = host.querySelector<HTMLElement>(".bullet-container")!;
    bullet.dispatchEvent(new MouseEvent("mouseenter"));
    await vi.waitFor(() => expect(tooltipText(bullet)).toContain(loaded ? "Last changed" : "Unknown"));
    if (loaded) expect(read.mock.calls[0][1]).toContain("Current source");
    else expect(read).not.toHaveBeenCalled();
  } finally { dispose(); }
});

it("touch input keeps the existing bullet gestures without fetching dates", () => {
  vi.spyOn(window, "matchMedia").mockImplementation(query => ({ matches: query === "(pointer: coarse)",
    media: query, onchange: null, addListener() {}, removeListener() {}, dispatchEvent() { return true; },
    addEventListener() {}, removeEventListener() {} } as MediaQueryList));
  const read = vi.spyOn(backend(), "getPageLastChanged");
  loadSingle({ name: "History", title: "History", kind: "page", id: "pages/History.md", format: "md", pre_block: null,
    blocks: [{ id: "a", raw: "A note", collapsed: false, children: [] }] });
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <Block id="a" />, host);
  try {
    const bullet = host.querySelector<HTMLElement>(".bullet-container")!;
    bullet.dispatchEvent(new MouseEvent("mouseenter"));
    expect(read).not.toHaveBeenCalled();
    expect(bullet.title).toContain("drag to move");
    expect(bullet.querySelector('[role="tooltip"]')).toBeNull();
  } finally { dispose(); }
});
