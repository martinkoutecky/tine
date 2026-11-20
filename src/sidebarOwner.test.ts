import { beforeEach, describe, expect, it, vi } from "vitest";

// The store's published answer for a name: which file it opens. Only this
// reader is replaced; the sidebar's own logic runs unchanged.
const owners = new Map<string, string>();
vi.mock("./pageIndex", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./pageIndex")>()),
  resolvedTarget: (name: string, kind = "page") => {
    const id = owners.get(`${kind}:${name}`);
    return id ? { kind: "existing", id } : undefined;
  },
}));

import { openBlockInSidebar, openPageInSidebar, rightSidebar, setRightSidebar } from "./ui";

// UI-ORG-RESULT-ROW-FORMAT, review 62c50db18 F1: result rows now carry their
// source file. Opening one in the sidebar must not close the name-only view of
// that same file; a different file of the same name still replaces it.
describe.each([
  { label: "Markdown page", name: "Src", pageKind: "page", path: "pages/Src.md", other: "pages/dup/Src.md" },
  { label: "Org page", name: "Src", pageKind: "page", path: "pages/Src.org", other: "pages/Src.md" },
  { label: "Org journal", name: "Nov 2nd, 2026", pageKind: "journal", path: "journals/2026_11_02.org", other: "journals/2026_11_02.md" },
] as const)("sidebar same-owner check, $label", ({ name, pageKind, path, other }) => {
  beforeEach(() => {
    owners.clear();
    owners.set(`${pageKind}:${name}`, path);
    setRightSidebar([]);
  });

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
});
