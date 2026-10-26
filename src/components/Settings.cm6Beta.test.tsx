import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
// Exercise the shipped Beta gate even when the source identity changes later.
vi.mock("../appIdentity", () => ({ APP_UPDATE_CHANNEL: "beta", APP_PRODUCT_NAME: "Tine" }));
// This test owns the switch itself, including during the switch-on census.
vi.unmock("../editor/cm6Setting");
import { backend } from "../backend";
import { Settings } from "./Settings";
import { Block } from "./Block";
import { closeSettings, openSettings } from "../ui";
import { cm6EditorEnabled, setCm6EditorEnabled } from "../editor/cm6Setting";
import { loadSingle } from "../document/workingSet";
import { node, resetStore } from "../document";
import { endEdit, startEditing } from "../editorController";
import { editorIn } from "../tests/editorDriver";

afterEach(() => {
  endEdit("page-navigation"); resetStore(); closeSettings();
  setCm6EditorEnabled(false); document.body.innerHTML = ""; vi.restoreAllMocks();
});

it("Beta search exposes the default-off setting and the opt-in mounts a document-backed CM6 editor", async () => {
  const save = vi.spyOn(backend(), "setAppBool").mockResolvedValue();
  const settings = document.createElement("div"); document.body.append(settings);
  const disposeSettings = render(() => <Settings />, settings);
  let disposeBlock: (() => void) | undefined;
  try {
    expect(cm6EditorEnabled()).toBe(false);
    openSettings("editor");
    await new Promise(resolve => setTimeout(resolve, 0));
    const search = settings.querySelector<HTMLInputElement>(".settings-search-input")!;
    search.value = "CodeMirror";
    search.dispatchEvent(new InputEvent("input", { bubbles: true }));
    settings.querySelector<HTMLButtonElement>(".settings-search-result")!.click();
    await new Promise(resolve => setTimeout(resolve, 0));
    const field = [...settings.querySelectorAll(".settings-field")].find(el => el.textContent?.includes("Experimental: CodeMirror editor"))!;
    const toggle = field.querySelector<HTMLButtonElement>('[role="switch"]')!;
    expect(toggle.getAttribute("aria-checked")).toBe("false");
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
