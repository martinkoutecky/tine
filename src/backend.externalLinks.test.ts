import { afterEach, expect, it, vi } from "vitest";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn(async () => undefined) }));
vi.mock("@tauri-apps/api/core", () => ({ invoke, convertFileSrc: (path: string) => path }));

afterEach(() => {
  vi.unstubAllGlobals();
  vi.resetModules();
  invoke.mockClear();
});

it("checks the desktop external link before IPC and preserves native routes", async () => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  const { backend } = await import("./backend");
  const api = backend();
  const refusals = await Promise.allSettled([
    api.openExternal(" java\nscript:alert(1)", "javascript"),
    api.openExternal("zotero://select/items/ABC"),
  ]);
  expect(invoke).not.toHaveBeenCalled();
  expect(refusals.map(({ status }) => status)).toEqual(["rejected", "rejected"]);
  for (const url of ["https://example.org", "mailto:a@b.c", "file:///tmp/note.txt", "tine://today"]) {
    await api.openExternal(url);
    expect(invoke).toHaveBeenLastCalledWith("open_external", { url, confirmedScheme: null });
  }
  const url = "zotero://select/items/ABC";
  await api.openExternal(url, "zotero");
  expect(invoke).toHaveBeenLastCalledWith("open_external", { url, confirmedScheme: "zotero" });
});

it("checks the browser preview's external link before opening a window", async () => {
  const open = vi.fn();
  vi.stubGlobal("window", { open });
  const { mockBackend } = await import("./mock");
  const api = mockBackend();
  const refusals = await Promise.allSettled([
    api.openExternal("about:blank"),
    api.openExternal("zotero://select/items/ABC"),
  ]);
  expect(open).not.toHaveBeenCalled();
  expect(refusals.map(({ status }) => status)).toEqual(["rejected", "rejected"]);
  await api.openExternal("https://example.org");
  expect(open).toHaveBeenCalledExactlyOnceWith("https://example.org", "_blank", "noreferrer");
});
