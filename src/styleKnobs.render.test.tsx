import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "./backend";
import { applyCustomCss, setCustomCssDisabled } from "./customCss";
import { Settings } from "./components/Settings";
import { StyleKnobsSettings } from "./components/StyleKnobsSettings";
import { CUSTOM_CSS_STYLE_ID, LS_SHIM_STYLE_ID } from "./lsShim";
import { STYLE_KNOBS_KEY, STYLE_KNOBS_STYLE_ID, initStyleKnobs, styleKnobValues, type StyleKnob } from "./styleKnobs";
import { THEME_GALLERY_STYLE_ID, ensureThemeStyle } from "./themeGallery";
import { closeSettings, openSettings } from "./ui";
import { setToasts, toasts } from "./toasts";

// Style knobs (GH #610 item 3): the DOM half. K3 untouched emits nothing, K2
// hostile stored values emit nothing, K4 safe mode empties the layer, and the
// layer sits after #tine-theme and before #tine-custom-css so custom.css wins.

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));
const knobsCss = () => document.getElementById(STYLE_KNOBS_STYLE_ID)?.textContent;
const htmlClasses = () => [...document.documentElement.classList].filter((c) => c.startsWith("tine-"));

async function start(stored: string): Promise<void> {
  vi.spyOn(backend(), "getAppString").mockImplementation(async (key, fallback) => (key === STYLE_KNOBS_KEY ? stored : fallback));
  await initStyleKnobs();
}

beforeEach(() => {
  document.head.innerHTML = "";
  vi.spyOn(backend(), "setAppString").mockResolvedValue(undefined);
});

afterEach(async () => {
  setCustomCssDisabled(false);
  await start("");
  applyCustomCss("");
  closeSettings();
  document.body.innerHTML = "";
  document.head.innerHTML = "";
  setToasts([]);
  vi.restoreAllMocks();
});

describe("#tine-knobs layer", () => {
  it("K3: with nothing stored the layer is empty and the html has no knob class", async () => {
    await start("");
    expect(knobsCss()).toBe("");
    expect(htmlClasses()).toEqual([]);
  });

  it("paints stored choices: variables on :root and the declared class on <html>", async () => {
    await start(JSON.stringify({ "external-link-underline": "none", "embed-shading": "accent" }));
    expect(knobsCss()).toBe(":root { --tine-external-link-underline: none; }");
    expect(htmlClasses()).toEqual(["tine-embed-shading-accent"]);
  });

  it("K2: a hostile stored value emits nothing at all", async () => {
    await start(JSON.stringify({
      "external-link-underline": "1px; } body { display:none",
      "missing-page-color": "red; background: url(https://example.com/x)",
      "embed-shading": "accent evil",
      "not-a-knob": "} body { display:none",
    }));
    expect(knobsCss()).toBe("");
    expect(htmlClasses()).toEqual([]);
  });

  it("an unreadable stored value leaves the defaults and does not throw", async () => {
    await start("{not json");
    expect(knobsCss()).toBe("");
  });

  it("a failed read toasts and leaves the defaults", async () => {
    vi.spyOn(backend(), "getAppString").mockRejectedValue(new Error("disk"));
    await initStyleKnobs();
    expect(knobsCss()).toBe("");
    expect(toasts().some((t) => t.message.includes("style settings"))).toBe(true);
  });

  it("sits after #tine-theme and before #tine-custom-css, whichever exists first", async () => {
    applyCustomCss("a { color: red }");
    await start(JSON.stringify({ "external-link-underline": "none" }));
    const ids = () => [...document.head.children].map((el) => el.id).filter((id) => [LS_SHIM_STYLE_ID, THEME_GALLERY_STYLE_ID, STYLE_KNOBS_STYLE_ID, CUSTOM_CSS_STYLE_ID].includes(id));
    expect(ids()).toEqual([LS_SHIM_STYLE_ID, THEME_GALLERY_STYLE_ID, STYLE_KNOBS_STYLE_ID, CUSTOM_CSS_STYLE_ID]);
    ensureThemeStyle();
    applyCustomCss("a { color: blue }");
    expect(ids()).toEqual([LS_SHIM_STYLE_ID, THEME_GALLERY_STYLE_ID, STYLE_KNOBS_STYLE_ID, CUSTOM_CSS_STYLE_ID]);
  });

  it("K4: Disable custom CSS silences the knobs too, and switching it back on restores them", async () => {
    await start(JSON.stringify({ "external-link-underline": "none", "embed-shading": "accent" }));
    setCustomCssDisabled(true);
    expect(knobsCss()).toBe("");
    expect(htmlClasses()).toEqual([]);
    setCustomCssDisabled(false);
    expect(knobsCss()).toBe(":root { --tine-external-link-underline: none; }");
    expect(htmlClasses()).toEqual(["tine-embed-shading-accent"]);
  });
});

describe("Settings: Fine-tuning control", () => {
  function mount(knobs?: readonly StyleKnob[]) {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <StyleKnobsSettings knobs={knobs} />, root);
    return { root, dispose };
  }
  const row = (root: HTMLElement, label: string) => root.querySelector<HTMLElement>(`[data-setting-label="${label}"]`)!;

  it("choosing an option applies at once, saves on this device, and Reset undoes it", async () => {
    await start("");
    const { root, dispose } = mount();
    const underline = row(root, "Links to the web or a file");
    expect(underline.querySelector(".knob-reset")).toBeNull();
    const select = underline.querySelector("select")!;
    select.value = "none";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    expect(knobsCss()).toBe(":root { --tine-external-link-underline: none; }");
    await vi.waitFor(() => expect(backend().setAppString).toHaveBeenCalledWith(STYLE_KNOBS_KEY, JSON.stringify({ "external-link-underline": "none" })));
    const reset = row(root, "Links to the web or a file").querySelector<HTMLButtonElement>(".knob-reset")!;
    reset.click();
    expect(knobsCss()).toBe("");
    await vi.waitFor(() => expect(backend().setAppString).toHaveBeenLastCalledWith(STYLE_KNOBS_KEY, "{}"));
    dispose();
  });

  it("choosing the default option stores nothing", async () => {
    await start(JSON.stringify({ "embed-shading": "accent" }));
    const { root, dispose } = mount();
    const select = row(root, "Embedded blocks").querySelector("select")!;
    expect(select.value).toBe("accent");
    select.value = "shade";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    expect(htmlClasses()).toEqual([]);
    await vi.waitFor(() => expect(backend().setAppString).toHaveBeenLastCalledWith(STYLE_KNOBS_KEY, "{}"));
    dispose();
  });

  it("keeps a stored choice for a knob this build does not know", async () => {
    await start(JSON.stringify({ "retired-knob": "x" }));
    const { root, dispose } = mount();
    const select = row(root, "Links to the web or a file").querySelector("select")!;
    select.value = "none";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await vi.waitFor(() => expect(backend().setAppString).toHaveBeenLastCalledWith(STYLE_KNOBS_KEY,
      JSON.stringify({ "retired-knob": "x", "external-link-underline": "none" })));
    dispose();
  });

  it("a failed save rolls back and toasts", async () => {
    await start("");
    vi.mocked(backend().setAppString).mockRejectedValue(new Error("disk"));
    const { root, dispose } = mount();
    const select = row(root, "Links to the web or a file").querySelector("select")!;
    select.value = "none";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    await vi.waitFor(() => expect(toasts().some((t) => t.message.includes("Could not save style settings"))).toBe(true));
    expect(knobsCss()).toBe("");
    dispose();
  });

  it("renders one control per type and each choice lands in the stored values", async () => {
    await start("");
    const every: StyleKnob[] = [
      { id: "group", type: "heading", level: 2, title: "Group" },
      { id: "flag", type: "class-toggle", title: "Flag" },
      { id: "mode", type: "class-select", title: "Mode", default: "plain", options: [{ label: "Plain", value: "plain" }, { label: "Fancy", value: "fancy" }] },
      { id: "gap", type: "variable-number-slider", title: "Gap", default: 4, min: 0, max: 20, step: 1, format: "px" },
      { id: "line", type: "variable-select", title: "Line", default: "1px solid", options: [{ label: "Solid", value: "1px solid" }, { label: "None", value: "none" }] },
      { id: "tint", type: "variable-color", title: "Tint", default: "#336699", opacity: true },
    ];
    const { root, dispose } = mount(every);
    expect(root.querySelector('[data-knob-heading="group"]')?.textContent).toBe("Group");

    row(root, "Flag").querySelector<HTMLButtonElement>('button[role="switch"]')!.click();
    expect(styleKnobValues().flag).toBe(true);

    const mode = row(root, "Mode").querySelector("select")!;
    mode.value = "fancy";
    mode.dispatchEvent(new Event("change", { bubbles: true }));
    expect(styleKnobValues().mode).toBe("fancy");

    const gap = row(root, "Gap").querySelector<HTMLInputElement>('input[type="range"]')!;
    gap.value = "12";
    gap.dispatchEvent(new Event("input", { bubbles: true }));
    expect(styleKnobValues().gap).toBe(12);

    const line = row(root, "Line").querySelector("select")!;
    line.value = "none";
    line.dispatchEvent(new Event("change", { bubbles: true }));
    expect(styleKnobValues().line).toBe("none");

    const tint = row(root, "Tint").querySelector<HTMLInputElement>('input[type="color"]')!;
    tint.value = "#ff0000";
    tint.dispatchEvent(new Event("input", { bubbles: true }));
    expect(styleKnobValues().tint).toBe("#ff0000");
    const alpha = row(root, "Tint").querySelector<HTMLInputElement>('input[type="range"]')!;
    alpha.value = "128";
    alpha.dispatchEvent(new Event("input", { bubbles: true }));
    expect(styleKnobValues().tint).toBe("#ff000080");

    await tick();
    dispose();
  });

  it("Settings > Appearance keeps Fine-tuning and Custom CSS inside Advanced", async () => {
    await start("");
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <Settings />, root);
    openSettings("appearance");
    await tick();
    expect(root.textContent).not.toContain("Fine-tuning");
    root.querySelector<HTMLButtonElement>(".settings-advanced-toggle")!.click();
    expect(root.querySelector(".settings-advanced-body")?.textContent).toContain("Fine-tuning");
    expect(root.querySelector(".settings-advanced-body")?.textContent).toContain("Custom CSS");
    dispose();
  });
});
