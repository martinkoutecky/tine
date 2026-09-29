import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { closeSettings, openSettings } from "../ui";
import {
  changeWideContentWidth,
  resetStandardContentWidth,
  standardContentWidth,
  wideContentWidth,
} from "../contentWidth";

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

afterEach(() => {
  vi.restoreAllMocks();
  vi.spyOn(backend(), "setAppString").mockResolvedValue();
  closeSettings();
  document.body.innerHTML = "";
  resetStandardContentWidth();
  changeWideContentWidth(null);
  vi.restoreAllMocks();
});

// GH #382: the page column's reading width is device-local and tunable; the
// theme keeps ownership of the default until the user overrides it.
describe("Settings page width (GH #382)", () => {
  it("finds and applies device-local standard and wide page widths", async () => {
    const saved = vi.spyOn(backend(), "setAppString").mockResolvedValue();
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("appearance");
    await tick();

    const search = root.querySelector<HTMLInputElement>(".settings-search-input")!;
    search.value = "standard page width";
    search.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await tick();
    const result = root.querySelector<HTMLButtonElement>(".settings-search-result")!;
    expect(result.textContent).toContain("Appearance › Advanced");
    result.click();
    await tick();

    const standard = root.querySelector<HTMLInputElement>('input[aria-label="Standard page width in pixels"]')!;
    standard.value = "960";
    standard.dispatchEvent(new Event("change", { bubbles: true }));
    await tick();
    expect(standardContentWidth()).toBe(960);
    expect(saved).toHaveBeenCalledWith("content_width_standard", "960");
    expect(document.documentElement.style.getPropertyValue("--tine-main-content-max-width")).toBe("960px");

    const wideMode = root.querySelector<HTMLSelectElement>('select[aria-label="Wide page width mode"]')!;
    wideMode.value = "custom";
    wideMode.dispatchEvent(new Event("change", { bubbles: true }));
    await tick();
    expect(wideContentWidth()).toBe(1280);
    expect(root.querySelector('input[aria-label="Wide page width in pixels"]')).not.toBeNull();

    wideMode.value = "fill";
    wideMode.dispatchEvent(new Event("change", { bubbles: true }));
    await tick();
    expect(wideContentWidth()).toBeNull();
    expect(saved).toHaveBeenLastCalledWith("content_width_wide", "");
    dispose();
  });
});
