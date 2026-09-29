// GH #254 family (master 7bd793bd0, og J1): a sidebar item pinned to one file
// whose name another file holds with unsaved input used to render a blank body
// with no message and no retry. It must say why and load once that holder is
// replaceable.
import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { ensurePageLoaded, pageByName, resetStore } from "../document";
import { endEdit, startEditing } from "../editorController";
import { applySidebarSession, setRightSidebar } from "../ui";
import { RightSidebar } from "./RightSidebar";
import type { PageDto } from "../types";

beforeAll(async () => { await initParser(); });
afterEach(() => { endEdit("graph-switch"); vi.restoreAllMocks(); applySidebarSession({ right: false, items: [] }); setRightSidebar([]); resetStore(); document.body.innerHTML = ""; });

const file = (id: string, raw: string): PageDto & { id: string; rev: string } => ({
  id, name: "P", title: "P", kind: "page", pre_block: null, rev: `rev-${id}`,
  blocks: [{ id: `${id}-b`, raw, collapsed: false, children: [] }],
});

it("says why a path-pinned item did not load, then loads it once the holder's edit ends", async () => {
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
  vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
  const read = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(file("pages/P.md", "requested text") as never);
  ensurePageLoaded(file("pages/stray.md", "stray text"));
  expect(pageByName("P")!.id).toBe("pages/stray.md");
  startEditing(pageByName("P")!.roots[0], 0);
  applySidebarSession({ right: true, items: [{ kind: "page", name: "P", pageKind: "page", path: "pages/P.md" }] });
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <RightSidebar />, root);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("pages/stray.md"));
    expect(root.textContent).toContain("pages/P.md");
    expect(pageByName("P")!.id).toBe("pages/stray.md");
    endEdit("blur");
    await vi.waitFor(() => expect(pageByName("P")!.id).toBe("pages/P.md"));
    expect(read.mock.calls.length).toBeGreaterThanOrEqual(2);
    await vi.waitFor(() => expect(root.textContent).toContain("requested text"));
    expect(root.textContent).not.toContain("pages/stray.md");
  } finally { dispose(); }
});
