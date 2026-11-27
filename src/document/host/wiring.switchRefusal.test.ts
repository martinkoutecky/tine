// tine#1041: the switch barrier (`flushAllUnsettled`) says what it refused for.
// A refused publication wait names its stuck files and the barrier carries them
// unchanged; every other failure carries its reason, and the boolean every
// other caller reads (`flushAll`) is the same as before.
import { afterEach, expect, it, vi } from "vitest";
import { backend } from "../../backend";
import type { PageDto } from "../../types";
import { flushAll, flushAllUnsettled, resetStore, setRaw, unsavedDrafts } from "../index";
import { pendingConflictCount } from "../../liveConflicts";
import { loadRoutedPage } from "../workingSet";
import { bindTestHost } from "./wiring.test.support";

afterEach(() => { resetStore(); vi.restoreAllMocks(); });

const KEY = "pages/Repro 964.md";

function dto(name: string): PageDto & { id: string } {
  return { name, kind: "page", title: name, id: `pages/${name}.md`, rev: "r1", pre_block: null,
    blocks: [{ id: `${name}-block`, raw: name, collapsed: false, children: [] }] };
}

it("a draft only the host holds, which never publishes: the barrier names its key", async () => {
  await bindTestHost();
  vi.spyOn(backend(), "pageOwed").mockResolvedValue([{ key: KEY, version: 2 }]);
  const wait = vi.spyOn(backend(), "pageWait").mockResolvedValue({ refused: [KEY] });
  expect(await flushAllUnsettled(false)).toEqual({ kind: "stuck", files: [KEY] });
  expect(wait.mock.calls[0]?.[1]).toEqual([{ key: KEY, version: 2 }]);
  expect(await flushAll()).toBe(false);
  // The rows this fix does not change: the panel and the badge still do not list it.
  expect(unsavedDrafts()).toEqual([]);
  expect(pendingConflictCount()).toBe(0);
});

it("a window page whose save keeps failing (tine#1033's shape): the barrier names that page's file", async () => {
  loadRoutedPage(dto("Failing"));
  await bindTestHost();
  const submit = vi.spyOn(backend(), "pageSubmit");
  setRaw("Failing-block", "typed", { timetracking: false });
  await vi.waitFor(() => expect(submit).toHaveBeenCalledOnce(), { timeout: 2000 });
  // The host owes the version it took (its save keeps failing), and the native
  // wait refuses for that needed key (its save-error notice).
  vi.spyOn(backend(), "pageOwed").mockResolvedValue([{ key: "pages/Failing.md", version: 2 }]);
  vi.spyOn(backend(), "pageWait").mockImplementation(async (_session, needs) =>
    ({ refused: needs.filter((need) => need.key === "pages/Failing.md").map((need) => need.key) }));
  expect(await flushAllUnsettled(false)).toEqual({ kind: "stuck", files: ["pages/Failing.md"] });
});

// Step 2: leaving for another graph. The host sets aside a draft only it holds
// (no window has worked on the page, its exact copy is recovered at the next launch):
// its wait answers with no revision for that key, the host still lists the key
// as owed, and the barrier passes. A refusal is carried as before.
it("a departure passes over a draft only the host holds; a refusal still names its key", async () => {
  await bindTestHost();
  vi.spyOn(backend(), "pageOwed").mockResolvedValue([{ key: KEY, version: 2 }]);
  const wait = vi.spyOn(backend(), "pageWait").mockResolvedValue([]);
  expect(await flushAllUnsettled(true)).toBeNull();
  expect(wait.mock.calls.map((call) => [call[1], call[3]])).toEqual([[[{ key: KEY, version: 2 }], true]]);
  wait.mockClear();
  wait.mockResolvedValue({ refused: [KEY] });
  expect(await flushAllUnsettled(true)).toEqual({ kind: "stuck", files: [KEY] });
  expect(await flushAllUnsettled(false)).toEqual({ kind: "stuck", files: [KEY] });
  expect(wait.mock.calls.map((call) => call[3])).toEqual([true, false]);
});

it.each([
  ["the debt query is unanswered", () => vi.spyOn(backend(), "pageOwed").mockResolvedValue(null)],
  ["the debt query fails", () => vi.spyOn(backend(), "pageOwed").mockRejectedValue(new Error("ipc"))],
  ["the wait call is rejected", () => vi.spyOn(backend(), "pageWait").mockRejectedValue(new Error("ipc"))],
  ["the wait refuses naming no key", () => vi.spyOn(backend(), "pageWait").mockResolvedValue(false)],
  ["the wait refuses with an empty key list", () => vi.spyOn(backend(), "pageWait").mockResolvedValue({ refused: [] })],
])("no answer when %s: refused, as before", async (_label, arrange) => {
  await bindTestHost();
  vi.spyOn(backend(), "pageOwed").mockResolvedValue([{ key: KEY, version: 2 }]);
  arrange();
  expect(await flushAllUnsettled(false)).toEqual({ kind: "no-answer" });
  expect(await flushAll()).toBe(false);
});

it("still busy: work arriving across every round is refused as busy, as before", async () => {
  loadRoutedPage(dto("Busy"));
  await bindTestHost();
  vi.spyOn(backend(), "pageOwed").mockResolvedValue([{ key: KEY, version: 2 }]);
  let typed = 0;
  vi.spyOn(backend(), "pageWait").mockImplementation(async (_session, needs) => {
    setRaw("Busy-block", `typed ${typed++}`, { timetracking: false });
    return needs.map(({ key, version }) => ({ key, version, rev: null }));
  });
  expect(await flushAllUnsettled(false)).toEqual({ kind: "busy" });
});

it("no client with early edits: refused as unbound, as before", async () => {
  loadRoutedPage(dto("Early"));
  setRaw("Early-block", "typed before any binding", { timetracking: false });
  expect(await flushAllUnsettled(false)).toEqual({ kind: "unbound" });
  expect(await flushAll()).toBe(false);
});

it("no client and no edits: settled, as before", async () => {
  expect(await flushAllUnsettled(false)).toBeNull();
  expect(await flushAll()).toBe(true);
});

it("every need published: settled, as before", async () => {
  await bindTestHost();
  vi.spyOn(backend(), "pageOwed").mockResolvedValueOnce([{ key: KEY, version: 2 }]).mockResolvedValue([]);
  vi.spyOn(backend(), "pageWait").mockResolvedValue([{ key: KEY, version: 2, rev: null }]);
  expect(await flushAllUnsettled(false)).toBeNull();
});
