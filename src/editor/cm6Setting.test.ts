import { afterEach, expect, it, vi } from "vitest";

const env = vi.hoisted(() => ({ channel: "beta", get: vi.fn(), set: vi.fn(), toast: vi.fn() }));
vi.mock("../appIdentity", () => ({ get APP_UPDATE_CHANNEL() { return env.channel; } }));
vi.mock("../backend", () => ({ backend: () => ({ getAppBool: env.get, setAppBool: env.set }) }));
vi.mock("../toasts", () => ({ pushToast: env.toast }));
const tick = () => new Promise(resolve => setTimeout(resolve, 0));

afterEach(() => { vi.resetModules(); vi.clearAllMocks(); });

it("defaults off in Beta, loads the device opt-in and rolls back a failed save", async () => {
  env.channel = "beta";
  env.get.mockResolvedValue(true);
  env.set.mockResolvedValue(undefined);
  const setting = await import("./cm6Setting");
  expect(setting.cm6EditorAvailable).toBe(true);
  expect(setting.cm6EditorEnabled()).toBe(false);
  setting.initCm6EditorSetting();
  await tick();
  expect(env.get).toHaveBeenCalledWith("experimental_cm6_editor", false);
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

it("stable refuses stored and programmatic opt-ins without preference IO", async () => {
  env.channel = "stable";
  env.get.mockResolvedValue(true);
  const setting = await import("./cm6Setting");
  setting.initCm6EditorSetting();
  setting.setCm6EditorEnabled(true);
  await tick();
  expect(setting.cm6EditorAvailable).toBe(false);
  expect(setting.cm6EditorEnabled()).toBe(false);
  expect(env.get).not.toHaveBeenCalled();
  expect(env.set).not.toHaveBeenCalled();
});
