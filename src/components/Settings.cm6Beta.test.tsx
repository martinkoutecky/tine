import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { preloadCm6Editor } from "./EditorGate";
beforeAll(preloadCm6Editor);
import { render } from "solid-js/web";
// Exercise the Beta identity even when the source identity changes later.
vi.mock("../appIdentity", () => ({ APP_UPDATE_CHANNEL: "beta", APP_PRODUCT_NAME: "Tine" }));
// This test owns the switch itself, including during the switch-on census.
vi.unmock("../editor/cm6Setting");
import { backend } from "../backend";
import { Settings } from "./Settings";
import { Block } from "./Block";
import { closeSettings, openSettings } from "../ui";
import { cm6EditorEnabled, initCm6EditorSetting, setCm6EditorEnabled } from "../editor/cm6Setting";
import { loadSingle } from "../document/workingSet";
import { node, resetStore } from "../document";
import { endEdit, startEditing } from "../editorController";
import { editorIn } from "../tests/editorDriver";

afterEach(() => {
  endEdit("page-navigation"); resetStore(); closeSettings();
  setCm6EditorEnabled(false); document.body.innerHTML = ""; vi.restoreAllMocks();
});

it("Beta search explains the classic opt-out and switching back mounts a document-backed CM6 editor", async () => {
  const save = vi.spyOn(backend(), "setAppBool").mockResolvedValue();
  vi.spyOn(backend(), "getAppBool").mockImplementation(async (_key, fallback) => fallback);
  await initCm6EditorSetting();
  const settings = document.createElement("div"); document.body.append(settings);
  const disposeSettings = render(() => <Settings />, settings);
  let disposeBlock: (() => void) | undefined;
  try {
    expect(cm6EditorEnabled()).toBe(true);
    openSettings("editor");
    await new Promise(resolve => setTimeout(resolve, 0));
    const search = settings.querySelector<HTMLInputElement>(".settings-search-input")!;
    search.value = "CodeMirror";
    search.dispatchEvent(new InputEvent("input", { bubbles: true }));
    settings.querySelector<HTMLButtonElement>(".settings-search-result")!.click();
    await new Promise(resolve => setTimeout(resolve, 0));
    const field = settings.querySelector('[data-setting-id="settings-cm6-label"]')!;
    expect(field.textContent).toContain("Use classic editor");
    expect(field.textContent).toContain("CodeMirror is the default");
    expect(field.textContent).toContain("classic editor is planned to be retired");
    expect(field.textContent).toContain("please report it (About → Report an issue)");
    expect(field.querySelector("a")).toBeNull();
    const toggle = field.querySelector<HTMLButtonElement>('[role="switch"]')!;
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    toggle.click();
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(save).toHaveBeenCalledWith("experimental_cm6_editor", false);
    expect(cm6EditorEnabled()).toBe(false);
    expect(toggle.getAttribute("aria-checked")).toBe("true");
    toggle.click();
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(save).toHaveBeenCalledWith("experimental_cm6_editor", true);
    closeSettings();
    loadSingle({ name: "Beta CM6", title: "Beta CM6", kind: "page", pre_block: null,
      blocks: [{ id: "beta-cm6", raw: "original", collapsed: false, children: [] }] });
    const root = document.createElement("div"); document.body.append(root);
    disposeBlock = render(() => <Block id="beta-cm6" />, root);
    startEditing("beta-cm6", 8);
    const editor = editorIn(root);
    expect(editor.kind).toBe("cm6");
    editor.type(" edited");
    expect(node("beta-cm6").raw).toBe("original edited");
  } finally { disposeBlock?.(); disposeSettings(); }
});
