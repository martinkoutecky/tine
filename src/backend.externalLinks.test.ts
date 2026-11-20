import { afterEach, expect, it, vi } from "vitest";
import { publishedBackend, type PublishedSnapshot } from "./publishedBackend";
import { mockBackend } from "./mock";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn(async () => undefined) }));
vi.mock("@tauri-apps/api/core", () => ({ invoke, convertFileSrc: (path: string) => path }));

afterEach(() => {
  vi.unstubAllGlobals();
  vi.resetModules();
  invoke.mockClear();
});

it("checks desktop external links before IPC and preserves native routes", async () => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  const { backend } = await import("./backend");
  const api = backend();
  const refusals = await Promise.allSettled([
    api.openExternal(" java\nscript:assets/refused", "javascript"),
    api.openExternal("zotero://select/items/ABC"),
  ]);
  expect(invoke).not.toHaveBeenCalled();
  expect(refusals.map(({ status }) => status)).toEqual(["rejected", "rejected"]);
  for (const url of ["https://example.org", "mailto:a@b.c", "file:///tmp/note.txt", "tine://today"]) {
    await api.openExternal(url);
    expect(invoke).toHaveBeenLastCalledWith("open_external", { url, confirmedScheme: null });
  }
  const url = "zotero://select/items/ABC";
  await api.openExternal(url, "ZOTERO");
  expect(invoke).toHaveBeenLastCalledWith("open_external", { url, confirmedScheme: "ZOTERO" });
});

it.each([
  ["HtTp://example.org", "HTTP"],
  ["HTTPS://example.org", "Https"],
  ["Mailto:a@b.c", "MAILTO"],
])("requires matching consent before native IPC for %s", async (url, consent) => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  const api = (await import("./backend")).backend();
  await expect(api.openExternal(url)).rejects.toThrow("confirmation");
  await expect(api.openExternal(url, "zotero")).rejects.toThrow("confirmation");
  expect(invoke).not.toHaveBeenCalled();
  await api.openExternal(url, consent);
  expect(invoke).toHaveBeenCalledExactlyOnceWith("open_external", { url, confirmedScheme: consent });
});

const reader = () => publishedBackend(async (): Promise<PublishedSnapshot> => {
  throw new Error("opening an external link does not load the graph");
});

const browsers = [
  { name: "published reader", create: reader, rel: "noopener" },
  { name: "browser preview", create: mockBackend, rel: "noreferrer" },
];

for (const { name, create, rel } of browsers) {
  it(`${name} refuses unsafe URLs and requires exact ASCII case-insensitive consent`, async () => {
    const open = vi.fn();
    vi.stubGlobal("window", { open, location: { href: "https://notes.example/export/index.html" } });
    const api = create();
    const refused = [
      ["javascript:assets/refused", "javascript"],
      [" java\tscript:assets/refused"],
      ["JaVaScRiPt:assets/refused"],
      ["vbscript:example"],
      ["data:text/html,hello"],
      ["blob:https://example.org/id"],
      ["about:blank"],
      ["file:///tmp/note.txt", "file"],
      ["tine://today", "tine"],
      ["zotero://select/items/ABC"],
      ["zotero://select/items/ABC", "obsidian"],
      ["keep://example", "\u212Aeep"],
    ] as const;
    const outcomes = await Promise.allSettled(refused.map(([url, consent]) => api.openExternal(url, consent)));
    expect(open).not.toHaveBeenCalled();
    expect(outcomes.map(({ status }) => status)).toEqual(refused.map(() => "rejected"));
    for (const consent of ["zotero", "Zotero", "ZOTERO"]) {
      await api.openExternal("zotero://select/items/ABC", consent);
      expect(open).toHaveBeenLastCalledWith("zotero://select/items/ABC", "_blank", rel);
    }
  });

  it(`${name} preserves web, mail and scheme-less tab destinations`, async () => {
    const open = vi.fn();
    vi.stubGlobal("window", { open, location: { href: "https://notes.example/export/index.html" } });
    const api = create();
    const urls = ["http://example.org", "https://example.org", "mailto:a@b.c", "#section",
      "#/page/Project", "*Heading", "other.html?x=1", "//example.org", "\\\\example.org", "/\\example.org"];
    for (const url of urls) await api.openExternal(url);
    expect(open.mock.calls).toEqual(urls.map((url) => [url, "_blank", rel]));
  });
}
