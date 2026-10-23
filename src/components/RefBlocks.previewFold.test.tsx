import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { ensurePageLoaded, node, resetStore, setCollapsed, undo } from "../document";
import type { PageRead } from "../types";
import { Block } from "./Block";
import { RefBlocks } from "./RefBlocks";

afterEach(() => { resetStore(); vi.restoreAllMocks(); document.body.replaceChildren(); });
const source = (format: "md" | "org" = "md", readOnly = false): PageRead => ({
  id: `pages/Probe.${format}`, name: "Probe", title: "Probe", kind: "page", format,
  pre_block: null, read_only: readOnly,
  blocks: [{ id: "parent", raw: "Parent", collapsed: false,
    children: [{ id: "child", raw: "Child", collapsed: false, children: [] }] }],
});
function mount(dto: PageRead, outline = false) {
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <>
    <div data-preview><RefBlocks blocks={dto.blocks} page={dto.name} pageKind={dto.kind} path={dto.id} readOnly pageBody /></div>
    {outline && <div data-outline><Block id="parent" /></div>}
  </>, host);
  const preview = () => host.querySelector<HTMLElement>("[data-preview]")!;
  const click = () => preview().querySelector<HTMLElement>(".collapse-toggle")!.click();
  return { host, preview, click, dispose };
}

it.each(["md", "org"] as const)("preview folds the %s source through the document, outline follows and undo restores both", async format => {
  const dto = source(format); ensurePageLoaded(dto);
  const { host, preview, click, dispose } = mount(dto, true);
  try {
    click();
    await vi.waitFor(() => expect(node("parent")?.collapsed).toBe(true));
    expect(node("parent")?.raw).toContain(format === "md" ? "collapsed:: true" : ":collapsed: true");
    expect(preview().textContent).not.toContain("Child");
    expect(host.querySelector("[data-outline]")?.textContent).not.toContain("Child");
    undo();
    expect(node("parent")?.collapsed).toBe(false);
    expect(preview().textContent).toContain("Child");
    expect(host.querySelector("[data-outline]")?.textContent).toContain("Child");
    // Click the actual outline arrow; the still-mounted DTO is deliberately stale.
    host.querySelector<HTMLElement>("[data-outline] .collapse-toggle")!.click();
    expect(preview().textContent).not.toContain("Child");
    click();
    await vi.waitFor(() => expect(node("parent")?.collapsed).toBe(false));
    expect(preview().textContent).toContain("Child");
  } finally { dispose(); }
});

it("loads an unopened source using the reference edit path before folding", async () => {
  const dto = source();
  const get = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
  const { click, dispose } = mount(dto);
  try {
    click();
    await vi.waitFor(() => expect(node("parent")?.collapsed).toBe(true));
    expect(get).toHaveBeenCalledWith(dto.id);
  } finally { dispose(); }
});

it("shows the live source fold even when the preview DTO is stale on opening", () => {
  const dto = source(); ensurePageLoaded(dto); setCollapsed("parent", true);
  const { preview, dispose } = mount(dto);
  try { expect(preview().textContent).not.toContain("Child"); }
  finally { dispose(); }
});

it("a missing source block falls back to local folding", async () => {
  const dto = source();
  ensurePageLoaded({ ...dto, blocks: [] });
  const { preview, click, dispose } = mount(dto);
  try {
    click();
    await vi.waitFor(() => expect(preview().textContent).not.toContain("Child"));
    expect(node("parent")).toBeUndefined();
  } finally { dispose(); }
});

it("a graph switch during source loading never folds the new graph", async () => {
  const dto = source();
  let finish!: (value: PageRead) => void;
  vi.spyOn(backend(), "getPageByPath").mockReturnValue(new Promise(resolve => { finish = resolve; }));
  const { preview, click, dispose } = mount(dto);
  try {
    click();
    resetStore(); ensurePageLoaded(dto);
    finish(dto);
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(node("parent")?.collapsed).toBe(false);
    expect(preview().textContent).toContain("Child");
  } finally { dispose(); }
});

it("a read-only source keeps a reversible preview-local fold and never edits raw", async () => {
  const dto = source("md", true); ensurePageLoaded(dto);
  const { preview, click, dispose } = mount(dto);
  try {
    click();
    await vi.waitFor(() => expect(preview().textContent).not.toContain("Child"));
    expect(node("parent")?.collapsed).toBe(false);
    expect(node("parent")?.raw).toBe("Parent");
    click();
    await vi.waitFor(() => expect(preview().textContent).toContain("Child"));
  } finally { dispose(); }
});
