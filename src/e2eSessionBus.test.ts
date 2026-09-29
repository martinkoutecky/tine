import { afterEach, describe, expect, it, vi } from "vitest";
import { spawnSync } from "node:child_process";
import { ensurePrivateSessionBus, privateSessionLaunch } from "../scripts/lib/e2e-session-bus.mjs";

vi.mock("node:child_process", () => ({ spawnSync: vi.fn() }));
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
  vi.mocked(spawnSync).mockReset();
});

function platform(name: string) {
  vi.stubGlobal("process", { ...process, platform: name });
}

describe("private native E2E session bus", () => {
  it("preserves script arguments and fixture env while marking a private session", () => {
    const launch = privateSessionLaunch("/tmp/journey with spaces.mjs", ["--probe", "a b"], {
      DISPLAY: ":55", XDG_DATA_HOME: "/tmp/fixture", DBUS_RUN_SESSION: "/tmp/dbus runner",
    });
    expect(launch.command).toBe("/tmp/dbus runner");
    expect(launch.args).toEqual(["--", process.execPath, "/tmp/journey with spaces.mjs", "--probe", "a b"]);
    expect(launch.env).toEqual({
      DISPLAY: ":55", XDG_DATA_HOME: "/tmp/fixture", DBUS_RUN_SESSION: "/tmp/dbus runner",
      TINE_E2E_PRIVATE_SESSION_BUS: "1",
    });
  });

  it.each([undefined, "unix:path=/host/session-bus"])("isolates direct Linux runs even with host bus %s", (hostBus) => {
    platform("linux");
    vi.stubEnv("TINE_E2E_PRIVATE_SESSION_BUS", "");
    vi.stubEnv("DBUS_SESSION_BUS_ADDRESS", hostBus);
    vi.mocked(spawnSync).mockReturnValue({ status: 7 } as ReturnType<typeof spawnSync>);
    const exit = vi.spyOn(process, "exit").mockImplementation(() => { throw new Error("wrapper exited"); });
    expect(() => ensurePrivateSessionBus()).toThrow("wrapper exited");
    expect(spawnSync).toHaveBeenCalledOnce();
    expect(vi.mocked(spawnSync).mock.calls[0][2]).toMatchObject({
      stdio: "inherit", env: { TINE_E2E_PRIVATE_SESSION_BUS: "1" },
    });
    expect(exit).toHaveBeenCalledWith(7);
  });

  it("reuses a runner-owned bus and rejects a broken private-session marker", () => {
    platform("linux");
    vi.stubEnv("TINE_E2E_PRIVATE_SESSION_BUS", "1");
    vi.stubEnv("DBUS_SESSION_BUS_ADDRESS", "unix:path=/fixture/bus");
    ensurePrivateSessionBus();
    expect(spawnSync).not.toHaveBeenCalled();
    vi.stubEnv("DBUS_SESSION_BUS_ADDRESS", "");
    expect(() => ensurePrivateSessionBus()).toThrow("no DBUS_SESSION_BUS_ADDRESS");
  });

  it("reports a missing bus launcher before starting the scenario", () => {
    platform("linux");
    vi.stubEnv("TINE_E2E_PRIVATE_SESSION_BUS", "");
    vi.mocked(spawnSync).mockReturnValue({ error: new Error("ENOENT") } as ReturnType<typeof spawnSync>);
    expect(() => ensurePrivateSessionBus()).toThrow("needs dbus-run-session");
  });

  it.each(["win32", "darwin", "android"])("leaves %s transports alone", (name) => {
    platform(name);
    ensurePrivateSessionBus();
    expect(spawnSync).not.toHaveBeenCalled();
  });
});
