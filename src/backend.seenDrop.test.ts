import { afterEach, expect, it, vi } from "vitest";

// ADR 0079 / F6: Tine's own rename, merge and delete make the backend drop the
// page's seen record. The page must not be marked seen on leave afterwards: a
// mark would recreate the record under the old key. The native adapter forgets
// the displayed page before it dispatches the command.
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke, convertFileSrc: (path: string) => path }));

afterEach(() => { vi.unstubAllGlobals(); vi.resetModules(); invoke.mockReset(); });

async function setup() {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  invoke.mockResolvedValue(null);
  const { backend } = await import("./backend");
  const displayed = await import("./seen/displayed");
  const owned = await import("./owned");
  const b = backend();
  await b.startupGraphPath();
  const show = (name: string, path: string) => displayed.registerDisplayed(name, { key: name.toLowerCase(), path, owner: owned.graphOwner() });
  return { b, show, displayed };
}

it("renaming, merging, renaming a file to a page and deleting each forget the displayed page", async () => {
  const { b, show, displayed } = await setup();
  show("Old", "pages/Old.md");
  show("Src", "pages/Src.md");
  show("Dst", "pages/Dst.md");
  show("Loose", "pages/loose.md");
  show("Gone", "pages/Gone.md");
  show("Other", "pages/Other.md");
  await b.renamePage("Old", "New", "rename-page");
  await b.mergePages("Src", "Dst", "rename-page" as never);
  await b.renameFileToPage("pages/loose.md", "Loose Page", "rename-page");
  await b.pageDelete(1, "Gone", "page");
  for (const name of ["Old", "Src", "Dst", "Loose", "Gone"]) expect(displayed.takeDisplayed(name), name).toBeUndefined();
  expect(displayed.takeDisplayed("Other")).toBeDefined();
});
