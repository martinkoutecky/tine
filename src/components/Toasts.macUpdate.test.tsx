import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { APP_PRODUCT_NAME, APP_UPDATE_CHANNEL } from "../appIdentity";
import { installUpdateForTest, setUpdateExitGuard } from "../update";
import { setToasts } from "../toasts";
import { Toasts } from "./Toasts";

const native = vi.hoisted(() => ({
  install: vi.fn(async () => {}),
  openExternal: vi.fn(async () => {}),
  relaunch: vi.fn(async () => {}),
}));
vi.mock("../backend", () => ({
  isTauri: () => true,
  backend: () => ({
    gpuEnv: async () => ({ packaged: false, flatpak: false, portable: false }),
    appArchitecture: async () => "aarch64",
    openExternal: native.openExternal,
  }),
}));
vi.mock("../platform", () => ({ platformKind: async () => "desktop" }));
vi.mock("../debug", () => ({ dbg: vi.fn(), recordDiagnostic: vi.fn(async () => {}) }));
vi.mock("../ui", () => ({ openSettings: vi.fn() }));
// The offer matches the channel this tree ships: a stable build refuses a Beta version.
vi.mock("@tauri-apps/plugin-updater", () => ({ check: async () => ({
  version: APP_UPDATE_CHANNEL === "beta" ? "0.8.0-beta.2" : "0.8.1", download: async () => {}, install: native.install, close: async () => {},
}) }));
vi.mock("@tauri-apps/plugin-process", () => ({ relaunch: native.relaunch }));

let dispose = () => {};
afterEach(() => {
  dispose();
  document.body.innerHTML = "";
  setToasts([]);
  setUpdateExitGuard(null);
  vi.unstubAllGlobals();
  vi.resetAllMocks();
});

function mount() {
  vi.stubGlobal("navigator", { userAgent: "Macintosh; Intel Mac OS X" });
  setToasts([]);
  const host = document.createElement("div");
  document.body.append(host);
  dispose = render(() => <Toasts />, host);
  return host;
}

it.each(["accepted", "rejected"] as const)("GH #405: installed macOS update shows Restart and respects a fresh %s save guard", async (answer) => {
  const host = mount();
  const prepare = vi.fn<() => Promise<"accepted" | "rejected">>(async () => "accepted");
  const reset = vi.fn();
  setUpdateExitGuard({ prepare, reset });
  await installUpdateForTest();
  expect(host.textContent).toContain("Restart to use");
  expect(native.install).toHaveBeenCalledOnce();
  expect(reset).toHaveBeenCalledOnce();
  expect(native.relaunch).not.toHaveBeenCalled();
  const button = [...host.querySelectorAll("button")].find((el) => el.textContent === "Restart");
  expect(button).toBeDefined();
  // Unsaved work created after installation can still refuse the restart.
  prepare.mockResolvedValueOnce(answer);
  button!.click();
  await vi.waitFor(() => expect(prepare).toHaveBeenCalledTimes(2));
  if (answer === "accepted") await vi.waitFor(() => expect(native.relaunch).toHaveBeenCalledOnce());
  else expect(native.relaunch).not.toHaveBeenCalled();
});

it("GH #405: rollback failure visibly names the previous bundle and offers Download manually", async () => {
  const host = mount();
  setUpdateExitGuard({ prepare: async () => "accepted", reset: vi.fn() });
  native.install.mockRejectedValueOnce(new Error("failed to install the update and to restore the previous app, which was left at /Applications/private.app"));
  await installUpdateForTest();
  expect(host.textContent).toContain(`${APP_PRODUCT_NAME} (previous version).app`);
  expect(host.textContent).toContain("beside the original app");
  expect(host.textContent).toContain("Your previous app was saved at: /Applications/private.app");
  const button = [...host.querySelectorAll("button")].find((el) => el.textContent === "Download manually");
  expect(button).toBeDefined();
  native.openExternal.mockClear();
  button!.click();
  expect(native.openExternal).toHaveBeenCalledOnce();
});
