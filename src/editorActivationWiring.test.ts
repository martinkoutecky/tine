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

  it("abandons a save whose graph went away while it was acquiring", async () => {
    loadRoutedPage(page("Note", "pages/old.md"));

    // WAIT until acquisition is genuinely in flight before switching graphs. An
    // earlier version of this test reset first, so the queued save never reached
    // acquisition and it passed VACUOUSLY — it also only excluded the token,
    // which cannot catch the real defect: the abandoned save continued and wrote
    // the replacement graph's bytes with `activation: undefined`.
    let inFlight!: () => void;
    const acquiring = new Promise<void>((r) => {
      inFlight = r;
    });
    let release: (h: unknown) => void = () => {};
    vi.spyOn(backend(), "activateEditor").mockImplementation(
      () =>
        new Promise((r) => {
          release = r as (h: unknown) => void;
          inFlight();
        }) as never,
    );
    const saved = vi.spyOn(backend(), "savePage").mockResolvedValue("rev-2");

    const { markDirty, flushPage } = await import("./persistence");
    markDirty("Note");
    const flush = flushPage("Note");
    await acquiring;

    // Only now does the graph go away and a NEW graph's page arrive.
    resetStore();
    loadRoutedPage(page("Note", "pages/new.md"));
    release({ activation: 71, target: "pages/old.md", prospective: false });
    await flush;

    // No write at all: the abandoned save must not serialize the replacement
    // graph's page, with or without an identity.
    expect(saved).not.toHaveBeenCalled();
    expect(editorActivationFor("Note")).not.toBe(71);
  });

  it("does not hand a replaced instance the outgoing editor's identity", async () => {
    loadRoutedPage(page("Note", "pages/Note.md"));

    let inFlight!: () => void;
    const acquiring = new Promise<void>((r) => {
      inFlight = r;
    });
    let release: (h: unknown) => void = () => {};
    vi.spyOn(backend(), "activateEditor").mockImplementation(
      () =>
        new Promise((r) => {
          release = r as (h: unknown) => void;
          inFlight();
        }) as never,
    );
    vi.spyOn(backend(), "savePage").mockResolvedValue("rev-2");
    vi.spyOn(backend(), "retireEditorActivation").mockResolvedValue(true);

    const { markDirty, flushPage } = await import("./persistence");
    markDirty("Note");
    const flush = flushPage("Note");
    await acquiring;

    // A SAME-PATH content replacement — the watcher-approved reload shape. The
    // path is unchanged, so a path-only check cannot see that this is a different
    // editor; only the instance generation can.
    // DIFFERENT content, or `upsertPage` treats it as a self-write echo and
    // deliberately keeps the working copy — which is genuinely the same editor,
    // so attaching the identity there would be correct.
    const { reloadPage } = await import("./store");
    reloadPage({
      ...page("Note", "pages/Note.md"),
      blocks: [{ raw: "external body", children: [] } as never],
      rev: "rev-external",
    });
    release({ activation: 73, target: "pages/Note.md", prospective: false });
    await flush;

    expect(editorActivationFor("Note")).not.toBe(73);
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

  it("does not count bookkeeping as post-click input", async () => {
    // Clearing a false-positive banner re-arms the existing draft. That changes
    // nothing the user wrote, so it must NOT read as input made after clicking
    // "Use disk version" — counting it cancels a discard the user never
    // interrupted, and the local draft gets written where the disk winner was
    // requested. (Reproduced by round-3 verification.)
    const { editGeneration } = await import("./store");
    const { markDirty } = await import("./persistence");
    loadRoutedPage(page("Note", "pages/Note.md"));

    const before = editGeneration("Note");
    markDirty("Note", { content: false });
    expect(editGeneration("Note")).toBe(before);

    // Real input still moves it, or the check protects nothing.
    markDirty("Note");
    expect(editGeneration("Note")).not.toBe(before);
  });

  it("retiring an editor that holds none is a no-op", () => {
    ensurePageLoaded(page("Note", "pages/Note.md"));
    const retire = vi.spyOn(backend(), "retireEditorActivation");
    retireEditorFor("Note");
    expect(retire).not.toHaveBeenCalled();
  });
});
