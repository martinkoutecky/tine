import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// tine#107 review CODE-1 F1/F2: the startup read, catalog loads, broadcasts and
// the user's choice finish in any order; the latest change of language wins.
// Catalog loads are held open by hand so each interleaving is exact.
const held: { tag: string; release: () => Promise<void>; fail: () => void }[] = [];
vi.mock("./index", async (importOriginal) => {
  const real = await importOriginal<typeof import("./index")>();
  return {
    ...real,
    loadLocale: (tag: string) => tag === "en" ? Promise.resolve() : new Promise<void>((resolve, reject) => {
      held.push({
        tag,
        release: async () => { await real.loadLocale(tag); resolve(); },
        fail: () => reject(new Error("held load failed")),
      });
    }),
  };
});
const listeners: ((event: { payload: string }) => void)[] = [];
// Set to hold this window's broadcast `emit` open.
let emitHold: Promise<void> | null = null;
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (_name: string, handler: (event: { payload: string }) => void) => {
    listeners.push(handler);
    return () => {};
  },
  emit: async () => { if (emitHold) await emitHold; },
}));

const { backend } = await import("../backend");
const { activeLocale } = await import("./index");
const { setUiLanguage, startUiLanguage, uiLanguagePreference } = await import("./preference");

// Real timers throughout: module loading and IPC mocks settle in real time, so
// every wait is on an observable state, never a tick count.
const flush = () => new Promise((resolve) => setTimeout(resolve, 20));
const until = (check: () => boolean) => vi.waitFor(() => { if (!check()) throw new Error("not yet"); }, { timeout: 4000, interval: 5 });
// Each startUiLanguage() call subscribes once (one per webview in the app); use the latest.
const broadcast = (payload: string) => listeners.at(-1)!({ payload });
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
/** Run startup past its 500 ms read bound (the read itself stays pending). */
async function pastTheReadBound(started: Promise<void>) {
  await started;
}
async function releaseAll(tag: string) {
  await until(() => held.some((entry) => entry.tag === tag));
  for (const load of held.splice(0).filter((entry) => entry.tag === tag)) await load.release();
  await flush();
}

let native: ReturnType<typeof vi.fn>;
beforeAll(async () => {
  // Load the real catalog once with real timers: the module loader may use timers,
  // and a held load's release then only has to publish the cached bundle.
  await (await vi.importActual<typeof import("./index")>("./index")).loadLocale("zh-CN");
  vi.spyOn(backend(), "getAppString").mockResolvedValue("");
  vi.spyOn(backend(), "setUiLocale").mockResolvedValue();
  await startUiLanguage();
  vi.restoreAllMocks();
});
beforeEach(() => {
  held.length = 0;
  native = vi.fn().mockResolvedValue(undefined);
  vi.spyOn(backend(), "setUiLocale").mockImplementation(native as never);
  vi.spyOn(backend(), "setAppString").mockResolvedValue();
});
afterEach(async () => {
  vi.restoreAllMocks();
});

async function resetToEnglish() {
  // A fresh operation back to the system's language (jsdom/node navigator: en-US).
  broadcast("");
  await flush();
}

describe("the UI language lifecycle", () => {
  it("a fast read waits for the chosen catalog, however slow, before startup settles (F1)", async () => {
    vi.spyOn(backend(), "getAppString").mockResolvedValue("zh-CN");
    let settled = false;
    const started = startUiLanguage().then(() => { settled = true; });
    await until(() => held.some((entry) => entry.tag === "zh-CN"));
    // Well past the 500 ms read bound, the catalog still holds startup.
    await new Promise((resolve) => setTimeout(resolve, 800));
    expect(settled).toBe(false);
    expect(activeLocale()).toBe("en");
    await releaseAll("zh-CN");
    await started;
    expect(activeLocale()).toBe("zh-CN");
    expect(native).toHaveBeenLastCalledWith("zh-CN");
    await resetToEnglish();
  });

  it("an old startup read landing after a broadcast does not undo it (F2)", async () => {
    const read = deferred<string>();
    vi.spyOn(backend(), "getAppString").mockReturnValue(read.promise);
    await pastTheReadBound(startUiLanguage());
    broadcast("zh-CN");
    await flush();
    await releaseAll("zh-CN");
    read.resolve("");
    await flush();
    expect(activeLocale()).toBe("zh-CN");
    expect(uiLanguagePreference()).toBe("zh-CN");
    await resetToEnglish();
  });

  it("an old startup read failing after the user chose does not undo the choice (F2)", async () => {
    const read = deferred<string>();
    vi.spyOn(backend(), "getAppString").mockReturnValue(read.promise);
    await pastTheReadBound(startUiLanguage());
    setUiLanguage("zh-CN");
    await flush();
    await releaseAll("zh-CN");
    read.reject(new Error("late failure"));
    await flush();
    expect(activeLocale()).toBe("zh-CN");
    expect(uiLanguagePreference()).toBe("zh-CN");
    setUiLanguage("");
    await flush();
  });

  it("an older catalog failure after a newer success does not fall back to English (F2)", async () => {
    broadcast("zh-CN");
    broadcast("zh-CN");
    await until(() => held.length === 2);
    const [older, newer] = held.splice(0);
    await newer.release();
    await flush();
    expect(activeLocale()).toBe("zh-CN");
    older.fail();
    await flush();
    expect(activeLocale()).toBe("zh-CN");
    expect(native).toHaveBeenLastCalledWith("zh-CN");
    await resetToEnglish();
  });

  it("startup waits for a broadcast that superseded its read (CODE-2 A)", async () => {
    const read = deferred<string>();
    vi.spyOn(backend(), "getAppString").mockReturnValue(read.promise);
    let settled = false;
    const started = startUiLanguage().then(() => { settled = true; });
    await flush();
    broadcast("zh-CN");
    await until(() => held.some((entry) => entry.tag === "zh-CN"));
    read.resolve("");
    await flush();
    // The retired read is inert, and startup still waits for the current catalog.
    expect(settled).toBe(false);
    expect(activeLocale()).toBe("en");
    await releaseAll("zh-CN");
    await started;
    expect(activeLocale()).toBe("zh-CN");
    await resetToEnglish();
  });

  it("startup does not wait for a catalog a broadcast made obsolete (CODE-2 A)", async () => {
    vi.spyOn(backend(), "getAppString").mockResolvedValue("zh-CN");
    let settled = false;
    const started = startUiLanguage().then(() => { settled = true; });
    await until(() => held.some((entry) => entry.tag === "zh-CN"));
    broadcast("");
    await started;
    expect(settled).toBe(true);
    expect(activeLocale()).toBe("en");
    expect(native).toHaveBeenLastCalledWith("en");
    await releaseAll("zh-CN");
    expect(activeLocale()).toBe("en");
  });

  it("an older local write failing after a broadcast does not roll back over it (CODE-2 B)", async () => {
    const write = deferred<void>();
    vi.spyOn(backend(), "setAppString").mockReturnValue(write.promise);
    setUiLanguage("en");
    await flush();
    broadcast("zh-CN");
    await releaseAll("zh-CN");
    expect(activeLocale()).toBe("zh-CN");
    write.reject(new Error("disk full"));
    await flush();
    expect(activeLocale()).toBe("zh-CN");
    expect(uiLanguagePreference()).toBe("zh-CN");
    expect(native).toHaveBeenLastCalledWith("zh-CN");
    await resetToEnglish();
  });

  it("a failed save shows what the settings file holds, not an older local value (CODE-3 B2)", async () => {
    const toasts = await import("../toasts");
    const emitted = deferred<void>();
    emitHold = emitted.promise;
    try {
      setUiLanguage("en");
      await flush();
      // Another window stores and broadcasts Chinese while this window's emit is held.
      vi.spyOn(backend(), "getAppString").mockResolvedValue("zh-CN");
      broadcast("zh-CN");
      await releaseAll("zh-CN");
      emitted.resolve();
      await flush();
    } finally {
      emitHold = null;
    }
    vi.spyOn(backend(), "setAppString").mockRejectedValue(new Error("disk full"));
    setUiLanguage("en");
    await flush();
    await releaseAll("zh-CN");
    expect(activeLocale()).toBe("zh-CN");
    expect(uiLanguagePreference()).toBe("zh-CN");
    expect(toasts.toasts().some((toast) => toast.key.startsWith("toast-language-save-failed:"))).toBe(true);
    await resetToEnglish();
  });

  it("only the latest operation reports to native code (F4)", async () => {
    broadcast("zh-CN");
    await flush();
    broadcast("");
    await flush();
    await releaseAll("zh-CN");
    expect(activeLocale()).toBe("en");
    expect(native.mock.calls.map(([tag]) => tag)).toEqual(["en"]);
  });
});
