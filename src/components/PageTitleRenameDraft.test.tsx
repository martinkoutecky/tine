// og 20b contract 2: an open page-title rename draft is uncommitted input. A
// watcher reload of the same page arriving meanwhile must not remount the title
// and throw the typed name away; the user commits or cancels it.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { doc, setDoc } from "../document/model";
import { reloadPageIfStillSafe } from "../document/workingSet";
import { endEdit } from "../editorController";
import { mainPaneRouter, resetTabsToJournals } from "../router";
import type { PageRead } from "../types";
import { PageView } from "./Page";

beforeAll(async () => { await initParser(); });
afterEach(() => {
  vi.restoreAllMocks();
  endEdit("blur");
  resetStore();
  document.body.innerHTML = "";
  resetTabsToJournals();
});

const tick = () => new Promise<void>((resolve) => queueMicrotask(resolve));

describe("a page-title rename draft survives an external reload (og 20b)", () => {
  it("keeps the typed name and the page instance until the user commits or cancels", async () => {
    const dto: PageRead = {
      name: "Draft title", kind: "page", title: "Draft title", pre_block: null, id: "pages/Draft title.md",
      blocks: [{ id: "draft-root", raw: "Body", collapsed: false, children: [] }],
    };
    setDoc({
      byId: { "draft-root": { id: "draft-root", raw: "Body", collapsed: false, parent: null, page: dto.name, children: [] } },
      pages: [{ name: dto.name, kind: "page", title: dto.name, preBlock: null, roots: ["draft-root"], format: "md", readOnly: false, guide: false, id: dto.id }],
      feed: [], loaded: true,
    });
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(dto);
    vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
    vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
    mainPaneRouter.openFile(dto.id, dto.name, "page", { inPlace: true });
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <PageView />, root);
    try {
      await tick(); await tick();
      root.querySelector<HTMLElement>(".page-title")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
      await tick();
      const input = root.querySelector<HTMLInputElement>(".page-title-input")!;
      input.value = "Half typed new na";
      input.dispatchEvent(new InputEvent("input", { bubbles: true }));

      const disk: PageRead = { ...dto, blocks: [{ id: "disk-root", raw: "Changed on disk", collapsed: false, children: [] }] };
      expect(reloadPageIfStillSafe(dto.name, disk)).toBe(false);
      await tick();
      expect(doc.byId["draft-root"]?.raw).toBe("Body");
      const still = root.querySelector<HTMLInputElement>(".page-title-input");
      expect(still?.value).toBe("Half typed new na");

      still!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      await tick(); await tick();
      expect(reloadPageIfStillSafe(dto.name, disk)).toBe(true);
    } finally {
      dispose();
    }
  });
});
