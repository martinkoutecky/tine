import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { For } from "solid-js";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { startEditing } from "../editorController";
import { initParser } from "../render/parse";
import { doc, loadSingle, pageByName, resetStore } from "../store";
import type { GraphMeta, PageDto } from "../types";
import { setGraphMeta } from "../ui";
import { Block } from "./Block";

const META: GraphMeta = {
  root: "/tmp/template-graph", journals_dir: "journals", pages_dir: "pages", preferred_workflow: "now",
  shortcuts: {}, start_of_week: 6, block_hidden_properties: [], default_journal_template: null,
  favorites: [], journal_page_title_format: "MMM do, yyyy", journal_file_name_format: "yyyy_MM_dd",
  preferred_format: "md", macros: {}, enable_timetracking: true, show_brackets: true, logbook_with_second_support: true,
  logbook_enabled_in_timestamped_blocks: false, logbook_enabled_in_all_blocks: false, guide_announced: true,
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
