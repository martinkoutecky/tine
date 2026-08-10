import { describe, expect, it, beforeEach, vi } from "vitest";
import {
  clearAllEditorActivations,
  loadRoutedPage,
  editorActivationFor,
  ensurePageLoaded,
  resetStore,
  retireEditorFor,
  setEditorActivation,
} from "./store";
import { backend } from "./backend";
import type { PageDto } from "./types";

/**
 * The wiring, not the primitive.
 *
 * Every piece of increment 3 can be individually correct and the feature still
 * inert: if nothing ever mints an activation on the paths real editors take, every
 * save carries `activation: undefined`, the override path refuses it by design,
 * and "Keep mine" silently stops working for everyone. These assert the identity
 * actually reaches a save, and is actually given up when the editor is.
 */
const page = (name: string, path: string): PageDto => ({
  name,
  kind: "page",
  title: name,
  pre_block: null,
  blocks: [{ raw: "body", children: [] } as never],
  format: "md",
  read_only: false,
  path,
  rev: "rev-1",
});

describe("editor activation wiring (GH #254 increment 3)", () => {
  beforeEach(() => {
    resetStore();
    clearAllEditorActivations();
    vi.restoreAllMocks();
  });

  it("gives a saving editor an identity, and stamps it on the DTO", async () => {
    loadRoutedPage(page("Note", "pages/Note.md"));
    expect(editorActivationFor("Note")).toBeUndefined();

    const activate = vi
      .spyOn(backend(), "activateEditor")
      .mockResolvedValue({ activation: 77, target: "pages/Note.md", prospective: false });

    const { markDirty, flushPage } = await import("./persistence");
    markDirty("Note");
    const saved = vi.spyOn(backend(), "savePage").mockResolvedValue("rev-2");
    await flushPage("Note");

    expect(activate).toHaveBeenCalledWith("pages/Note.md", "reuse");
    expect(editorActivationFor("Note")).toBe(77);
    // The half that matters: it reached the wire. A registry entry nobody stamps
    // onto the DTO leaves the override path just as refused as no registry at all.
    expect(saved.mock.calls[0]?.[0]?.activation).toBe(77);
  });

  it("does not re-mint for an editor that already holds one", async () => {
    loadRoutedPage(page("Note", "pages/Note.md"));
    setEditorActivation("Note", 12);
    const activate = vi.spyOn(backend(), "activateEditor");

    const { markDirty, flushPage } = await import("./persistence");
    markDirty("Note");
    vi.spyOn(backend(), "savePage").mockResolvedValue("rev-2");
    await flushPage("Note");

    // Churning the token would strand the identity a live banner is bound to.
    expect(activate).not.toHaveBeenCalled();
    expect(editorActivationFor("Note")).toBe(12);
  });

  it("gives up the identity when the editor is retired, naming the exact one", () => {
    ensurePageLoaded(page("Note", "pages/Note.md"));
    setEditorActivation("Note", 5);
    const retire = vi.spyOn(backend(), "retireEditorActivation").mockResolvedValue(true);

    retireEditorFor("Note");

    expect(editorActivationFor("Note")).toBeUndefined();
    // Compare-and-retire: naming the exact activation is what stops a late
    // retirement from revoking a newer editor of the same path.
    expect(retire).toHaveBeenCalledWith("pages/Note.md", 5);
  });

  it("does not record an identity across a graph switch", async () => {
    loadRoutedPage(page("Note", "pages/old.md"));
    let release: (h: unknown) => void = () => {};
    vi.spyOn(backend(), "activateEditor").mockReturnValue(
      new Promise((r) => {
        release = r as (h: unknown) => void;
      }) as never,
    );
    const saved = vi.spyOn(backend(), "savePage").mockResolvedValue("rev-2");

    const { markDirty, flushPage } = await import("./persistence");
    markDirty("Note");
    const flush = flushPage("Note");

    // The graph goes away while the activation IPC is still in flight, and a NEW
    // graph's page arrives under the same name.
    resetStore();
    loadRoutedPage(page("Note", "pages/new.md"));
    release({ activation: 71, target: "pages/old.md", prospective: false });
    await flush;

    // The old graph's identity must not attach to the new graph's editor, and the
    // abandoned save must not write the replacement page.
    expect(editorActivationFor("Note")).not.toBe(71);
    for (const [dto] of saved.mock.calls) {
      expect(dto.activation).not.toBe(71);
    }
  });

  it("carries an absent editor's prospective target on its DTO", async () => {
    // No file yet: the core cannot recognise its own absent editor when the
    // target drifts unless the DTO is pinned to what it was promised.
    loadRoutedPage({ ...page("New", ""), path: "", rev: null });
    vi.spyOn(backend(), "activateAbsentEditor").mockResolvedValue({
      activation: 72,
      target: "pages/New.md",
      prospective: true,
    });
    const saved = vi.spyOn(backend(), "savePage").mockResolvedValue("rev-2");

    const { markDirty, flushPage } = await import("./persistence");
    markDirty("New");
    await flushPage("New");

    expect(saved.mock.calls[0]?.[0]?.activation).toBe(72);
    expect(saved.mock.calls[0]?.[0]?.path).toBe("pages/New.md");
  });

  it("retiring an editor that holds none is a no-op", () => {
    ensurePageLoaded(page("Note", "pages/Note.md"));
    const retire = vi.spyOn(backend(), "retireEditorActivation");
    retireEditorFor("Note");
    expect(retire).not.toHaveBeenCalled();
  });
});
