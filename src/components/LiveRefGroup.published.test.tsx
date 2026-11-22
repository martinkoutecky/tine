import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { testPublishedBackend } from "../publishedBackend.methods.test.support";
import { blockWritable, isDirty, node, resetStore } from "../document";
import { editingId, endEdit } from "../editorController";
import { bumpGraphEpoch } from "../graphSession";
import { adoptResults, captureResultsTicket } from "../queryResultsEpoch";
import { initParser } from "../render/parse";
import { LiveRefGroup } from "./LiveRefGroup";

const UUID = "58900000-0000-4000-8000-0000000000e1";

beforeAll(() => initParser());
afterEach(() => {
  endEdit("page-navigation");
  resetStore();
  bumpGraphEpoch();
  vi.restoreAllMocks();
  document.body.replaceChildren();
});

async function fixture(collapsed = false) {
  const api = testPublishedBackend({
    "pages/A.md": `- Published parent\n  id:: ${UUID}${collapsed ? "\n  collapsed:: true" : ""}\n  - Published child\n    - Published grandchild\n`,
  });
  vi.spyOn(backend(), "getPageByPath").mockImplementation(api.getPageByPath);
  vi.spyOn(backend(), "getPage").mockImplementation(api.getPage);
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
  const id = group.blocks[0].id;
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <LiveRefGroup {...group} surface="embed" embedId={UUID} eager />, root);
  try {
    await vi.waitFor(() => expect(node(id)).toBeDefined());
    expect(root.textContent).not.toContain("Published child");
    const toggle = await vi.waitFor(() => {
      const element = root.querySelector<HTMLElement>(".collapse-toggle");
      expect(element).not.toBeNull();
      return element!;
    });
    toggle.click();
    expect(root.textContent).toContain("Published child");
    expect(root.textContent).toContain("Published grandchild");
    expect(node(id).collapsed).toBe(true);
    expect(node(id).raw).toContain("collapsed:: true");
    expect(isDirty("A")).toBe(false);
    toggle.click();
    expect(root.textContent).not.toContain("Published child");
  } finally { dispose(); }
});
