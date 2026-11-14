import { afterEach, expect, it, vi } from "vitest";

const env = vi.hoisted(() => ({ channel: "beta", stored: undefined as boolean | undefined,
  get: vi.fn(), set: vi.fn() }));
vi.mock("../appIdentity", () => ({ get APP_UPDATE_CHANNEL() { return env.channel; } }));
vi.mock("../backend", () => ({ backend: () => ({ getAppBool: env.get, setAppBool: env.set }) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {}, emit: async () => {} }));
afterEach(() => { vi.resetModules(); vi.clearAllMocks(); });

it.each(["beta", "stable"])("a fresh %s profile defaults to CM6 without writing a choice", async channel => {
  env.channel = channel;
  env.stored = undefined;
  env.get.mockImplementation(async (_key, fallback) => env.stored ?? fallback);
  const setting = await import("./cm6Setting");
  expect(setting.cm6EditorEnabled()).toBe(true);
  await setting.initCm6EditorSetting();
  expect(setting.cm6EditorEnabled()).toBe(true);
  expect(env.get).toHaveBeenCalledTimes(1);
  expect(env.set).not.toHaveBeenCalled();
});

it.each(["beta", "stable"])("a stored classic choice is respected in %s, including a delayed startup read", async channel => {
  env.channel = channel;
  let finish!: (choice: boolean) => void;
  env.get.mockReturnValue(new Promise<boolean>(resolve => { finish = resolve; }));
  const setting = await import("./cm6Setting");
  const loading = setting.initCm6EditorSetting();
  expect(setting.cm6EditorLoading()).toBe(true);
  finish(false);
  await loading;
  expect(setting.cm6EditorLoading()).toBe(false);
  expect(setting.cm6EditorEnabled()).toBe(false);
  expect(env.set).not.toHaveBeenCalled();
});
