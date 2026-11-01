import { afterEach, expect, it, vi } from "vitest";

const tick = () => new Promise(resolve => setTimeout(resolve, 0));
afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); });

it("GH #824: Editor exposes one searchable numeric hover delay and persists valid seconds", async () => {
  vi.resetModules();
  const { render } = await import("solid-js/web");
  const { backend } = await import("../backend");
  const { Settings } = await import("./Settings");
  const ui = await import("../ui");
  const saved = vi.spyOn(backend(), "setAppString").mockResolvedValue();
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Settings />, root);
  ui.openSettings("editor");
  await tick();
  const search = root.querySelector<HTMLInputElement>(".settings-search-input")!;
  search.value = "hover preview delay";
  search.dispatchEvent(new InputEvent("input", { bubbles: true }));
  await tick();
  try {
    expect(root.querySelector(".settings-search-result")?.textContent).toContain("Hover preview delay");
    root.querySelector<HTMLButtonElement>(".settings-search-result")!.click();
    await tick();
    const input = root.querySelector<HTMLInputElement>('input[aria-label="Hover preview delay (seconds)"]');
    expect(input).not.toBeNull();
    expect(input!.value).toBe("1");
    for (const value of ["0.25", "100", "1e30", "0"]) {
      input!.value = value;
      input!.dispatchEvent(new Event("change", { bubbles: true }));
      await tick();
      expect(saved).toHaveBeenLastCalledWith("hover_preview_delay_seconds", String(Number(value)));
    }
    const writes = saved.mock.calls.length;
    for (const value of ["", "-1", "1e999"]) {
      input!.value = value;
      input!.dispatchEvent(new Event("change", { bubbles: true }));
      await tick();
    }
    expect(saved).toHaveBeenCalledTimes(writes);
    expect(input!.value).toBe("0");
  } finally { dispose(); }
});

it("GH #824: restores valid values, defaults malformed storage, and rolls back a failed save", async () => {
  vi.resetModules();
  const { backend } = await import("../backend");
  const prefs = await import("../hoverPreviewSettings");
  const read = vi.spyOn(backend(), "getAppString");
  for (const [stored, expected] of [["0", 0], ["0.75", 0.75], ["1e30", 1e30], ["", 1], ["NaN", 1], ["Infinity", 1], ["-1", 1]] as const) {
    read.mockResolvedValue(stored);
    prefs.initHoverPreviewSettings(); await tick();
    expect(prefs.hoverPreviewDelaySeconds()).toBe(expected);
  }
  vi.spyOn(backend(), "setAppString").mockRejectedValue(new Error("disk"));
  prefs.setHoverPreviewDelaySeconds(2.5);
  expect(prefs.hoverPreviewDelaySeconds()).toBe(2.5);
  await tick();
  expect(prefs.hoverPreviewDelaySeconds()).toBe(1);
});
