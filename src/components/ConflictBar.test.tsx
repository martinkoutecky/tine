import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { ConflictBar } from "./ConflictBar";
import { markConflict, conflicts, clearConflict } from "../ui";
import { backend } from "../backend";
import { loadSingle, resetStore, pageByName } from "../store";
import type { PageDto } from "../types";

afterEach(() => {
  for (const name of conflicts()) clearConflict(name);
  resetStore();
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

// Two files can carry the same page NAME — the duplicate-day stray of GH #21,
// or two same-titled pages in different folders. The editor pins such a page to
// its exact `path` so a save writes back to the file it was loaded from.
//
// "Use disk version" has to honour the same pin. Resolving by name reaches the
// backend's CANONICAL owner of that name, so the button silently re-points the
// tab at a different file: the user asked to discard their edits to THIS file
// and instead got someone else's file loaded in its place, with their own
// unsaved work gone. (Direct Files data-safety audit, 2026-08-09, finding 10.)
describe("resolving a conflict on a page pinned to a specific file", () => {
  const sharedName = "2026_06_26";
  const strayPath = "journals/2026_06_26 (1).md";
  const canonicalPath = "journals/2026_06_26.md";

  const page = (path: string, text: string): PageDto => ({
    name: sharedName,
    kind: "journal",
    title: sharedName,
    pre_block: null,
    path,
    rev: `rev-of-${path}`,
    blocks: [{ id: `block-of-${path}`, raw: text, collapsed: false, children: [], properties: [] }],
  });

  function mountWithStrayLoaded() {
    loadSingle(page(strayPath, "the stray's text"));
    markConflict(sharedName);
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <ConflictBar />, root);
    return { root, dispose };
  }

  it("reloads the pinned file, not the canonical owner of the name", async () => {
    const getPageByPath = vi.spyOn(backend(), "getPageByPath")
      .mockImplementation(async (path) => (path === strayPath ? page(strayPath, "disk text of the stray") : null));
    const getPage = vi.spyOn(backend(), "getPage")
      .mockResolvedValue(page(canonicalPath, "the CANONICAL day, a different file"));

    const { root, dispose } = mountWithStrayLoaded();
    const actions = root.querySelectorAll<HTMLButtonElement>(".conflict-btn");
    expect(actions[0].textContent?.trim()).toBe("Use current version");
    expect(actions[1].textContent?.trim()).toBe("Keep mine");
    expect(actions[1].disabled).toBe(false);
    actions[0].click();

    await vi.waitFor(() => expect(conflicts()).toEqual([]));
    expect(getPageByPath).toHaveBeenCalledWith(strayPath);
    expect(getPage).not.toHaveBeenCalled();
    expect(pageByName(sharedName)?.path).toBe(strayPath);
    dispose();
  });

  // The other half of the same pin: if the pinned file really is gone, falling
  // back to the name would resurrect the tab pointing at an unrelated file. The
  // page must be dropped, exactly as it is for an unpinned page.
  it("drops the page when the pinned file itself is gone", async () => {
    vi.spyOn(backend(), "getPageByPath").mockResolvedValue(null);
    const getPage = vi.spyOn(backend(), "getPage")
      .mockResolvedValue(page(canonicalPath, "the CANONICAL day, a different file"));

    const { root, dispose } = mountWithStrayLoaded();
    root.querySelectorAll<HTMLButtonElement>(".conflict-btn")[0].click();

    await vi.waitFor(() => expect(pageByName(sharedName)).toBeUndefined());
    expect(getPage).not.toHaveBeenCalled();
    dispose();
  });

  // An ordinary page with no pin keeps resolving by name — a brand-new page has
  // no path at all, and name resolution is what finds its file once it exists.
  it("still resolves an unpinned page by name", async () => {
    const withoutPath: PageDto = { ...page(canonicalPath, "text"), path: undefined };
    loadSingle(withoutPath);
    markConflict(sharedName);
    const getPageByPath = vi.spyOn(backend(), "getPageByPath").mockResolvedValue(null);
    const getPage = vi.spyOn(backend(), "getPage").mockResolvedValue(withoutPath);

    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <ConflictBar />, root);
    root.querySelectorAll<HTMLButtonElement>(".conflict-btn")[0].click();

    await vi.waitFor(() => expect(conflicts()).toEqual([]));
    expect(getPage).toHaveBeenCalledWith(sharedName, "journal");
    expect(getPageByPath).not.toHaveBeenCalled();
    dispose();
  });
});
