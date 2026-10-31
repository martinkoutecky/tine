import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { publishedBackend, type PublishedSnapshot } from "../publishedBackend";
import { blockWritable, isDirty, node, resetStore } from "../document";
import { editingId, endEdit } from "../editorController";
import { bumpGraphEpoch } from "../graphSession";
import { adoptResults, captureResultsTicket } from "../queryResultsEpoch";
import { initParser } from "../render/parse";
import type { BlockDto } from "../types";
import { LiveRefGroup } from "./LiveRefGroup";

const UUID = "58900000-0000-4000-8000-0000000000e1";
const block = (id: string, raw: string, children: BlockDto[] = []): BlockDto => ({ id, raw, collapsed: false, children });

beforeAll(() => initParser());
afterEach(() => {
  endEdit("page-navigation");
  resetStore();
  bumpGraphEpoch();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

async function fixture(collapsed = false) {
  const source = block(UUID, `Published parent\nid:: ${UUID}${collapsed ? "\ncollapsed:: true" : ""}`,
    [block("child", "Published child", [block("grandchild", "Published grandchild")])]);
  source.collapsed = collapsed;
  const snapshot: PublishedSnapshot = {
    schema: 1, name: "Publication", exported_at: "2026-10-31T00:00:00Z", home: "B",
    pages: [{ name: "A", title: "A", kind: "page", path: "pages/A.md", pre_block: null,
      read_only: true, blocks: [source] }],
    entries: [], backlinks: {}, block_ref_counts: {}, aliases: [], icons: {}, queries: [],
  };
  const api = publishedBackend(async () => snapshot);
  vi.spyOn(backend(), "getPageByPath").mockImplementation(api.getPageByPath);
  const group = (await api.resolveBlock(UUID))!;
  // Membership stays shallow; the source page is the authority for descendants.
  expect(group.blocks[0].children).toEqual([]);
  return { api, group };
}

it.each(["embed", "ref", "preview", "query"] as const)("hydrates a published %s from its read-only source page", async (surface) => {
  const { api, group } = await fixture();
  if (surface === "query") adoptResults(group.blocks, captureResultsTicket());
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <LiveRefGroup {...group} surface={surface} eager />, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("Published child"));
    expect(root.textContent).toContain("Published grandchild");
    expect(blockWritable(UUID)).toBe(false);
    const content = root.querySelector<HTMLElement>(".block-content")!;
    content.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 }));
    content.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, button: 0 }));
    content.click();
    expect(editingId()).toBeNull();
    expect(isDirty("A")).toBe(false);
    expect(await api.resolveBlock("private-uuid")).toBeNull();
    expect(await api.getPage("Private", "page")).toBeNull();
  } finally { dispose(); }
});

it("expands a published embedded source fold without changing the snapshot or document", async () => {
  const { group } = await fixture(true);
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <LiveRefGroup {...group} surface="embed" embedId={UUID} eager />, root);
  try {
    await vi.waitFor(() => expect(node(UUID)).toBeDefined());
    expect(root.textContent).not.toContain("Published child");
    const toggle = await vi.waitFor(() => {
      const element = root.querySelector<HTMLElement>(".collapse-toggle");
      expect(element).not.toBeNull();
      return element!;
    });
    toggle.click();
    expect(root.textContent).toContain("Published child");
    expect(root.textContent).toContain("Published grandchild");
    expect(node(UUID).collapsed).toBe(true);
    expect(node(UUID).raw).toContain("collapsed:: true");
    expect(isDirty("A")).toBe(false);
    toggle.click();
    expect(root.textContent).not.toContain("Published child");
  } finally { dispose(); }
});
