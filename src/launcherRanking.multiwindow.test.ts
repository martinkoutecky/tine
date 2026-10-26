import { expect, it, vi } from "vitest";

const bus = vi.hoisted(() => ({
  listeners: new Set<(event: { payload: { origin: string } }) => void>(),
  stored: false,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_name, callback) => { bus.listeners.add(callback); return () => bus.listeners.delete(callback); }),
  emit: vi.fn(async (_name, payload) => { for (const listener of bus.listeners) listener({ payload }); }),
}));
vi.mock("./backend", () => ({ backend: () => ({
  getAppString: async () => "en",
  getAppBool: async () => bus.stored,
  setAppBool: async (key: string, value: boolean) => { if (key === "pinyin-page-matching") bus.stored = value; },
}) }));

it("propagates persisted Pinyin changes between two already-open WebView modules", async () => {
  const main = await import("./launcherRanking");
  await main.initPinyinMatchingSetting();
  vi.resetModules();
  const capture = await import("./launcherRanking");
  await capture.initPinyinMatchingSetting();
  expect(main.pinyinMatchingEnabled()).toBe(false);
  expect(capture.pinyinMatchingEnabled()).toBe(false);
  main.setPinyinMatchingEnabled(true);
  await vi.waitFor(() => expect(capture.pinyinMatchingEnabled()).toBe(true));
  capture.setPinyinMatchingEnabled(false);
  await vi.waitFor(() => expect(main.pinyinMatchingEnabled()).toBe(false));
  // A missed event while Capture was hidden is reconciled on show.
  bus.stored = true;
  await capture.initPinyinMatchingSetting();
  expect(capture.pinyinMatchingEnabled()).toBe(true);
});
