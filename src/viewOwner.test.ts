import { beforeEach, describe, expect, it, vi } from "vitest";

// Two inputs of "which file does this view show": the store's published
// answer for a name, and the page loaded under that name. Only these readers
// are replaced; the sidebar and router logic run unchanged.
const owners = new Map<string, string>();
const loaded = new Map<string, { id: string; kind: string }>();
vi.mock("./pageIndex", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./pageIndex")>()),
  resolvedTarget: (name: string, kind = "page") => {
    const id = owners.get(`${kind}:${name}`);
    return id ? { kind: "existing", id } : undefined;
  },
}));
vi.mock("./document/model", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./document/model")>()),
  loadedPage: (name: string) => loaded.get(name),
}));

import { openBlockInSidebar, openPageInSidebar, rightSidebar, setRightSidebar } from "./ui";
import { sameNavigationRoute } from "./router";

// UI-ORG-RESULT-ROW-FORMAT, reviews 62c50db18 F1, c81135cba F1 and
// cb69e3d96 F1: result rows carry their source file. Opening one must not
// close the name-only view of that same file; a different same-name file
// still replaces it. The answer comes from the inventory only: a loaded
// same-name file is never taken for the name's owner, and while the
// inventory cannot say, nothing is evicted and no tab is reused on file grounds.
describe.each([
  { label: "Markdown page", name: "Src", pageKind: "page", path: "pages/Src.md", other: "pages/dup/Src.md" },
  { label: "Org page", name: "Src", pageKind: "page", path: "pages/Src.org", other: "pages/Src.md" },
  { label: "Markdown journal", name: "Nov 2nd, 2026", pageKind: "journal", path: "journals/2026_11_02.md", other: "journals/2026_11_02.org" },
  { label: "Org journal", name: "Nov 2nd, 2026", pageKind: "journal", path: "journals/2026_11_02.org", other: "journals/2026_11_02.md" },
] as const)("one owner answer for views, $label", ({ name, pageKind, path, other }) => {
  const byName = { kind: "page", name, pageKind } as const;
  const byPath = { kind: "page", name, pageKind, path } as const;
  const byOther = { kind: "page", name, pageKind, path: other } as const;
  beforeEach(() => {
    owners.clear();
    loaded.clear();
    setRightSidebar([]);
  });

  describe("inventory names the file", () => {
    beforeEach(() => { owners.set(`${pageKind}:${name}`, path); });

    it("a block of the same file opened after the name-only page keeps the page", () => {
      setRightSidebar([{ kind: "page", name, pageKind }]);
      openBlockInSidebar({ uuid: "u1", page: name, pageKind, path });
      expect(rightSidebar().map((i) => i.kind)).toEqual(["block", "page"]);
    });

    it("a block of the same file opened after a name-only block keeps both", () => {
      setRightSidebar([{ kind: "block", uuid: "u0", page: name, pageKind }]);
      openBlockInSidebar({ uuid: "u1", page: name, pageKind, path });
      expect(rightSidebar().map((i) => i.kind === "block" && i.uuid)).toEqual(["u1", "u0"]);
    });

    it("the same file's page opened by path after a name-only block keeps the block", () => {
      setRightSidebar([{ kind: "block", uuid: "u0", page: name, pageKind }]);
      openPageInSidebar({ name, pageKind, path });
      expect(rightSidebar().map((i) => i.kind)).toEqual(["page", "block"]);
    });

    it("a block of a different same-name file still replaces the name-only page", () => {
      setRightSidebar([{ kind: "page", name, pageKind }]);
      openBlockInSidebar({ uuid: "u1", page: name, pageKind, path: other });
      expect(rightSidebar().map((i) => i.kind)).toEqual(["block"]);
    });

    it("a name-only tab is reused for its owner's exact path, in both orders, and never for another file", () => {
      expect(sameNavigationRoute(byName, byPath)).toBe(true);
      expect(sameNavigationRoute(byPath, byName)).toBe(true);
      expect(sameNavigationRoute(byName, byOther)).toBe(false);
    });

    it("a loaded same-name alternate does not make the name mean that file (cb69e3d96 F1)", () => {
      loaded.set(name, { id: other, kind: pageKind });
      expect(sameNavigationRoute(byName, byOther)).toBe(false);
      expect(sameNavigationRoute(byName, byPath)).toBe(true);
      setRightSidebar([{ kind: "page", name, pageKind }]);
      openBlockInSidebar({ uuid: "u1", page: name, pageKind, path });
      expect(rightSidebar().map((i) => i.kind)).toEqual(["block", "page"]);
    });
  });

  describe("inventory cannot say (refresh), file loaded under the name", () => {
    beforeEach(() => { loaded.set(name, { id: path, kind: pageKind }); });

    it.each([path, other])("an exact result block of %s keeps the name-only page", (resultPath) => {
      setRightSidebar([{ kind: "page", name, pageKind }]);
      openBlockInSidebar({ uuid: "u1", page: name, pageKind, path: resultPath });
      expect(rightSidebar().map((i) => i.kind)).toEqual(["block", "page"]);
    });

    it("no tab is reused on file grounds", () => {
      expect(sameNavigationRoute(byName, byPath)).toBe(false);
      expect(sameNavigationRoute(byPath, byName)).toBe(false);
    });
  });
});
