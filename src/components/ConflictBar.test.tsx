import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { ConflictBar } from "./ConflictBar";
import { markConflict, conflicts, clearConflict } from "../ui";
import { backend } from "../backend";
import { loadSingle, resetStore, pageByName, doc } from "../store";
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

  it("refuses to install disk bytes read against a graph that has been reopened", async () => {
    // Acceptance row D2 requires the click to capture the graph binding and
    // re-check it at the final boundary. It captured the edit generation and the
    // instance generation but never the binding, so a backend reopen landing
    // between the click and the read — `changeJournalTitleFormat` and five other
    // settings all reach `refresh_graph`, which may MIGRATE journal filenames —
    // let bytes describing the old graph replace the user's unsaved work and
    // clear the banner. (GH #254 increment 3, round 15.)
    let landRead: (dto: unknown) => void = () => {};
    vi.spyOn(backend(), "getPageByPath").mockReturnValue(
      new Promise((r) => {
        landRead = r as (dto: unknown) => void;
      }) as never,
    );
    const { notifyGraphRebound } = await import("../modeHooks");

    const { root, dispose } = mountWithStrayLoaded();
    root.querySelectorAll<HTMLButtonElement>(".conflict-btn")[0].click();
    await Promise.resolve();

    // The graph is reopened while the disk read is outstanding...
    notifyGraphRebound();
    landRead(page(strayPath, "bytes from the graph that was replaced"));
    await Promise.resolve();
    await Promise.resolve();

    // ...so those bytes must not be installed over the editor's content, and the
    // banner must not be silently cleared as though the discard succeeded.
    // Read the actual node text. `FeedPage` has no `blocks` field — asserting on
    // one compares against "" and passes however the product behaves.
    const live = pageByName(sharedName);
    const text = live ? (doc.byId[live.roots[0]]?.raw ?? "") : "";
    expect(text, "stale bytes must not replace the editor's content").not.toContain(
      "was replaced",
    );
    expect(conflicts(), "nor may the banner be cleared as though it succeeded").toContain(
      sharedName,
    );
    dispose();
  });

  it("reloads the pinned file, not the canonical owner of the name", async () => {
    const getPageByPath = vi.spyOn(backend(), "getPageByPath")
      .mockImplementation(async (path) => (path === strayPath ? page(strayPath, "disk text of the stray") : null));
    const getPage = vi.spyOn(backend(), "getPage")
      .mockResolvedValue(page(canonicalPath, "the CANONICAL day, a different file"));

    const { root, dispose } = mountWithStrayLoaded();
    root.querySelectorAll<HTMLButtonElement>(".conflict-btn")[0].click();

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
