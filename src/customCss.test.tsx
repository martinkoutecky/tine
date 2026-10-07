import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "./backend";
import { captureBinding } from "./binding";
import { applyCustomCss, setCustomCssDisabled } from "./customCss";
import { applyCustomCssChange } from "./graph";
import { CUSTOM_CSS_STYLE_ID } from "./lsShim";
import { CustomCssSettings } from "./components/CustomCssSettings";
import { setToasts, toasts } from "./toasts";

const platform = vi.hoisted(() => ({ kind: "desktop" as "desktop" | "android" }));
vi.mock("./platform", () => ({
  platformKind: async () => platform.kind,
  isMobile: async () => platform.kind !== "desktop",
}));

// GH #610: logseq/custom.css is discoverable (Edit custom.css), live (an
// outside edit re-applies without reopening the graph) and recoverable
// (Disable custom CSS is a session-only safe mode).

const css = () => document.getElementById(CUSTOM_CSS_STYLE_ID)?.textContent;
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

afterEach(() => {
  setCustomCssDisabled(false);
  applyCustomCss("");
  document.head.querySelectorAll("style").forEach((el) => el.remove());
  document.body.innerHTML = "";
  setToasts([]);
  vi.restoreAllMocks();
});

describe("custom.css live reload", () => {
  it("re-reads and applies an outside edit announced for this graph binding", async () => {
    applyCustomCss("a { color: red }");
    vi.spyOn(backend(), "readCustomCss").mockResolvedValue("a { color: blue }");
    applyCustomCssChange({ binding_generation: captureBinding().backendGeneration });
    await vi.waitFor(() => expect(css()).toBe("a { color: blue }"));
  });

  it("ignores an event from an older graph binding", async () => {
    applyCustomCss("a { color: red }");
    const read = vi.spyOn(backend(), "readCustomCss").mockResolvedValue("a { color: blue }");
    applyCustomCssChange({ binding_generation: captureBinding().backendGeneration + 1 });
    await tick();
    expect(read).not.toHaveBeenCalled();
    expect(css()).toBe("a { color: red }");
  });

  it("a file deleted outside Tine clears the stylesheet", async () => {
    applyCustomCss("a { color: red }");
    vi.spyOn(backend(), "readCustomCss").mockResolvedValue("");
    applyCustomCssChange({});
    await vi.waitFor(() => expect(css()).toBe(""));
  });
});

describe("Disable custom CSS (session-only safe mode)", () => {
  it("blanks the stylesheet, remembers an outside edit, and restores the latest text when re-enabled", () => {
    applyCustomCss("a { color: red }");
    setCustomCssDisabled(true);
    expect(css()).toBe("");
    applyCustomCss("a { color: green }");
    expect(css()).toBe("");
    setCustomCssDisabled(false);
    expect(css()).toBe("a { color: green }");
  });

  it("is not persisted anywhere", () => {
    setCustomCssDisabled(true);
    expect(JSON.stringify({ ...localStorage })).not.toMatch(/custom/i);
  });
});

async function mount(kind: "desktop" | "android") {
  platform.kind = kind;
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <CustomCssSettings />, root);
  await tick();
  await tick();
  return { root, dispose };
}

describe("Settings custom.css affordances", () => {
  it("desktop: Edit custom.css asks the backend, and a failure is shown as an error toast", async () => {
    const edit = vi.spyOn(backend(), "editCustomCss").mockRejectedValueOnce(new Error("no opener"));
    const { root, dispose } = await mount("desktop");
    const button = root.querySelector<HTMLButtonElement>('[data-setting-label="Edit custom.css"] button')!;
    expect(button).not.toBeNull();
    button.click();
    await vi.waitFor(() => expect(toasts().some((t) => t.kind === "error" && /no opener/.test(t.message))).toBe(true));
    expect(edit).toHaveBeenCalledTimes(1);
    expect(root.querySelector('[data-setting-label="Developer tools"]')).not.toBeNull();
    dispose();
  });

  it("android: no editor or inspector button, a file-path hint instead", async () => {
    const { root, dispose } = await mount("android");
    expect(root.querySelector('[data-setting-label="Edit custom.css"]')).toBeNull();
    expect(root.querySelector('[data-setting-label="Developer tools"]')).toBeNull();
    expect(root.textContent).toContain("logseq/custom.css");
    expect(root.querySelector('[data-setting-label="Disable custom CSS"]')).not.toBeNull();
    dispose();
  });

  it("the switch toggles safe mode on the live stylesheet", async () => {
    applyCustomCss("a { color: red }");
    const { root, dispose } = await mount("desktop");
    const toggle = root.querySelector<HTMLButtonElement>('[data-setting-label="Disable custom CSS"] button[role="switch"]')!;
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    toggle.click();
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    expect(css()).toBe("");
    toggle.click();
    expect(css()).toBe("a { color: red }");
    dispose();
  });
});
