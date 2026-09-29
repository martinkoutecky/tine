import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import type { PageRead } from "../types";
import { PageView } from "./Page";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import { clearRecent, recentPages, setRecentPages } from "../ui";
import { setGraphMeta } from "../graphSession";
import { setToasts } from "../toasts";

beforeAll(async () => { await initParser(); });
afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  setGraphMeta(null);
  setToasts([]);
  clearRecent();
  document.body.innerHTML = "";
  resetTabsToJournals();
});
function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  return { root, dispose: render(node, root) };
}

/** GH #597 (master d42121be6177): a route pinned to another case spelling of the
 * file (saved while Tine handed out `pages/Contents.md` for `contents.md`) opens
 * the file under its disk spelling and re-keys the tab and Recent entry to it. */
describe("a route pinned to another case spelling of its file (GH #597)", () => {
  it("opens the file under its disk spelling and re-keys the tab and Recent entry", async () => {
    const dto: PageRead = { name: "contents", title: "contents", kind: "page", id: "pages/contents.md",
      pre_block: null, rev: "disk-rev",
      blocks: [{ id: "contents-root", raw: "Table of contents", children: [], collapsed: false }] };
    const read = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    setRecentPages([{ name: "Contents", kind: "page", path: "pages/Contents.md" }]);
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Contents", pageKind: "page", path: "pages/Contents.md" });
    const { root, dispose } = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(root.textContent).toContain("Table of contents"));
      expect(read).toHaveBeenCalledWith("pages/Contents.md");
      expect(root.textContent).not.toContain("no longer available at that path");
      expect(mainPaneRouter.route()).toMatchObject({ kind: "page", name: "contents", path: "pages/contents.md" });
      expect(recentPages().filter((r) => r.path === "pages/Contents.md")).toEqual([]);
      expect(recentPages()).toContainEqual({ name: "contents", kind: "page", path: "pages/contents.md" });
    } finally { dispose(); clearRecent(); }
  });

  it("still refuses a pinned path whose file is a different page, not a case spelling", async () => {
    const dto: PageRead = { name: "Other", title: "Other", kind: "page", id: "pages/Other.md",
      pre_block: null, rev: "r", blocks: [{ id: "o", raw: "Other body", children: [], collapsed: false }] };
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    mainPaneRouter.replaceActiveRoute({ kind: "page", name: "Contents", pageKind: "page", path: "pages/Contents.md" });
    const { root, dispose } = mount(() => <PageView />);
    try {
      await vi.waitFor(() => expect(root.textContent).toContain("no longer available at that path"));
      expect(mainPaneRouter.route()).toMatchObject({ path: "pages/Contents.md" });
    } finally { dispose(); }
  });
});
