import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { captureBinding } from "../binding";
import { errorFamily } from "../errorFamily";
import type { PageDto } from "../types";
import { createPage, CreatePageRefusal, flushPage, isConflicted, isSaving, markDirty } from "./host/wiring";
import { loadSingle, resetStore } from "./workingSet";
import { bindTestHost, type TestHost } from "./host/wiring.test.support";

const dto = (name = "New"): PageDto => ({ name, kind: "page", title: name, pre_block: null, blocks: [] });

let host: TestHost;
beforeEach(async () => {
  resetStore();
  host = await bindTestHost();
});
afterEach(() => {
  resetStore();
  vi.restoreAllMocks();
});

async function expectLocalRefusal(create: Promise<unknown>, reason: CreatePageRefusal["reason"]) {
  const error = await create.catch((e: unknown) => e);
  expect(error).toBeInstanceOf(CreatePageRefusal);
  if (!(error instanceof CreatePageRefusal)) throw error;
  expect(error.reason).toBe(reason);
  expect(errorFamily(error)).toBe("unknown");
}

/** Submits the host admitted but has not answered yet: the test answers them. */
function holdAnswers(): Array<{ id: number; key: string }> {
  const admitted: Array<{ id: number; key: string }> = [];
  vi.spyOn(backend(), "pageSubmit").mockImplementation(async (_session, id, key) => { admitted.push({ id, key }); return null; });
  return admitted;
}

/** The host's answer to an admitted submit: took it, at `version`, maybe conflicted. */
function answer({ id, key }: { id: number; key: string }, version: number, conflict = false) {
  host.deliver({ key, answer: { id, version, took: true, outcome: { kind: "applied" } }, notice: { conflictReported: conflict },
    page: { version, conflict, risk: false, disk: conflict ? { kind: "file", rev: "external" } : null, text: { kind: "unchanged" } } });
}

describe("createPage refusal families", () => {
  it("rejects a host-refused create with the host's reason, never as a disk conflict", async () => {
    vi.spyOn(backend(), "pageSubmit").mockResolvedValueOnce({ reason: "twin", existing: "pages/new.md" });
    const error = await createPage("New", dto()).catch((e: unknown) => e);
    expect(error).not.toBeInstanceOf(CreatePageRefusal);
    expect(errorFamily(error)).not.toBe("conflict");
    expect((error as Error).message).toContain("pages/new.md is already this page");
  });

  it("keeps name, conflict, dirty and stale-binding refusals distinct from disk conflict", async () => {
    await expectLocalRefusal(createPage("Different", dto()), "name-mismatch");
    const admitted = holdAnswers();
    loadSingle(dto());
    markDirty("New", "save-block");
    void flushPage("New");
    await vi.waitFor(() => expect(admitted).toHaveLength(1));
    answer(admitted[0], 5, true);
    expect(isConflicted("New")).toBe(true);
    vi.restoreAllMocks();
    await expectLocalRefusal(createPage("New", dto()), "page-conflicted");
    resetStore();
    host = await bindTestHost();
    loadSingle(dto());
    markDirty("New", "save-block");
    await expectLocalRefusal(createPage("New", dto()), "page-dirty");
    resetStore();
    host = await bindTestHost();
    await expectLocalRefusal(createPage("New", dto(), { bindingGeneration: captureBinding().backendGeneration + 1 }), "stale-binding");
  });

  it("keeps an alias refusal distinct from disk conflict", async () => {
    vi.spyOn(backend(), "pageOpen").mockResolvedValueOnce({ reason: "alias", owners: ["pages/Owner.md"] });
    await expectLocalRefusal(createPage("New", dto()), "alias");
  });

  it("keeps a sent, unanswered save refusal distinct from disk conflict", async () => {
    loadSingle(dto());
    const admitted = holdAnswers();
    markDirty("New", "save-block");
    const saving = flushPage("New");
    await vi.waitFor(() => expect(admitted).toHaveLength(1));
    await vi.waitFor(() => expect(isSaving("New")).toBe(true));
    await expectLocalRefusal(createPage("New", dto()), "page-saving");
    answer(admitted[0], 5);
    expect(await saving).toBe(true);
  });

  it("keeps a graph switch while the create is being published distinct from disk conflict", async () => {
    let publish!: (done: boolean) => void;
    vi.spyOn(backend(), "pageWait").mockImplementationOnce(() => new Promise((resolve) => { publish = resolve; }));
    const pending = createPage("New", dto());
    await vi.waitFor(() => expect(publish).toBeTypeOf("function"));
    resetStore();
    publish(true);
    await expectLocalRefusal(pending, "graph-changed");
  });

  it("settles a create the host has not answered yet when the graph switches", async () => {
    const admitted = holdAnswers();
    const pending = createPage("New", dto());
    let settled = false;
    void pending.catch(() => undefined).finally(() => { settled = true; });
    await vi.waitFor(() => expect(admitted).toHaveLength(1));
    resetStore();
    // The old session's answer never reaches the window after the switch (its
    // client was dropped); the create must not wait for it forever.
    await vi.waitFor(() => expect(settled).toBe(true));
    await expectLocalRefusal(pending, "graph-changed");
  });

  it("refuses the backend disk-conflict token when the file already exists", async () => {
    vi.spyOn(backend(), "pageOpen").mockImplementationOnce(async (_session, id, request) => {
      const key = request.path ?? "pages/New.md";
      queueMicrotask(() => host.deliver({ key, page: { version: 2, conflict: false, risk: false,
        disk: { kind: "file", rev: "on-disk" }, text: { kind: "page", dto: { ...dto(), rev: "on-disk" } } },
      answer: { id, version: 2, took: false, outcome: { kind: "applied" } } }));
      return { key, baselineEntry: true };
    });
    const submit = vi.spyOn(backend(), "pageSubmit");
    const error = await createPage("New", dto()).catch((e: unknown) => e);
    expect(error).not.toBeInstanceOf(CreatePageRefusal);
    expect(errorFamily(error)).toBe("conflict");
    expect(submit).not.toHaveBeenCalled();
  });
});
