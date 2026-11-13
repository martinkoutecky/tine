import { expect, it, vi } from "vitest";

const env = vi.hoisted(() => ({
  stored: "",
  listeners: [] as ((event: { payload: string }) => void)[],
  messages: [] as string[],
  get: vi.fn(),
  set: vi.fn(),
}));
vi.mock("../backend", () => ({ backend: () => ({
  getAppString: env.get,
  setAppString: env.set,
  setUiLocale: async () => {},
}) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (_name: string, handler: (event: { payload: string }) => void) => {
    env.listeners.push(handler);
    return () => {};
  },
  emit: async (_name: string, payload: string) => { env.messages.push(payload); },
}));

it("two language writers converge on storage when broadcasts arrive in reverse write order (CBNyU)", async () => {
  env.get.mockImplementation(async () => env.stored);
  env.set.mockImplementation(async (_key: string, value: string) => { env.stored = value; });
  const a = await import("./preference");
  const localeA = await import("./index");
  await a.startUiLanguage();
  // Separate module instances model the independent signals and write queues in webviews.
  vi.resetModules();
  const b = await import("./preference");
  const localeB = await import("./index");
  await b.startUiLanguage();
  a.setUiLanguage("zh-CN");
  await vi.waitFor(() => expect(env.messages).toEqual(["zh-CN"]));
  b.setUiLanguage("en");
  await vi.waitFor(() => expect(env.messages).toEqual(["zh-CN", "en"]));
  expect(env.stored).toBe("en");
  const reads = env.get.mock.calls.length;
  for (const payload of [...env.messages].reverse()) {
    for (const listener of env.listeners) listener({ payload });
  }
  await vi.waitFor(() => {
    expect(a.uiLanguagePreference()).toBe(env.stored);
    expect(b.uiLanguagePreference()).toBe(env.stored);
    expect(localeA.activeLocale()).toBe(env.stored);
    expect(localeB.activeLocale()).toBe(env.stored);
  });
  expect(env.get.mock.calls.length - reads).toBe(4);
  expect(env.set).toHaveBeenCalledTimes(2);
});
