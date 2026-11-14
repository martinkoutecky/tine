import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { commandDefaults, installKeybindings, paletteCommands } from "../keybindings";
import { initParser } from "../render/parse";
import { STYLE_KNOBS_KEY, initStyleKnobs, styleKnobValues } from "../styleKnobs";
import { useEditorKind } from "../tests/editorKind";
import { resetShortcutOverride, setShortcutOverride, shortcutOverrides } from "../ui";
import { EmbedMacro } from "./EmbedMacro";
import { StyleKnobsSettings } from "./StyleKnobsSettings";

const COMMAND = "ui/toggle-embed-breadcrumbs";
let dispose: (() => void) | undefined;
let uninstall: (() => void) | undefined;
let stored = "";

beforeAll(initParser);
beforeEach(async () => {
  stored = "";
  vi.spyOn(backend(), "getAppString").mockImplementation(async (key, fallback) => key === STYLE_KNOBS_KEY ? stored : fallback);
  vi.spyOn(backend(), "setAppString").mockImplementation(async (key, value) => {
    if (key === STYLE_KNOBS_KEY) stored = value ?? "";
  });
  await initStyleKnobs();
});
afterEach(async () => {
  dispose?.();
  dispose = undefined;
  uninstall?.();
  uninstall = undefined;
  resetShortcutOverride(COMMAND);
  installKeybindings()();
  stored = "";
  await initStyleKnobs();
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe.each(["textarea", "cm6"] as const)("embed breadcrumb command (%s, GH #860)", (kind) => {
  useEditorKind(kind);

  it("is offered in the palette and Keyboard shortcuts with no default key", () => {
    expect(commandDefaults().find(c => c.id === COMMAND)).toMatchObject({
      label: "Toggle embed breadcrumbs", binding: "", scope: "global",
    });
    expect(paletteCommands().find(c => c.id === COMMAND)).toMatchObject({
      label: "Toggle embed breadcrumbs", binding: "",
    });
  });

  it("toggles the rendered trail and Settings together, preserving the device choice on reload", async () => {
    vi.spyOn(backend(), "resolveBlocks").mockResolvedValue([{
      page: "Source", kind: "page", blocks: [{
        id: "breadcrumb-command-block", raw: "Embedded content", collapsed: false,
        children: [], breadcrumb: ["Parent"],
      }],
    }] as never);
    const root = document.createElement("div");
    document.body.append(root);
    dispose = render(() => <><StyleKnobsSettings /><EmbedMacro body="embed ((breadcrumb-command-block))" /></>, root);
    await vi.waitFor(() => expect(root.textContent).toContain("Embedded content"));
    const toggle = () => root.querySelector<HTMLButtonElement>('[data-setting-label="Block embed breadcrumbs"] [role="switch"]')!;
    const trail = () => root.querySelector(".embed-source-breadcrumb");
    const command = paletteCommands().find(c => c.id === COMMAND)!;
    expect(trail()).toBeNull();
    expect(toggle().getAttribute("aria-checked")).toBe("false");

    command.run();
    expect(styleKnobValues()["embed-breadcrumbs"]).toBe(true);
    expect(toggle().getAttribute("aria-checked")).toBe("true");
    expect(trail()?.textContent).toContain("Source");
    expect(trail()?.textContent).toContain("Parent");
    await vi.waitFor(() => expect(JSON.parse(stored)["embed-breadcrumbs"]).toBe(true));
    await initStyleKnobs();
    expect(trail()).not.toBeNull();

    command.run();
    expect(toggle().getAttribute("aria-checked")).toBe("false");
    expect(trail()).toBeNull();
    await vi.waitFor(() => expect(JSON.parse(stored)["embed-breadcrumbs"]).toBeUndefined());
    await initStyleKnobs();
    expect(trail()).toBeNull();

    toggle().click();
    expect(trail()).not.toBeNull();
    command.run();
    expect(toggle().getAttribute("aria-checked")).toBe("false");
    expect(trail()).toBeNull();
    await vi.waitFor(() => expect(JSON.parse(stored)["embed-breadcrumbs"]).toBeUndefined());
  });

  it("runs from a shortcut the user assigns", () => {
    setShortcutOverride(COMMAND, "mod+shift+b");
    uninstall = installKeybindings(shortcutOverrides());
    const press = () => document.body.dispatchEvent(new KeyboardEvent("keydown", {
      key: "b", ctrlKey: true, shiftKey: true, bubbles: true, cancelable: true,
    }));
    press();
    expect(styleKnobValues()["embed-breadcrumbs"]).toBe(true);
    press();
    expect(styleKnobValues()["embed-breadcrumbs"]).not.toBe(true);
  });
});
