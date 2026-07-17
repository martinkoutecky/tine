import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { startEditing } from "../editorController";
import { pluginManager } from "../plugins/manager";
import type { PluginEffect } from "../plugins/protocol";
import { initParser } from "../render/parse";
import { doc, loadSingle, resetStore } from "../store";
import type { GraphMeta, PageDto } from "../types";
import { bumpGraphEpoch, setGraphMeta, setGraphTransitioning } from "../ui";
import { Block } from "./Block";

beforeAll(() => initParser());

function meta(root: string): GraphMeta {
  return {
    root, journals_dir: "journals", pages_dir: "pages", preferred_workflow: "now",
    shortcuts: {}, start_of_week: 6, block_hidden_properties: [], default_journal_template: null,
    favorites: [], journal_page_title_format: "MMM do, yyyy", journal_file_name_format: "yyyy_MM_dd",
    preferred_format: "md", macros: {}, enable_timetracking: true, logbook_with_second_support: true,
    logbook_enabled_in_timestamped_blocks: false, logbook_enabled_in_all_blocks: false, guide_announced: true,
  };
}

function page(raw = "/Delayed"): PageDto {
  return {
    name: "Shared", kind: "page", title: "Shared", pre_block: null,
    blocks: [{ id: "shared-id", raw, collapsed: false, children: [] }],
  };
}

function mountEditor() {
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Block id="shared-id" />, root);
  return { root, dispose, textarea: root.querySelector<HTMLTextAreaElement>("textarea.block-editor")! };
}

async function choosePlugin(textarea: HTMLTextAreaElement) {
  textarea.focus();
  textarea.setSelectionRange(textarea.value.length, textarea.value.length);
  textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: "d" }));
  await vi.waitFor(() => expect(document.body.querySelector(".autocomplete .ac-label")?.textContent).toBe("Delayed"));
  textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", bubbles: true, cancelable: true }));
}

afterEach(() => {
  vi.restoreAllMocks();
  setGraphTransitioning(false);
  setGraphMeta(null);
  resetStore();
  document.body.innerHTML = "";
});

describe("plugin slash editor ownership", () => {
  it("drops a delayed graph-A completion after unmount without changing or focusing equal graph-B content", async () => {
    vi.spyOn(pluginManager, "slashCommands").mockReturnValue([{
      pluginId: "page.tine.delayed",
      contribution: { id: "delayed", title: "Delayed" },
    }]);
    let resolve!: (effects: PluginEffect[]) => void;
    vi.spyOn(pluginManager, "invokeSlashCommand").mockReturnValue(new Promise((done) => { resolve = done; }));

    setGraphMeta(meta("/graph-a"));
    loadSingle(page());
    startEditing("shared-id", 8);
    const graphA = mountEditor();
    await choosePlugin(graphA.textarea);
    await vi.waitFor(() => expect(pluginManager.invokeSlashCommand).toHaveBeenCalledTimes(1));

    setGraphTransitioning(true);
    graphA.dispose();
    resetStore();
    bumpGraphEpoch();
    setGraphMeta(meta("/graph-b"));
    loadSingle(page());
    startEditing("shared-id", 8);
    setGraphTransitioning(false);
    const graphB = mountEditor();
    const focus = vi.spyOn(graphB.textarea, "focus");
    resolve([{ kind: "insert-at-caret", text: "inserted" }]);
    await Promise.resolve();
    await Promise.resolve();

    expect(doc.byId["shared-id"].raw).toBe("/Delayed");
    expect(graphB.textarea.value).toBe("/Delayed");
    expect(focus).not.toHaveBeenCalled();
    graphB.dispose();
  });

  it("inserts and restores the caret for an unchanged current editor", async () => {
    vi.spyOn(pluginManager, "slashCommands").mockReturnValue([{
      pluginId: "page.tine.delayed",
      contribution: { id: "delayed", title: "Delayed" },
    }]);
    vi.spyOn(pluginManager, "invokeSlashCommand").mockResolvedValue([{ kind: "insert-at-caret", text: "inserted" }]);
    setGraphMeta(meta("/graph-a"));
    loadSingle(page());
    startEditing("shared-id", 8);
    const current = mountEditor();
    await choosePlugin(current.textarea);
    await vi.waitFor(() => expect(doc.byId["shared-id"].raw).toBe("inserted"));
    await Promise.resolve();
    expect(current.textarea.value).toBe("inserted");
    expect(current.textarea.selectionStart).toBe("inserted".length);
    expect(document.activeElement).toBe(current.textarea);
    current.dispose();
  });
});
