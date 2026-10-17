import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";

// The platform probe resolves a little after the settings open, as on a real
// desktop where it is an IPC round trip: Developer tools mounts late (GH #706).
vi.mock("../platform", () => ({
  platformKind: () => new Promise<"desktop">((resolve) => setTimeout(() => resolve("desktop"), 40)),
}));

const { Settings } = await import("./Settings");
const { closeSettings, openSettings } = await import("../ui");
const { setActiveLocale } = await import("../i18n");

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

afterEach(() => {
  void setActiveLocale("en");
  closeSettings();
  document.body.innerHTML = "";
  localStorage.clear();
});

describe("Settings search result for an async-mounted setting (GH #706)", () => {
  it.each([["en", "developer tools", "Developer tools"], ["zh-CN", "开发者工具", "开发者工具"]] as const)(
    "scrolls to Developer tools once the platform probe resolves (%s)",
    async (locale, query, label) => {
      await setActiveLocale(locale);
      const root = document.createElement("div");
      document.body.append(root);
      const dispose = render(() => <Settings />, root);
      const scroll = vi.spyOn(HTMLElement.prototype, "scrollIntoView");
      openSettings("appearance");
      await wait(0);
      const search = root.querySelector<HTMLInputElement>(".settings-search-input")!;
      search.value = query;
      search.dispatchEvent(new InputEvent("input", { bubbles: true }));
      await wait(0);
      const result = [...root.querySelectorAll<HTMLButtonElement>(".settings-search-result")].find((button) => button.textContent?.includes(label));
      expect(result).toBeDefined();
      result!.click();
      await wait(0);
      expect(root.querySelector(`[data-setting-label="${label}"]`)).toBeNull(); // not mounted yet
      await wait(200);
      const target = root.querySelector<HTMLElement>(`[data-setting-label="${label}"]`);
      expect(target).not.toBeNull();
      expect(scroll.mock.contexts).toContain(target);
      scroll.mockRestore();
      dispose();
    },
  );

  it("gives up silently when the target never mounts", async () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    const scroll = vi.spyOn(HTMLElement.prototype, "scrollIntoView");
    openSettings("appearance");
    await wait(0);
    const search = root.querySelector<HTMLInputElement>(".settings-search-input")!;
    search.value = "no-such-setting-706";
    search.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await wait(0);
    expect(root.querySelector(".settings-search-empty")).not.toBeNull();
    expect(scroll).not.toHaveBeenCalled();
    scroll.mockRestore();
    dispose();
  });
});
