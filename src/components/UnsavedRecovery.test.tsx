import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { setDoc, resetStore } from "../store";
import { addDirty, isDirty } from "../persistence";
import { openUnsavedRecovery, closeUnsavedRecovery, unsavedRecoveryOpen } from "../unsavedRecovery";
import { RecoveryDraft, UnsavedRecovery } from "./UnsavedRecovery";
import { notifyGraphRebound } from "../modeHooks";
import type { PageDto } from "../types";

const draft: PageDto = { name: "Notes", title: "Notes", kind: "page", format: "md", path: "pages/Notes.md",
  pre_block: "tags:: private", blocks: [{ id: "a", raw: "**Parent**  \nsecond line\n", collapsed: false,
    children: [{ id: "b", raw: "Child #tag", collapsed: true, children: [] }] }] };
const tick = async () => { for (let i=0; i<15; i++) await Promise.resolve(); };
afterEach(() => { closeUnsavedRecovery(); resetStore(); vi.restoreAllMocks(); vi.unstubAllGlobals(); document.body.innerHTML = ""; });

describe("unsaved recovery", () => {
  it("copies every retained source field exactly without writing or retiring the draft", async () => {
    const copy = vi.spyOn(backend(), "writeText").mockResolvedValue();
    const save = vi.spyOn(backend(), "savePage");
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <RecoveryDraft page={draft} />, root);
    try {
      const button = [...root.querySelectorAll("button")].find((b) => b.textContent === "Copy complete recovery data")!;
      button.click(); await tick();
      expect(JSON.parse(copy.mock.calls[0][0])).toEqual(draft);
      expect(save).not.toHaveBeenCalled();
      expect(root.textContent).toContain("Copied. Paste into a separate file");
    } finally { dispose(); }
  });
  it("keeps the preview selectable and reports a clipboard failure", async () => {
    vi.spyOn(backend(), "writeText").mockRejectedValue(new Error("clipboard denied"));
    vi.stubGlobal("navigator", { clipboard: { writeText: vi.fn().mockRejectedValue(new Error("denied")) } });
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <RecoveryDraft page={draft} />, root);
    try {
      root.querySelector("button")!.click(); await tick();
      expect(root.textContent).toContain("Could not copy");
      expect(root.querySelector("pre")!.textContent).toContain("Child #tag");
      expect(root.querySelector("pre")!.textContent).toContain("tags:: private");
    } finally { dispose(); }
  });
  it("identifies a failed page, retries, and preserves it if saving fails again", async () => {
    setDoc({ loaded: true, feed: [], pages: [{ name: "Notes", title: "Notes", kind: "page", format: "md",
      preBlock: null, roots: ["a"], readOnly: false, guide: false }],
      byId: { a: { id: "a", raw: "Unsaved writing", collapsed: false, parent: null, children: [], page: "Notes" } } });
    addDirty("Notes");
    vi.spyOn(backend(), "savePage").mockRejectedValue(new Error("disk full"));
    openUnsavedRecovery();
    const root = document.createElement("div"); document.body.append(root);
    const dispose = render(() => <UnsavedRecovery />, root);
    try {
      expect(root.textContent).toContain("Notes — Not saved");
      [...root.querySelectorAll("button")].find((b) => b.textContent === "Retry saving")!.click();
      await tick();
      expect(isDirty("Notes")).toBe(true);
      expect(root.textContent).toContain("Unsaved writing");
      expect(unsavedRecoveryOpen()).toBe(true);
      notifyGraphRebound();
      expect(root.textContent).toBe("");
    } finally { dispose(); }
  });
});
