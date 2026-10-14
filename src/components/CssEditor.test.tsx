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
