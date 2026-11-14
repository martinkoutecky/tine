import { afterEach, expect, it, vi } from "vitest";

const env = vi.hoisted(() => ({ channel: "beta", get: vi.fn(), set: vi.fn(), toast: vi.fn(), emit: vi.fn(), handlers: [] as Array<(event: { payload: { origin: string } }) => void> }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (_name: string, handler: (event: { payload: { origin: string } }) => void) => { env.handlers.push(handler); return () => {}; },
  emit: env.emit,
}));
vi.mock("../appIdentity", () => ({ get APP_UPDATE_CHANNEL() { return env.channel; } }));
vi.mock("../backend", () => ({ backend: () => ({ getAppBool: env.get, setAppBool: env.set }) }));
vi.mock("../toasts", () => ({ pushToast: env.toast }));
const tick = () => new Promise(resolve => setTimeout(resolve, 0));

// The module registers its change listener after a dynamic import; under a loaded
// suite that can land after the next test began, so wait for it before resetting.
afterEach(async () => {
  await vi.waitFor(() => expect(env.handlers.length).toBeGreaterThan(0), { timeout: 10_000 });
  vi.resetModules(); vi.clearAllMocks(); env.handlers.length = 0;
});

it("defaults to CM6, loads the device choice and rolls back a failed save", async () => {
  env.channel = "beta";
  env.get.mockResolvedValue(true);
  env.set.mockResolvedValue(undefined);
  const setting = await import("./cm6Setting");
  expect(setting.cm6EditorEnabled()).toBe(true);
  setting.initCm6EditorSetting();
  await tick();
  expect(env.get).toHaveBeenCalledWith("experimental_cm6_editor", true);
  expect(setting.cm6EditorEnabled()).toBe(true);
  env.set.mockRejectedValueOnce(new Error("disk full"));
  setting.setCm6EditorEnabled(false);
  expect(setting.cm6EditorEnabled()).toBe(false);
  await tick();
  expect(setting.cm6EditorEnabled()).toBe(true);
  expect(env.toast).toHaveBeenCalled();
});

it("does not let an older startup read undo a user's choice", async () => {
  env.channel = "beta";
  let finish!: (value: boolean) => void;
  env.get.mockReturnValue(new Promise<boolean>(resolve => { finish = resolve; }));
  env.set.mockResolvedValue(undefined);
  const setting = await import("./cm6Setting");
  setting.initCm6EditorSetting();
  setting.setCm6EditorEnabled(true);
  finish(false);
  await tick();
  expect(setting.cm6EditorEnabled()).toBe(true);
});

it("stable loads the stored choice and saves an explicit classic choice", async () => {
  env.channel = "stable";
  env.get.mockResolvedValue(true);
  const setting = await import("./cm6Setting");
  await setting.initCm6EditorSetting();
  expect(setting.cm6EditorEnabled()).toBe(true);
  setting.setCm6EditorEnabled(false);
  await tick();
  expect(setting.cm6EditorEnabled()).toBe(false);
  expect(env.get).toHaveBeenCalledWith("experimental_cm6_editor", true);
  expect(env.set).toHaveBeenCalledWith("experimental_cm6_editor", false);
});

it("settles only after the stored choice has landed, so a window that mounts an editor first can wait for it", async () => {
  env.channel = "beta";
  let finish!: (value: boolean) => void;
  env.get.mockReturnValue(new Promise<boolean>(resolve => { finish = resolve; }));
  const setting = await import("./cm6Setting");
  let landed = false;
  const loading = setting.initCm6EditorSetting().then(() => { landed = true; });
  await tick();
  expect(landed).toBe(false);
  expect(setting.cm6EditorEnabled()).toBe(true);
  finish(true);
  await loading;
  expect(setting.cm6EditorEnabled()).toBe(true);

  env.get.mockRejectedValue(new Error("unreadable"));
  await setting.initCm6EditorSetting();
  expect(setting.cm6EditorEnabled()).toBe(true);
});

it("a choice made in another window reaches this one, and this window's own choice is announced", async () => {
  env.channel = "beta";
  env.get.mockResolvedValue(false);
  env.set.mockResolvedValue(undefined);
  env.emit.mockResolvedValue(undefined);
  const setting = await import("./cm6Setting");
  await setting.initCm6EditorSetting();
  await tick();
  expect(setting.cm6EditorEnabled()).toBe(false);
  await vi.waitFor(() => expect(env.handlers).toHaveLength(1)); // the listener registers after a dynamic import

  // Another window turned it on and wrote the file.
  env.get.mockResolvedValue(true);
  env.handlers[0]({ payload: { origin: "another-window" } });
  await vi.waitFor(() => expect(setting.cm6EditorEnabled()).toBe(true));

  // This window's own change is announced once the file is written, with its origin,
  // and its own echo does not trigger a re-read.
  setting.setCm6EditorEnabled(false);
  // The announcement also goes through a dynamic import, so it can take more than a tick.
  await vi.waitFor(() => expect(env.emit).toHaveBeenCalledWith("cm6-editor-setting-changed", { origin: expect.any(String) }));
  expect(env.set).toHaveBeenCalledWith("experimental_cm6_editor", false);
  const origin = env.emit.mock.calls[0][1].origin;
  const reads = env.get.mock.calls.length;
  env.handlers[0]({ payload: { origin } });
  await tick();
  expect(env.get.mock.calls.length).toBe(reads);
  expect(setting.cm6EditorEnabled()).toBe(false);
});
