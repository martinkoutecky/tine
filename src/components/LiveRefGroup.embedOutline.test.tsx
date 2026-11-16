import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { endEdit, editingSurface } from "../editorController";
import { beginPageHeaderEdit, indentBlock, outdentBlock, resetStore, undo } from "../document";
import { loadFeed } from "../document/workingSet";
import { pageToDto } from "../document/convert";
import { initParser } from "../render/parse";
import { maybeEditorIn } from "../tests/editorDriver";
import { Block, SurfaceContext } from "./Block";
import { LiveRefGroup } from "./LiveRefGroup";
import { EmbedMacro } from "./EmbedMacro";
import { PeekContext } from "../render/PeekPopup";
import type { BlockDto, PageRead } from "../types";

beforeAll(initParser);
afterEach(() => {
  endEdit("page-navigation"); resetStore(); vi.restoreAllMocks(); document.body.replaceChildren();
});
const leaf = (id: string, raw: string, children: BlockDto[] = [], collapsed = false): BlockDto =>
  ({ id, raw, children, collapsed });
const page = (name: string, blocks: BlockDto[]): PageRead => ({ id: `pages/${name}.md`, name, title: name,
  kind: "page", format: "md", pre_block: null, blocks });
const source = () => page("Source", [
  leaf("trial", "Survey trial", [leaf("trial-child", "Trial child")]),
  leaf("review", "Review together", [leaf("todo", "TODO Prepare questions"), leaf("later", "LATER Review answers")]),
  leaf("questions", "Open questions", [leaf("question-child", "Hidden question")], true),
  leaf("definitions", "Definitions", [leaf("definition-child", "Hidden definition")], true),
]);
function outline(root: Element) {
  return [...root.querySelectorAll<HTMLElement>(".ls-block")].filter(row => !row.closest(".block-references")).map(row => {
    let depth = 0, ancestor = row.parentElement?.closest(".ls-block");
    while (ancestor && root.contains(ancestor)) { depth++; ancestor = ancestor.parentElement?.closest(".ls-block"); }
    return { id: row.dataset.blockId, depth, collapsed: row.classList.contains("collapsed"), readOnly: row.classList.contains("ref-block") };
  });
}
function sourceOutline() {
  const rows: ReturnType<typeof outline> = [];
  const visit = (block: BlockDto, depth: number) => {
    rows.push({ id: block.id, depth, collapsed: block.collapsed, readOnly: false });
    if (!block.collapsed) block.children.forEach(child => visit(child, depth + 1));
  };
  pageToDto("Source")!.blocks.forEach(block => visit(block, 0));
  return rows;
}
async function mount(surface: string, kind: "page" | "block" = "page", withHeader = false) {
  const dto = source();
  if (withHeader) dto.pre_block = "tags:: Example\n";
  if (kind === "block") dto.blocks = [leaf("source-tree", "Shared outline\nid:: source-root", dto.blocks)];
  const host = page("Host", [leaf("host", kind === "page" ? "{{embed [[Source]]}}" : "{{embed ((source-root))}}")]);
  loadFeed([host, dto]);
  vi.spyOn(backend(), "getPage").mockImplementation(async name => name === "Source" ? { ...pageToDto("Source")!, id: dto.id } : null);
  vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([{ page: "Source", kind: "page", path: dto.id,
    blocks: [{ ...dto.blocks[0], children: [] }] }]);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => surface.startsWith("ref:")
    ? <LiveRefGroup page="Host" kind="page" path={host.id} blocks={host.blocks} surface="ref" eager />
    : <SurfaceContext.Provider value={surface}><Block id="host" /></SurfaceContext.Provider>, root);
  await vi.waitFor(() => expect(root.querySelector('.embed-block [data-block-id="todo"]:not(.ref-block)')).not.toBeNull());
  const embed = root.querySelector(".embed-block")!;
  const content = embed.querySelector('[data-block-id="todo"] > .block-main .block-content')!;
  content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
  document.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
  await vi.waitFor(() => expect(maybeEditorIn(embed as HTMLElement, "todo")).not.toBeNull());
  return { root, embed, dispose, surface: editingSurface() };
}
for (const surface of ["main", "sidebar:example", "ref:example"]) for (const kind of ["page", "block"] as const) {
  it(`${surface}: a ${kind} embed follows the source during a sibling structural edit and undo`, async () => {
    const { embed, dispose, surface: editSurface } = await mount(surface, kind);
    try {
      const before = outline(embed);
      const depth = kind === "block" ? 1 : 0;
      // The retained membership DTO still lists review as a root while its
      // live node is now trial's child. A transclusion must render it once.
      indentBlock("review", 0, editSurface);
      expect(outline(embed)).toEqual(sourceOutline());
      expect(outline(embed).filter(row => row.id === "review")).toEqual([
        { id: "review", depth: depth + 1, collapsed: false, readOnly: false },
      ]);
      expect(outline(embed).filter(row => row.id === "definitions")).toEqual([
        { id: "definitions", depth, collapsed: true, readOnly: false },
      ]);
      expect(embed.textContent).not.toContain("Hidden definition");
      undo(); expect(outline(embed)).toEqual(before);
      outdentBlock("todo", 0, editSurface);
      expect(outline(embed)).toEqual(sourceOutline());
      expect(outline(embed).filter(row => row.id === "todo")).toEqual([
        { id: "todo", depth, collapsed: false, readOnly: false },
      ]);
      endEdit("page-navigation");
      expect(outline(embed).filter(row => row.id === "todo")).toHaveLength(1);
      undo(); expect(outline(embed)).toEqual(before);
    } finally { dispose(); }
  });
}

it("a source's temporary page-header editor does not become an embedded outline row", async () => {
  const { embed, dispose } = await mount("main", "page", true);
  try {
    const before = outline(embed);
    endEdit("page-navigation");
    expect(beginPageHeaderEdit("Source")).toBeTruthy();
    expect(outline(embed)).toEqual(before);
    expect(embed.textContent).not.toContain("tags:");
  } finally { dispose(); }
});

it("an embed's read-only copy preserves source folds while hydration is unavailable", async () => {
  const getPage = vi.spyOn(backend(), "getPage").mockResolvedValue(null);
  const root = document.createElement("div"); document.body.append(root);
  const rows = [leaf("folded", "Definitions", [leaf("hidden", "Hidden definition")], true)];
  const dispose = render(() => <LiveRefGroup page="Unavailable" kind="page" blocks={rows} surface="embed" eager />, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("Definitions"));
    expect(root.textContent).not.toContain("Hidden definition");
    expect(outline(root)).toEqual([{ id: "folded", depth: 0, collapsed: true, readOnly: true }]);
    expect(root.querySelector(".collapse-toggle.has-children")).not.toBeNull();
    await vi.waitFor(() => expect(getPage).toHaveBeenCalledTimes(1));
    root.querySelector<HTMLElement>(".collapse-toggle.has-children")!.click();
    expect(root.textContent).toContain("Hidden definition");
    expect(getPage).toHaveBeenCalledTimes(1); // local presentation, no source admission/write
  } finally { dispose(); }
});

it("an embed inside a hover preview preserves the same folded source outline", async () => {
  vi.spyOn(backend(), "getPage").mockResolvedValue(source());
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <PeekContext.Provider value={true}><EmbedMacro body="embed [[Source]]" /></PeekContext.Provider>, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("Definitions"));
    expect(root.textContent).not.toContain("Hidden definition");
    expect(root.querySelector('[data-block-id="definitions"]')?.classList.contains("collapsed")).toBe(true);
    expect(root.querySelector('[data-block-id="definitions"] .collapse-toggle.has-children')).not.toBeNull();
  } finally { dispose(); }
});
