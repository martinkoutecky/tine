import { afterEach, expect, it, vi } from "vitest";
import { setToasts, toasts } from "./toasts";
import { backend } from "./backend";
import { resetStore } from "./document";
import { graphMeta, setGraphMeta, setJournalTemplate } from "./graphSession";

afterEach(() => { setGraphMeta(null); vi.restoreAllMocks(); });

it("does not roll back the new graph's template when an old config write fails", async () => {
  const loaded = await backend().loadGraph("");
  if (loaded.kind === "focused_existing") throw new Error("no graph metadata");
  setGraphMeta({ ...loaded.meta, root: "/old", default_journal_template: "Old" });
  let fail!: (error: Error) => void;
  vi.spyOn(backend(), "setDefaultJournalTemplate").mockImplementationOnce(() => new Promise((_, reject) => { fail = reject; }));
  setJournalTemplate("Changed");
  // Writes are queued (microtask); let the old graph's write actually start so
  // it is in flight when the graph changes, as it is for a real click.
  for (let i = 0; i < 20; i += 1) await Promise.resolve();
  resetStore();
  setGraphMeta({ ...loaded.meta, root: "/new", default_journal_template: "New" });
  fail(new Error("old graph disk error"));
  await Promise.resolve();
  await Promise.resolve();
  expect(graphMeta()?.default_journal_template).toBe("New");
});

async function seed(root: string, template: string | null) {
  const loaded = await backend().loadGraph("");
  if (loaded.kind === "focused_existing") throw new Error("no graph metadata");
  setGraphMeta({ ...loaded.meta, root, default_journal_template: template });
  setToasts([]);
}
const settle = async () => { for (let i = 0; i < 20; i += 1) await Promise.resolve(); };

it("I-20: a failed OLDER template write does not roll back the newer choice", async () => {
  await seed("/tpl-order-1", "Disk");
  const rejects: Array<(e: Error) => void> = [];
  const resolves: Array<() => void> = [];
  vi.spyOn(backend(), "setDefaultJournalTemplate").mockImplementation(() => new Promise((resolve, reject) => {
    resolves.push(() => resolve(undefined as never));
    rejects.push(reject);
  }));
  setJournalTemplate("First");
  setJournalTemplate("Second");
  await settle();
  // Writes are ordered: only the first has started.
  expect(rejects.length).toBe(1);
  rejects[0](new Error("disk full"));
  await settle();
  expect(rejects.length).toBe(2);
  resolves[1]();
  await settle();
  expect(graphMeta()?.default_journal_template).toBe("Second");
  expect(toasts().some((t) => t.kind === "error")).toBe(true);
});

it("I-20: a failed newest write restores the last PERSISTED template, not its neighbour", async () => {
  await seed("/tpl-order-2", "Disk");
  const settleCalls: Array<{ ok: () => void; fail: (e: Error) => void }> = [];
  vi.spyOn(backend(), "setDefaultJournalTemplate").mockImplementation(() => new Promise((resolve, reject) => {
    settleCalls.push({ ok: () => resolve(undefined as never), fail: reject });
  }));
  setJournalTemplate("First");
  setJournalTemplate("Second");
  await settle();
  settleCalls[0].ok();
  await settle();
  settleCalls[1].fail(new Error("disk full"));
  await settle();
  expect(graphMeta()?.default_journal_template).toBe("First");
});

it("I-9: a config write that fails after the graph changed still tells the user", async () => {
  await seed("/tpl-switch", "Old");
  let fail!: (error: Error) => void;
  vi.spyOn(backend(), "setDefaultJournalTemplate").mockImplementationOnce(() => new Promise((_, reject) => { fail = reject; }));
  setJournalTemplate("Changed");
  await settle();
  resetStore();
  await seed("/tpl-switch-new", "New");
  fail(new Error("old graph disk error"));
  await settle();
  expect(graphMeta()?.default_journal_template).toBe("New");
  expect(toasts().some((t) => t.kind === "error" && t.message.includes("journal template"))).toBe(true);
});
