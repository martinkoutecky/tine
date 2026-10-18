import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { CssEditor } from "./CssEditor";
import { openCssEditor, closeCssEditor, setCssEditorText, flushCssEditor, cssEditorText, cssEditorProblem, checkCssEditorDisk, retireGraphCssEditor } from "../cssEditor";
import { installBackgroundFlush } from "../backgroundFlush";

afterEach(async () => {
  retireGraphCssEditor();
  vi.spyOn(backend(), "saveCssFile").mockResolvedValue("cleanup");
  await closeCssEditor();
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("in-app stylesheet editor", () => {
  it.each([false, true])("background flush preserves a recoverable %s global draft with its original revision", async (global) => {
    const storage = new Map<string, string>();
    vi.spyOn(backend(), "getAppString").mockImplementation(async key => storage.get(key) ?? "");
    vi.spyOn(backend(), "setAppString").mockImplementation(async (key, value) => { storage.set(key, value); });
    const read = vi.spyOn(backend(), "readCssFile").mockResolvedValue({ text: "before", revision: "base" });
    const save = vi.spyOn(backend(), "saveCssFile").mockRejectedValue(new Error("conflict"));
    await openCssEditor(global);
    setCssEditorText("recover me");
    const stop = installBackgroundFlush({ endEdit: () => {}, flushAll: flushCssEditor, closeInFlight: () => false, isHidden: () => true });
    document.dispatchEvent(new Event("freeze"));
    await vi.waitFor(() => expect(save).toHaveBeenCalledWith(global, "recover me", "base"));
    await vi.waitFor(() => expect(cssEditorProblem()).toContain("could not be saved"));
    stop();
    const capsule = [...storage.entries()].find(([, value]) => value.includes("recover me"));
    expect(capsule).toBeDefined();
    // Retire the rendering session to model WebView loss; recover from app settings.
    if (!global) retireGraphCssEditor();
    else { save.mockResolvedValueOnce("cleanup"); await closeCssEditor(); }
    // Keep the captured recovery capsule for the simulated restart.
    storage.set(capsule![0], capsule![1]);
    read.mockResolvedValue({ text: "external", revision: "changed" });
    await openCssEditor(global);
    expect(cssEditorText()).toBe("recover me");
    await flushCssEditor();
    expect(save).toHaveBeenLastCalledWith(global, "recover me", "base");
  });
  it("a failed attempt to open the other file keeps the current editor usable", async () => {
    const read = vi.spyOn(backend(), "readCssFile").mockResolvedValue({ text: "before", revision: "base" });
    await openCssEditor(false);
    read.mockRejectedValueOnce(new Error("permission denied"));
    await openCssEditor(true);
    setCssEditorText("mine");
    const save = vi.spyOn(backend(), "saveCssFile").mockResolvedValue("next");
    expect(await flushCssEditor()).toBe(true);
    expect(save).toHaveBeenCalledWith(false, "mine", "base");
  });
  it.each([false, true])("reads, edits and guarded-saves the %s global layer", async (global) => {
    vi.spyOn(backend(), "getAppString").mockResolvedValue("");
    const read = vi.spyOn(backend(), "readCssFile").mockResolvedValue({ text: "before", revision: "base" });
    const save = vi.spyOn(backend(), "saveCssFile").mockImplementation(async () => {
      read.mockResolvedValue({ text: "after", revision: "next" });
      return "next";
    });
    await openCssEditor(global);
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <CssEditor />, root);
    const editor = root.querySelector("textarea")!;
    editor.value = "after";
    editor.dispatchEvent(new Event("input", { bubbles: true }));
    [...root.querySelectorAll("button")].find((button) => button.textContent === "Save")!.click();
    await vi.waitFor(() => expect(save).toHaveBeenCalledWith(global, "after", "base"));
    await vi.waitFor(() => expect(cssEditorProblem()).toBe(""));
    await flushCssEditor();
    expect(cssEditorText()).toBe("after");
    dispose();
  });

  it("retains the draft and original revision when an external change conflicts", async () => {
    const read = vi.spyOn(backend(), "readCssFile").mockResolvedValue({ text: "before", revision: "base" });
    await openCssEditor(false);
    setCssEditorText("mine");
    read.mockResolvedValue({ text: "external", revision: "changed" });
    await checkCssEditorDisk(false);
    expect(cssEditorProblem()).toContain("changed outside");
    const save = vi.spyOn(backend(), "saveCssFile").mockRejectedValue(new Error("conflict"));
    expect(await flushCssEditor()).toBe(false);
    expect(save).toHaveBeenCalledWith(false, "mine", "base");
    expect(cssEditorText()).toBe("mine");
    expect(cssEditorProblem()).toContain("could not be saved");
  });
});

describe("CSS editor action placement (D-20261018-UI2b)", () => {
  async function mount(text = "before") {
    const storage = new Map<string, string>();
    vi.spyOn(backend(), "getAppString").mockImplementation(async key => storage.get(key) ?? "");
    vi.spyOn(backend(), "setAppString").mockImplementation(async (key, value) => { storage.set(key, value); });
    vi.spyOn(backend(), "readCssFile").mockResolvedValue({ text, revision: "base" });
    const save = vi.spyOn(backend(), "saveCssFile").mockResolvedValue("next");
    await openCssEditor(false);
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <CssEditor />, root);
    const type = (value: string) => {
      const editor = root.querySelector("textarea")!;
      editor.value = value;
      editor.dispatchEvent(new Event("input", { bubbles: true }));
    };
    const labels = () => [...root.querySelectorAll(".dialog-actions button")].map((button) => button.textContent?.trim());
    const button = (label: string) => [...root.querySelectorAll<HTMLButtonElement>(".dialog-actions button")].find((b) => b.textContent?.trim() === label)!;
    return { root, dispose, type, labels, button, save, storage };
  }

  it("reads [Reload] ... [Cancel][Save & close], with the primary last", async () => {
    const { labels, button, dispose } = await mount();
    const all = labels();
    expect(all.at(-2)).toBe("Cancel");
    expect(all.at(-1)).toBe("Save & close");
    expect(all[0]).toBe("Reload");
    expect(button("Save & close").classList.contains("settings-btn-primary")).toBe(true);
    dispose();
  });

  it("an outside click closes a clean editor and does nothing when edits are unsaved", async () => {
    const { root, dispose, type, save } = await mount();
    type("changed");
    root.querySelector<HTMLElement>(".modal-overlay")!.click();
    await Promise.resolve();
    expect(save).not.toHaveBeenCalled();
    expect(root.querySelector(".modal-overlay")).not.toBeNull();
    expect(cssEditorText()).toBe("changed");
    type("before");
    root.querySelector<HTMLElement>(".modal-overlay")!.click();
    await vi.waitFor(() => expect(root.querySelector(".modal-overlay")).toBeNull());
    expect(save).not.toHaveBeenCalled();
    dispose();
  });

  it("Cancel on a clean editor closes without asking; with edits it asks, and declining keeps them", async () => {
    const { root, dispose, type, button, save } = await mount();
    const ask = vi.spyOn(backend(), "confirm").mockResolvedValue(false);
    type("changed");
    button("Cancel").click();
    await vi.waitFor(() => expect(ask).toHaveBeenCalledOnce());
    expect(ask.mock.calls[0][1]).toMatchObject({ okLabel: "Discard changes", cancelLabel: "Keep editing" });
    await Promise.resolve();
    expect(root.querySelector(".modal-overlay")).not.toBeNull();
    expect(cssEditorText()).toBe("changed");
    type("before");
    button("Cancel").click();
    await vi.waitFor(() => expect(root.querySelector(".modal-overlay")).toBeNull());
    expect(ask).toHaveBeenCalledOnce();
    expect(save).not.toHaveBeenCalled();
    dispose();
  });

  it("confirmed Cancel discards the edits without saving and drops the recovery draft", async () => {
    const { root, dispose, type, button, save, storage } = await mount();
    vi.spyOn(backend(), "confirm").mockResolvedValue(true);
    type("changed");
    button("Cancel").click();
    await vi.waitFor(() => expect(root.querySelector(".modal-overlay")).toBeNull());
    expect(save).not.toHaveBeenCalled();
    expect([...storage.values()].some((value) => value.includes("changed"))).toBe(false);
    dispose();
  });

  it("Reload asks with explicit labels before discarding edits", async () => {
    const { dispose, type, button } = await mount();
    const ask = vi.spyOn(backend(), "confirm").mockResolvedValue(false);
    type("changed");
    button("Reload").click();
    await vi.waitFor(() => expect(ask).toHaveBeenCalledOnce());
    expect(ask.mock.calls[0][1]).toMatchObject({ okLabel: "Discard and reload", cancelLabel: "Keep editing" });
    dispose();
  });
});
