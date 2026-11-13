import { afterEach, expect, expectTypeOf, it, vi } from "vitest";
import type { GraphMeta } from "../types";

it("receives the effective preview switch as a required boolean from Rust", () => {
  expectTypeOf<GraphMeta["enable_tooltip"]>().toEqualTypeOf<boolean>();
});

const tick = () => new Promise(resolve => setTimeout(resolve, 0));
const flush = async () => { for (let i = 0; i < 6; i++) await tick(); };
afterEach(() => { document.body.replaceChildren(); vi.restoreAllMocks(); });

// GH #824 / D-20261110-TT: Settings → Editor → Advanced, next to the Hover
// preview delay, switches Logseq's `:ui/enable-tooltip?` through the existing
// graph-setting writer (backend setEnableTooltip → config.edn).
it("GH #824: the Reference previews switch sits beside the hover delay, is searchable and writes :ui/enable-tooltip?", async () => {
  vi.resetModules();
  const { render } = await import("solid-js/web");
  const { backend } = await import("../backend");
  const { Settings } = await import("./Settings");
  const ui = await import("../ui");
  const previews = await import("../hoverPreviewSettings");
  const { setGraphMeta, graphMeta } = await import("../graphSession");
  const loaded = await backend().loadGraph("");
  if (loaded.kind === "focused_existing") throw new Error("missing graph");
  setGraphMeta({ ...loaded.meta, root: "/test" });
  const saved = vi.spyOn(backend(), "setEnableTooltip").mockResolvedValue();
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <Settings />, root);
  ui.openSettings("editor");
  await tick();
  try {
    const search = root.querySelector<HTMLInputElement>(".settings-search-input")!;
    search.value = "reference previews";
    search.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await tick();
    expect(root.querySelector(".settings-search-result")?.textContent).toContain("Reference previews");
    root.querySelector<HTMLButtonElement>(".settings-search-result")!.click();
    await tick();
    const field = root.querySelector<HTMLElement>('[data-setting-id="settings-reference-previews"]')!;
    expect(field).not.toBeNull();
    // Beside the hover delay, which stays.
    expect(field.nextElementSibling?.getAttribute("data-setting-id")).toBe("settings-hover-preview-delay");
    const toggle = field.querySelector<HTMLButtonElement>('button[role="switch"]')!;
    // The backend serves the effective value, including the absent-config-key default.
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(previews.previewsEnabled()).toBe(true);

    toggle.click();
    await flush();
    expect(saved).toHaveBeenLastCalledWith(false);
    expect(saved).toHaveBeenCalledTimes(1);
    expect(graphMeta()?.enable_tooltip).toBe(false);
    expect(previews.previewsEnabled()).toBe(false);
    expect(toggle.getAttribute("aria-checked")).toBe("false");

    toggle.click();
    await flush();
    expect(saved).toHaveBeenLastCalledWith(true);
    expect(previews.previewsEnabled()).toBe(true);
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(saved).toHaveBeenCalledTimes(2);
  } finally { dispose(); setGraphMeta(null); }
});

it("GH #824: a refused config write restores the switch and says so", async () => {
  vi.resetModules();
  const { backend } = await import("../backend");
  const previews = await import("../hoverPreviewSettings");
  const { setGraphMeta } = await import("../graphSession");
  const { toasts, setToasts } = await import("../toasts");
  const loaded = await backend().loadGraph("");
  if (loaded.kind === "focused_existing") throw new Error("missing graph");
  setGraphMeta({ ...loaded.meta, root: "/test", enable_tooltip: true });
  vi.spyOn(backend(), "setEnableTooltip").mockRejectedValueOnce(new Error("disk full"));
  try {
    previews.changePreviewsEnabled(false);
    await flush();
    expect(previews.previewsEnabled()).toBe(true);
    expect(toasts().filter(toast => toast.kind === "error")).toHaveLength(1);
  } finally { setGraphMeta(null); setToasts([]); }
});

it("GH #824: a refused change in Chinese Settings shows no English preference label", async () => {
  vi.resetModules();
  const { render } = await import("solid-js/web");
  const { backend } = await import("../backend");
  const { Settings } = await import("./Settings");
  const { Toasts } = await import("./Toasts");
  const { setActiveLocale } = await import("../i18n");
  const { setGraphMeta } = await import("../graphSession");
  const { toasts, setToasts } = await import("../toasts");
  const ui = await import("../ui");
  const loaded = await backend().loadGraph("");
  if (loaded.kind === "focused_existing") throw new Error("missing graph");
  setGraphMeta({ ...loaded.meta, root: "/test", enable_tooltip: true });
  const saved = vi.spyOn(backend(), "setEnableTooltip").mockRejectedValue(new Error("disk full"));
  await setActiveLocale("zh-CN");
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <><Settings /><Toasts /></>, root);
  ui.openSettings("editor");
  try {
    const search = root.querySelector<HTMLInputElement>(".settings-search-input")!;
    search.value = "引用预览";
    search.dispatchEvent(new InputEvent("input", { bubbles: true }));
    await tick();
    root.querySelector<HTMLButtonElement>(".settings-search-result")!.click();
    await tick();
    const toggle = root.querySelector<HTMLButtonElement>('[data-setting-id="settings-reference-previews"] button[role="switch"]')!;
    toggle.click();
    await flush();
    expect(saved).toHaveBeenCalledExactlyOnceWith(false);
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    const text = root.querySelector(".toast-error .toast-msg")!.textContent!;
    expect(text).toContain("引用预览");
    expect(text).not.toMatch(/[a-z]/i);
    expect(toasts().find(toast => toast.kind === "error")?.message).toBe("Could not save reference preview preference.");
    await setActiveLocale("en");
    expect(root.querySelector(".toast-error .toast-msg")!.textContent).toBe("Could not save reference preview preference.");
  } finally { dispose(); setGraphMeta(null); setToasts([]); await setActiveLocale("en"); }
});
