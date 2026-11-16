import { afterEach, describe, expect, it, vi } from "vitest";

// GH #796 (D-20261029-ST): "Search results open in" is a device-local choice,
// New tab (the GH #416 behaviour) by default, This tab on request.
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

async function open() {
  vi.resetModules();
  const { render } = await import("solid-js/web");
  const { backend } = await import("../backend");
  const { Settings } = await import("./Settings");
  const ui = await import("../ui");
  const nav = await import("../navSettings");
  nav.resetSearchResultsOpenInThisTabForTests(false);
  const saved = vi.spyOn(backend(), "setAppBool").mockResolvedValue();
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  ui.openSettings("editor");
  await tick();
  // The setting lives behind the Editor tab's Advanced disclosure.
  root.querySelector<HTMLButtonElement>(".settings-advanced-toggle")!.click();
  await tick();
  return { root, dispose, saved, nav, backend };
}

const field = (root: HTMLElement) => root.querySelector<HTMLElement>('[data-setting-label="Search results open in"]');

describe("Settings: Search results open in (GH #796)", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.restoreAllMocks();
  });

  it("defaults to New tab; choosing This tab applies at once and is remembered on this device", async () => {
    const { root, dispose, saved, nav } = await open();
    try {
      const select = field(root)!.querySelector<HTMLSelectElement>("select")!;
      expect([...select.options].map((option) => option.textContent)).toEqual(["New tab", "This tab"]);
      expect(select.value).toBe("tab");
      expect(nav.searchResultsOpenInThisTab()).toBe(false);
      select.value = "here";
      select.dispatchEvent(new Event("change", { bubbles: true }));
      await tick();
      expect(nav.searchResultsOpenInThisTab()).toBe(true);
      expect(saved).toHaveBeenCalledWith("search_results_open_in_this_tab", true);
      select.value = "tab";
      select.dispatchEvent(new Event("change", { bubbles: true }));
      await tick();
      expect(nav.searchResultsOpenInThisTab()).toBe(false);
      expect(saved).toHaveBeenLastCalledWith("search_results_open_in_this_tab", false);
    } finally { dispose(); }
  });

  it("is restored from the device preference at startup", async () => {
    const { dispose, nav, backend } = await open();
    try {
      vi.spyOn(backend(), "getAppBool").mockImplementation(async (key: string, fallback: boolean) => key === "search_results_open_in_this_tab" ? true : fallback);
      await nav.initNavSettings();
      await tick();
      expect(nav.searchResultsOpenInThisTab()).toBe(true);
    } finally { dispose(); }
  });

  it("Settings search finds it", async () => {
    const { root, dispose } = await open();
    try {
      const search = root.querySelector<HTMLInputElement>(".settings-search-input")!;
      search.value = "search tab same tab";
      search.dispatchEvent(new InputEvent("input", { bubbles: true }));
      await tick();
      expect(root.querySelector(".settings-search-result")?.textContent).toContain("Search results open in");
    } finally { dispose(); }
  });
});
