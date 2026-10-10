import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { loadSingle, resetStore } from "./workingSet";
import { flushPage, isConflicted, markDirty } from "./host/wiring";
import { bindTestHost } from "./host/wiring.test.support";

beforeAll(() => initParser());
afterEach(() => {
  resetStore();
  vi.restoreAllMocks();
});

const page = (raw: string) => ({ name: "Old page", kind: "page" as const, title: "Old page", pre_block: null,
  blocks: [{ id: "o1", raw, collapsed: false, children: [] }] });

it("clears a page conflict when the document store resets", async () => {
  resetStore();
  const host = await bindTestHost();
  loadSingle(page("mine"));
  const admitted: Array<{ id: number; key: string }> = [];
  const submit = vi.spyOn(backend(), "pageSubmit").mockImplementation(async (_session, id, key) => { admitted.push({ id, key }); return null; });
  markDirty("Old page", "save-block");
  void flushPage("Old page");
  await vi.waitFor(() => expect(admitted).toHaveLength(1));
  host.deliver({ key: admitted[0].key, answer: { id: admitted[0].id, version: 5, took: true, outcome: { kind: "applied" } },
    notice: { conflictReported: true },
    page: { version: 5, conflict: true, risk: false, disk: { kind: "file", rev: "theirs" }, text: { kind: "unchanged" } } });
  expect(isConflicted("Old page")).toBe(true);
  submit.mockRestore();

  resetStore();
  expect(isConflicted("Old page")).toBe(false);
  // The next graph's binding (a fresh window session) starts with no conflict,
  // also for a page of the same name.
  await bindTestHost();
  loadSingle(page("other graph"));
  expect(isConflicted("Old page")).toBe(false);
});
