import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { renameOrMergePage } from "./graph";
import { resetStore } from "./document";

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
});

// GH #327 / OG `merge-pages!`: renaming onto another page's name offers a merge.
it("asks before merging onto an existing page and names the confirmed survivor", async () => {
  vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "existing", id: "pages/New.md", others: [] });
  const confirm = vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  const rename = vi.spyOn(backend(), "renamePage").mockResolvedValue();
  expect(await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("merged");
  expect(confirm).toHaveBeenCalledWith("Page “New” already exists. Merge “Old” into it?");
  expect(rename).toHaveBeenCalledWith("Old", "New", "rename-page", "pages/Old.md", "pages/New.md");
});

it("declining the merge writes nothing", async () => {
  vi.spyOn(backend(), "resolvePage").mockResolvedValue({ kind: "existing", id: "pages/New.md", others: [] });
  vi.spyOn(backend(), "confirm").mockResolvedValue(false);
  const rename = vi.spyOn(backend(), "renamePage").mockResolvedValue();
  expect(await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("cancelled");
  expect(rename).not.toHaveBeenCalled();
});

it("renames without asking when the name is free or is the page's own", async () => {
  const resolve = vi.spyOn(backend(), "resolvePage");
  const confirm = vi.spyOn(backend(), "confirm");
  const rename = vi.spyOn(backend(), "renamePage").mockResolvedValue();
  resolve.mockResolvedValueOnce({ kind: "absent", id: "pages/New.md" });
  expect(await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("renamed");
  // A case-only rename resolves to the source itself.
  resolve.mockResolvedValueOnce({ kind: "existing", id: "pages/Old.md", others: [] })
    .mockResolvedValueOnce({ kind: "existing", id: "pages/Old.md", others: [] });
  expect(await renameOrMergePage("Old", "old")).toBe("renamed");
  expect(confirm).not.toHaveBeenCalled();
  expect(rename).toHaveBeenNthCalledWith(1, "Old", "New", "rename-page", "pages/Old.md");
  expect(rename).toHaveBeenNthCalledWith(2, "Old", "old", "rename-page");
});
