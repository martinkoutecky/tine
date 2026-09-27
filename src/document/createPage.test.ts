import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { captureBinding } from "../binding";
import { errorFamily } from "../errorFamily";
import type { PageDto } from "../types";
import { clearConflict, createPage, CreatePageRefusal, flushPage, markConflict, markDirty } from "./save/engine";
import { loadSingle, reloadPage, resetStore } from "./workingSet";

const dto = (name = "New"): PageDto => ({ name, kind: "page", title: name, pre_block: null, blocks: [] });

afterEach(() => {
  resetStore();
  clearConflict("New");
  vi.restoreAllMocks();
});

async function expectLocalRefusal(create: Promise<string>, reason: CreatePageRefusal["reason"]) {
  const error = await create.catch((e: unknown) => e);
  expect(error).toBeInstanceOf(CreatePageRefusal);
  if (!(error instanceof CreatePageRefusal)) throw error;
  expect(error.reason).toBe(reason);
  expect(errorFamily(error)).toBe("unknown");
}

describe("createPage refusal families", () => {
  it("keeps name, conflict, dirty and stale-binding refusals distinct from disk conflict", async () => {
    await expectLocalRefusal(createPage("Different", dto()), "name-mismatch");
    markConflict("New");
    await expectLocalRefusal(createPage("New", dto()), "page-conflicted");
    resetStore();
    clearConflict("New");
    loadSingle(dto());
    markDirty("New");
    await expectLocalRefusal(createPage("New", dto()), "page-dirty");
    resetStore();
    await expectLocalRefusal(createPage("New", dto(), { bindingGeneration: captureBinding().backendGeneration + 1 }), "stale-binding");
  });

  it("keeps alias and page-rebound refusals distinct from disk conflict", async () => {
    vi.spyOn(backend(), "resolvePage").mockResolvedValueOnce({ kind: "alias", owners: ["pages/Owner.md"] });
    await expectLocalRefusal(createPage("New", dto()), "alias");
    let resolve!: (value: { kind: "absent"; id: string }) => void;
    vi.spyOn(backend(), "resolvePage").mockImplementationOnce(() => new Promise((r) => { resolve = r; }));
    loadSingle(dto());
    const pending = createPage("New", dto());
    reloadPage({ ...dto(), blocks: [{ id: "replacement", raw: "changed", collapsed: false, children: [] }] });
    resolve({ kind: "absent", id: "pages/New.md" });
    await expectLocalRefusal(pending, "page-rebound");
  });

  it("keeps a queued save refusal distinct from disk conflict", async () => {
    loadSingle(dto());
    markDirty("New");
    let finish!: (rev: string) => void;
    vi.spyOn(backend(), "savePage").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const saving = flushPage("New");
    await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
    await expectLocalRefusal(createPage("New", dto()), "page-saving");
    finish("rev-1");
    await saving;
  });

  it("keeps a graph switch during create distinct from disk conflict", async () => {
    let finish!: (rev: string) => void;
    vi.spyOn(backend(), "savePage").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const pending = createPage("New", dto(), { id: "pages/New.md" });
    await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
    resetStore();
    finish("rev-1");
    await expectLocalRefusal(pending, "graph-changed");
  });

  it("preserves the backend disk-conflict token and marks the current page conflicted", async () => {
    loadSingle(dto());
    vi.spyOn(backend(), "savePage").mockRejectedValueOnce(new Error("conflict"));
    const error = await createPage("New", dto(), { id: "pages/New.md" }).catch((e: unknown) => e);
    expect(error).not.toBeInstanceOf(CreatePageRefusal);
    expect(errorFamily(error)).toBe("conflict");
  });
});
