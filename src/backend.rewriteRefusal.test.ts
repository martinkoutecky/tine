import { afterEach, expect, it, vi } from "vitest";
import { renameRefusalMessage } from "./renameRefusal";
import { render, setActiveLocale } from "./i18n";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke, convertFileSrc: (path: string) => path }));
afterEach(async () => { await setActiveLocale("en"); vi.unstubAllGlobals(); invoke.mockReset(); });

it.each([
  ["page header", "rename_offer", "Old", "New"],
  ["context menu", "rename_offer", "Old", "New"],
  ["All pages", "rename_offer", "Old", "New"],
  ["alias rename", "rename_offer", "Shortcut", "Quick"],
  ["Favorites", "rename_offer", "Old", "New"],
  ["namespace rename", "rename_page", "Old", "New"],
  ["confirmed name merge", "rename_page", "Old", "New"],
  ["file merge", "merge_pages", "pages/Old.md", "pages/New.md"],
  ["duplicate-file rename", "rename_file_to_page", "journals/stray.md", "Rescued"],
])("keeps typed refusals through the %s command boundary", async (_entry, command, from, to) => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  const failure = { refused: { kind: "unindexed" as const, file: "pages/unreadable.md" } };
  invoke.mockRejectedValue(failure);
  const { backend } = await import("./backend");
  const api = backend();
  const request = command === "rename_offer" ? api.renameOffer(from, to)
    : command === "rename_page" ? api.renamePage(from, to, "rename-page")
    : command === "rename_file_to_page" ? api.renameFileToPage(from, to, "rename-page")
    : api.mergePages(from, to, ["insert-blocks", "delete-page"]);
  const caught = await request.catch((error: unknown) => error);
  expect(caught).toBe(failure);
  expect(invoke.mock.calls.some(([called]) => called === command)).toBe(true);
  await setActiveLocale("zh-CN");
  const message = render(renameRefusalMessage(failure.refused));
  expect(message).toContain("无法安全读取 pages/unreadable.md");
  expect(message).not.toContain("[object Object]");
});
