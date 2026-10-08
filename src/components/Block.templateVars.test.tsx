import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { pageByName, resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { doc } from "../document/model";
import type { GraphMeta, PageDto } from "../types";
import { bumpGraphEpoch, setGraphMeta } from "../graphSession";
import { Block } from "./Block";

const META: GraphMeta = {
  root: "/tmp/template-graph", journals_dir: "journals", pages_dir: "pages", preferred_workflow: "now",
  shortcuts: {}, start_of_week: 6, block_hidden_properties: [], linked_references_collapsed_threshold: 100, default_journal_template: null,
  favorites: [], journal_page_title_format: "MMM do, yyyy", journal_file_name_format: "yyyy_MM_dd",
  preferred_format: "md", macros: {}, enable_timetracking: true, show_brackets: true, logbook_with_second_support: true,
  logbook_enabled_in_timestamped_blocks: false, logbook_enabled_in_all_blocks: false, guide_announced: true, mobile_gestures_disabled_in_block_with_tags: [],
};

beforeAll(() => initParser());

afterEach(() => {
  vi.restoreAllMocks();
  setGraphMeta(null);
  resetStore();
  document.body.innerHTML = "";
});

it("routes slash-template insertion through applyTemplateVars with the current page", async () => {
  vi.spyOn(backend(), "listTemplates").mockResolvedValue([{
    name: "Daily",
    page: "Templates",
    kind: "page",
    blocks: [{ id: "template", raw: "on <% current page %>", collapsed: false, children: [] }],
  }]);
  setGraphMeta(META);
  const page: PageDto = {
    name: "Shared", kind: "page", title: "Shared", pre_block: null,
    blocks: [{ id: "host", raw: "/Daily", collapsed: false, children: [] }],
  };
  loadSingle(page);
  startEditing("host", "/Daily".length);

  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => (
    <For each={pageByName("Shared")?.roots ?? []}>{(id) => <Block id={id} />}</For>
  ), root);
  try {
    const textarea = root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
    textarea.focus();
    textarea.dispatchEvent(new InputEvent("input", {
      bubbles: true, inputType: "insertText", data: "y",
    }));
    await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("Template: Daily"));
    textarea.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Enter", code: "Enter", bubbles: true, cancelable: true,
    }));
    await vi.waitFor(() => expect(Object.values(doc.byId).map((block) => block.raw)).toContain("on [[Shared]]"));
  } finally {
    dispose();
  }
});

it("does not offer the previous graph's cached templates after a graph switch", async () => {
  const list = vi.spyOn(backend(), "listTemplates")
    .mockResolvedValueOnce([{ name: "From A", page: "Templates", kind: "page", blocks: [] }])
    .mockResolvedValueOnce([{ name: "From B", page: "Templates", kind: "page", blocks: [] }]);
  const open = async (rootPath: string, blockId: string) => {
    setGraphMeta({ ...META, root: rootPath });
    bumpGraphEpoch();
    loadSingle({ name: "Shared", kind: "page", title: "Shared", pre_block: null,
      blocks: [{ id: blockId, raw: "/", collapsed: false, children: [] }] });
    startEditing(blockId, 1);
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Block id={blockId} />, root);
    const textarea = root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
    textarea.focus();
    textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "x" }));
    return { root, dispose };
  };
  const a = await open("/tmp/template-A", "host-A");
  await vi.waitFor(() => expect(list).toHaveBeenCalledTimes(1));
  a.dispose();
  resetStore();
  const b = await open("/tmp/template-B", "host-B");
  try {
    await vi.waitFor(() => expect(list).toHaveBeenCalledTimes(2));
  } finally {
    b.dispose();
  }
});

it("reports a failed template listing and retries it instead of caching 'no templates' (C5 I-9)", async () => {
  const { toasts, setToasts } = await import("../toasts");
  setToasts([]);
  const list = vi.spyOn(backend(), "listTemplates")
    .mockRejectedValueOnce(new Error("templates unreadable"))
    .mockResolvedValueOnce([{ name: "Later", page: "Templates", kind: "page", blocks: [] }]);
  setGraphMeta({ ...META, root: "/tmp/template-fail" });
  bumpGraphEpoch();
  loadSingle({ name: "Shared", kind: "page", title: "Shared", pre_block: null,
    blocks: [{ id: "host-fail", raw: "/", collapsed: false, children: [] }] });
  startEditing("host-fail", 1);
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Block id="host-fail" />, root);
  try {
    const textarea = root.querySelector<HTMLTextAreaElement>("textarea.block-editor")!;
    textarea.focus();
    textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "L" }));
    await vi.waitFor(() => expect(toasts().some((t) => t.kind === "error" && t.message.includes("templates unreadable"))).toBe(true));
    textarea.value = "/La";
    textarea.setSelectionRange(3, 3);
    textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "a" }));
    await vi.waitFor(() => expect(list).toHaveBeenCalledTimes(2));
  } finally {
    dispose();
    setToasts([]);
  }
});
