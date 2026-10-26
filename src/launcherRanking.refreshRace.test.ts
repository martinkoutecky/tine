import { expect, it, vi } from "vitest";
const bus = vi.hoisted(() => ({ listeners: new Set<(event: { payload: { origin: string } }) => void>(), stored: false, reads: null as null | Array<(value: boolean) => void>, rejectWrite: null as null | ((err: Error) => void) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_name, cb) => { bus.listeners.add(cb); return () => bus.listeners.delete(cb); }),
  emit: vi.fn(async (_name, payload) => { for (const cb of bus.listeners) cb({ payload }); }),
}));
vi.mock("./backend", () => ({ backend: () => ({
  getAppString: async () => "en",
  getAppBool: async () => bus.reads ? new Promise<boolean>(resolve => bus.reads!.push(resolve)) : bus.stored,
  setAppBool: async (key: string, value: boolean) => {
    if (key !== "pinyin-page-matching") return;
    if (value) return new Promise<void>((_resolve, reject) => { bus.rejectWrite = reject; });
    bus.stored = value;
  },
}) }));

// GH #151 re-review F1: a hidden Capture refreshing on show must not be
// overwritten by an older in-flight read that settles last.
it("a newer preference refresh retires an earlier delayed read", async () => {
  const capture = await import("./launcherRanking");
  await capture.initPinyinMatchingSetting();
  bus.reads = [];
  const older = capture.initPinyinMatchingSetting();
  await vi.waitFor(() => expect(bus.reads).toHaveLength(1));
  const newer = capture.initPinyinMatchingSetting();
  await vi.waitFor(() => expect(bus.reads).toHaveLength(2));
  bus.reads![1](true); await newer;
  expect(capture.pinyinMatchingEnabled()).toBe(true);
  bus.reads![0](false); await older;
  bus.reads = null;
  expect(capture.pinyinMatchingEnabled()).toBe(true);
});
