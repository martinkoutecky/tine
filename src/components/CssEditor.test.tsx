import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { CssEditor } from "./CssEditor";
import { openCssEditor, closeCssEditor, setCssEditorText, flushCssEditor, cssEditorText, cssEditorProblem, checkCssEditorDisk, retireGraphCssEditor } from "../cssEditor";

afterEach(async () => {
  retireGraphCssEditor();
  vi.spyOn(backend(), "saveCssFile").mockResolvedValue("cleanup");
  await closeCssEditor();
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("in-app stylesheet editor", () => {
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
    vi.spyOn(backend(), "readCssFile").mockResolvedValue({ text: "before", revision: "base" });
    const save = vi.spyOn(backend(), "saveCssFile").mockResolvedValue("next");
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
