import { afterEach, expect, it, vi } from "vitest";
import { setActiveLocale, t } from "./i18n";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke, convertFileSrc: (path: string) => path }));
afterEach(async () => { await setActiveLocale("en"); vi.unstubAllGlobals(); invoke.mockReset(); });

it("rename and merge explain the title identity refusal through the catalog", async () => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  invoke.mockImplementation((command: string) =>
    ["rename_page", "merge_pages"].includes(command) ? Promise.reject("Q-TITLE") : Promise.resolve(null));
  const { backend } = await import("./backend");
  for (const locale of ["en", "zh-CN", "fa"]) {
    await setActiveLocale(locale);
    const expected = t("page-rename-title-identity");
    expect(expected).toContain("Q-TITLE");
    expect(expected).not.toBe("Q-TITLE");
    await expect(backend().renamePage("Old", "New", "rename-page")).rejects.toBe(expected);
    await expect(backend().mergePages("a.md", "b.md", ["insert-blocks", "delete-page"])).rejects.toBe(expected);
  }
});

it.each([
  ["F-LINK", "page-merge-missing-name-range"],
  ["Q-OWNER", "page-merge-alias-owner"],
] as const)("merge explains %s through the catalog", async (reason, key) => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  invoke.mockImplementation((command: string) => command === "merge_pages" ? Promise.reject(reason) : Promise.resolve(null));
  const { backend } = await import("./backend");
  for (const locale of ["en", "zh-CN", "fa"]) {
    await setActiveLocale(locale);
    const expected = t(key);
    expect(expected).toContain(reason);
    expect(expected).not.toBe(reason);
    await expect(backend().mergePages("a.md", "b.md", ["insert-blocks", "delete-page"])).rejects.toBe(expected);
  }
});
