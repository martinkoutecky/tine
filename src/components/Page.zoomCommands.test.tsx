import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { loadSingle } from "../document/workingSet";
import { editingId, endEdit, startEditing } from "../editorController";
import { maybeEditorIn } from "../tests/editorDriver";
import { PageView } from "./Page";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { runGlobalCommand } from "../keybindings";
import { setGraphMeta } from "../graphSession";
import { resetPaneLayoutToSingle } from "../panes";
import type { PageDto, PageRead } from "../types";

beforeAll(async () => { await initParser(); });
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  endEdit("blur");
  resetStore();
  setGraphMeta(null);
  resetPaneLayoutToSingle();
  document.body.innerHTML = "";
  resetTabsToJournals();
});

const leaf = (id: string, children: PageDto["blocks"] = []) => ({ id, raw: `Text of ${id}`, collapsed: false, children });
const dto: PageDto = {
  name: "Zoom keys", kind: "page", title: "Zoom keys", pre_block: null,
  blocks: [leaf("top", [leaf("mid", [leaf("deep")])])],
};

async function mounted() {
  loadSingle(dto);
  vi.spyOn(backend(), "getPage").mockResolvedValue(dto as PageRead);
  mainPaneRouter.replaceActiveRoute({ kind: "page", name: dto.name, pageKind: "page" });
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <PageView />, root);
  await vi.waitFor(() => expect(root.querySelector('[data-block-id="deep"]')).not.toBeNull());
  return { root, dispose };
}

// GH #799 on a routed page: the keyboard twin of the bullet click and the breadcrumb.
it("Zoom in on block opens the edited block's zoomed view with the caret still in it; Zoom out one level steps to the parent", async () => {
  const { root, dispose } = await mounted();
  try {
    startEditing("mid", 4, null, "main");
    await vi.waitFor(() => expect(maybeEditorIn(root)?.blockId).toBe("mid"));
    expect(runGlobalCommand("editor/zoom-in")).toBe(true);
    await vi.waitFor(() => expect(root.querySelector(".zoomed-page")).not.toBeNull());
    // The zoom root is "mid": its breadcrumb names the ancestor "top", not "mid" itself.
    expect(root.querySelector(".zoom-breadcrumb")?.textContent).toContain("Text of top");
    expect(root.querySelector(".zoom-breadcrumb")?.textContent).not.toContain("Text of mid");
    await vi.waitFor(() => {
      expect(editingId()).toBe("mid");
      const editor = maybeEditorIn(root);
      expect(editor?.blockId).toBe("mid");
      expect(editor?.selection()).toEqual([4, 4]);
    });
    // One level out: the zoom root becomes "top" and the same block is edited again.
    runGlobalCommand("editor/zoom-out");
    await vi.waitFor(() => expect(root.querySelector(".zoom-breadcrumb")?.textContent).not.toContain("Text of top"));
    await vi.waitFor(() => expect(maybeEditorIn(root)?.blockId).toBe("mid"));
    // Out again: back to the plain page; a further press changes nothing.
    runGlobalCommand("editor/zoom-out");
    await vi.waitFor(() => expect(root.querySelector(".zoomed-page")).toBeNull());
    expect(runGlobalCommand("editor/zoom-out")).toBe(true);
    expect(root.querySelector(".zoomed-page")).toBeNull();
  } finally {
    dispose();
  }
});
