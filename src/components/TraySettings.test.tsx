// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { setToasts } from "../toasts";
import { TraySettings } from "./TraySettings";

afterEach(() => { document.body.innerHTML = ""; setToasts([]); vi.restoreAllMocks(); });

const supported = { supported: true, active: false, problem: null };

function mount() {
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <TraySettings />, root);
  const toggle = (label: string) =>
    root.querySelector<HTMLButtonElement>(`[data-setting-label="${label}"] button[role="switch"]`)!;
  return { root, dispose, toggle };
}

it("renders nothing where the platform has no tray (mobile, mock)", async () => {
  const apply = vi.spyOn(backend(), "trayApply").mockResolvedValue({ supported: false, active: false, problem: null });
  const { root, dispose } = mount();
  await vi.waitFor(() => expect(apply).toHaveBeenCalled());
  await Promise.resolve();
  expect(root.querySelector("[data-setting-label]")).toBeNull();
  dispose();
});

it("is all off by default and the dependent controls are disabled until the icon is on", async () => {
  vi.spyOn(backend(), "trayApply").mockResolvedValue(supported);
  vi.spyOn(backend(), "getAppBool").mockResolvedValue(false);
  const { toggle, dispose } = mount();
  await vi.waitFor(() => expect(toggle("Show Tine in the system tray")).not.toBeNull());
  expect(toggle("Show Tine in the system tray").getAttribute("aria-checked")).toBe("false");
  expect(toggle("Minimize to tray").disabled).toBe(true);
  expect(toggle("Start minimized to tray").disabled).toBe(true);
  dispose();
});

it("turning the icon on saves the key, asks native to create it, and enables the other two", async () => {
  const apply = vi.spyOn(backend(), "trayApply").mockResolvedValue({ ...supported, active: true });
  vi.spyOn(backend(), "getAppBool").mockResolvedValue(false);
  const set = vi.spyOn(backend(), "setAppBool").mockResolvedValue();
  const { toggle, dispose } = mount();
  await vi.waitFor(() => expect(toggle("Show Tine in the system tray")).not.toBeNull());
  apply.mockClear();
  toggle("Show Tine in the system tray").click();
  await vi.waitFor(() => expect(set).toHaveBeenCalledWith("tray_show", true));
  await vi.waitFor(() => expect(apply).toHaveBeenCalledTimes(1));
  expect(toggle("Minimize to tray").disabled).toBe(false);
  expect(toggle("Start minimized to tray").disabled).toBe(false);
  toggle("Minimize to tray").click();
  await vi.waitFor(() => expect(set).toHaveBeenCalledWith("tray_minimize", true));
  toggle("Start minimized to tray").click();
  await vi.waitFor(() => expect(set).toHaveBeenCalledWith("tray_start_minimized", true));
  dispose();
});

it("tray unavailable: shows the one-line note and never claims the icon is active", async () => {
  vi.spyOn(backend(), "getAppBool").mockImplementation(async (key) => key === "tray_show");
  vi.spyOn(backend(), "trayApply").mockResolvedValue({
    supported: true,
    active: false,
    problem: "No system tray was found on this desktop, so the tray options are off for now.",
  });
  const { root, dispose } = mount();
  await vi.waitFor(() => expect(root.querySelector("[data-tray-note]")?.textContent).toContain("No system tray was found"));
  dispose();
});
