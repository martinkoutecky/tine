import { afterEach, expect, it, vi } from "vitest";
import { setActiveLocale, t } from "./i18n";
import { errorFamily } from "./errorFamily";

// Card HVgII. A page write refused because config.edn could not be read leaves
// the native side as the wire family `config-unreadable` followed by the read
// error (graph-features `ConfigUnreadable`). The backend call funnel turns it
// into the sentence the open-time notice gives (`ui-failure-config-read`), so
// every write entry that prints its rejection shows the explanation, never
// "invalid utf-8 sequence of 1 bytes from index 35".
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke, convertFileSrc: (path: string) => path }));
afterEach(async () => { await setActiveLocale("en"); vi.unstubAllGlobals(); invoke.mockReset(); });

const WIRE = "config-unreadable: invalid utf-8 sequence of 1 bytes from index 35";
const SHOWN: Record<string, { sentence: string; rename: string }> = {
  en: {
    sentence: "Couldn't read config.edn. The graph is open read-only; repair the config and reopen the graph.",
    rename: "Rename failed: Couldn't read config.edn. The graph is open read-only; repair the config and reopen the graph.",
  },
  "zh-CN": {
    sentence: "无法读取 config.edn。图谱以只读方式打开，请修复配置并重新打开图谱。",
    rename: "重命名失败：无法读取 config.edn。图谱以只读方式打开，请修复配置并重新打开图谱。",
  },
};

async function refusedBackend() {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  invoke.mockImplementation((command: string) =>
    ["rename_page", "merge_pages"].includes(command) ? Promise.reject(WIRE) : Promise.resolve(null));
  const { backend } = await import("./backend");
  return backend();
}

it("a refused rename shows the read-only explanation, in both languages, not the raw read error", async () => {
  const api = await refusedBackend();
  for (const locale of ["en", "zh-CN"]) {
    await setActiveLocale(locale);
    const failure = await api.renamePage("Start", "Begin", "rename-page").catch((error: unknown) => error);
    // What the three rename entries (title, context menu, rename toast) print: `String(error)`.
    const shown = t("page-ui-rename-failed", { value1: String(failure) });
    expect(shown).toBe(SHOWN[locale].rename);
    expect(shown).not.toMatch(/utf-8|config-unreadable/);
  }
});

it("any other refused write through the same funnel gets the same sentence", async () => {
  const api = await refusedBackend();
  const failure = await api.mergePages("a.md", "b.md", ["insert-blocks", "delete-page"]).catch((error: unknown) => error);
  expect(String(failure)).toBe(SHOWN.en.sentence);
  // The refusal is final: nothing downstream branches on the wire family any more.
  expect(errorFamily(failure)).toBe("unknown");
});

it("an unrelated rejection passes through unchanged", async () => {
  vi.stubGlobal("window", { __TAURI_INTERNALS__: {} });
  invoke.mockImplementation(() => Promise.reject("conflict"));
  const { backend } = await import("./backend");
  await expect(backend().mergePages("a.md", "b.md", ["insert-blocks", "delete-page"])).rejects.toBe("conflict");
});
