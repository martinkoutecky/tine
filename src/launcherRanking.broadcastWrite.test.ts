import { beforeEach, expect, it, vi } from "vitest";

// GH #151 re-review F2 (queue card hApslk): another window's change broadcast
// that arrives while this window's own write is still pending retires that
// write's rollback and refuses the re-read. Whatever the write's outcome, this
// window must end on what the settings file holds once its writes settle.
const bus = vi.hoisted(() => ({
  listeners: new Set<(event: { payload: { origin: string } }) => void>(),
  stored: false,
  heldTrue: null as null | { fail: (error: Error) => void; reply: () => void },
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_name, cb) => { bus.listeners.add(cb); return () => bus.listeners.delete(cb); }),
  emit: vi.fn(async (_name, payload) => { for (const cb of bus.listeners) cb({ payload }); }),
}));
vi.mock("./backend", () => ({ backend: () => ({
  getAppString: async () => "en",
  getAppBool: async () => bus.stored,
  // Writing true is held open by hand (its reply is delayed or fails); writing
  // false lands at once.
  setAppBool: async (key: string, value: boolean) => {
    if (key !== "pinyin-page-matching") return;
    if (!value) { bus.stored = false; return; }
    return new Promise<void>((resolve, reject) => {
      bus.heldTrue = { fail: reject, reply: () => resolve() };
    });
  },
}) }));

const settle = () => new Promise(resolve => setTimeout(resolve, 30));

async function twoWindows() {
  vi.resetModules();
  bus.listeners.clear(); bus.stored = false; bus.heldTrue = null;
  const main = await import("./launcherRanking");
  await main.initPinyinMatchingSetting();
  vi.resetModules();
  const other = await import("./launcherRanking");
  await other.initPinyinMatchingSetting();
  return { main, other };
}

beforeEach(() => { bus.heldTrue = null; });

it("a failed local write overlapped by another window's broadcast ends on the stored value", async () => {
  const { main, other } = await twoWindows();
  main.setPinyinMatchingEnabled(true);
  await vi.waitFor(() => expect(bus.heldTrue).not.toBeNull());
  other.setPinyinMatchingEnabled(false);
  await settle();
  bus.heldTrue!.fail(new Error("disk full"));
  await settle();
  expect(bus.stored).toBe(false);
  expect(other.pinyinMatchingEnabled()).toBe(false);
  expect(main.pinyinMatchingEnabled()).toBe(false);
});

it("a local write whose reply lands after another window's newer write ends on the stored value", async () => {
  const { main, other } = await twoWindows();
  main.setPinyinMatchingEnabled(true);
  await vi.waitFor(() => expect(bus.heldTrue).not.toBeNull());
  // main's write reached the file first, but its reply is still in flight when
  // the other window writes false (the later write on disk) and broadcasts.
  other.setPinyinMatchingEnabled(false);
  await settle();
  bus.heldTrue!.reply();
  await settle();
  expect(bus.stored).toBe(false);
  expect(other.pinyinMatchingEnabled()).toBe(false);
  expect(main.pinyinMatchingEnabled()).toBe(false);
});
