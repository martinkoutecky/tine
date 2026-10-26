// og I1c (port of master c68c0b6e7, Direct Files data-safety audit F17): a
// conflict raised by a transient removal (an external editor's temp+rename, a
// mid-delivery sync pass) is lifted when the file provably comes back to the
// editor's own baseline, and the edit it froze is saved. A file that returns
// with different bytes stays conflicted. The page host detects and lifts the
// conflict (step 3b) and tells the window by page mail; the window's watcher
// entry point (applyGraphChange) must leave a host-held page to that mail.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "../render/parse";
import { backend, type GraphChange } from "../backend";
import { applyGraphChange, ensurePageLoaded, flushAll, flushPage, installExternalChangeUiHandler, isConflicted, pageByName, resetStore, setRaw } from "./index";
import { doc } from "./model";
import { setToasts } from "../toasts";
import type { PageDto } from "../types";
import type { MailPage } from "./host/protocol";
import { answerOpensFromDocument } from "./host/documentHost.test.support";
import { bindTestHost, type TestHost } from "./host/wiring.test.support";

const NAME = "Synced";
const dto = (rev: string, raw: string): PageDto & { id: string; rev: string } => ({
  id: `pages/${NAME}.md`, name: NAME, title: NAME, kind: "page", pre_block: null, rev,
  blocks: [{ id: "b1", raw, collapsed: false, children: [] }],
});
const event = (patch: Partial<GraphChange>): GraphChange => ({ name: NAME, kind: "page", created: false, removed: false, ...patch });
const raws = () => pageByName(NAME)?.roots.map((id) => doc.byId[id].raw) ?? [];

let disk: (PageDto & { id: string; rev: string }) | null;
let host: TestHost;
let left: string[];
let submit: { mock: { calls: unknown[][] } };
let key: string;
beforeAll(() => initParser());
beforeEach(async () => {
  resetStore(); setToasts([]); disk = null; left = [];
  host = await bindTestHost();
  answerOpensFromDocument(host);
  vi.spyOn(backend(), "getPage").mockImplementation(async () => disk as never);
  vi.spyOn(backend(), "getPageByPath").mockImplementation(async () => disk as never);
  installExternalChangeUiHandler(() => ({ pageOpen: () => true, journalsOpen: false, leaveRemovedPage: (name) => { left.push(name); },
    restartJournalFeed: () => {} }));
});
afterEach(() => { resetStore(); vi.restoreAllMocks(); });

/** The host's page state after `version`: conflicted or not, over the given disk. */
const state = (version: number, conflict: boolean, rev: string | null): MailPage => ({ version, conflict, risk: false,
  disk: rev ? { kind: "file", rev } : { kind: "no-file" }, text: { kind: "unchanged" } });

/** "mine" typed and taken by the host just as the file vanished (an external
 * editor's temp+rename): the host answers it conflicted over the removal. */
async function dirtyThenRemoved() {
  ensurePageLoaded(dto("rev-1", "original"));
  const admitted: Array<{ id: number; key: string }> = [];
  submit = vi.spyOn(backend(), "pageSubmit").mockImplementation(async (_session, id, sent) => { admitted.push({ id, key: sent }); return null; });
  setRaw(pageByName(NAME)!.roots[0], "mine");
  void flushPage(NAME);
  await vi.waitFor(() => expect(admitted).toHaveLength(1));
  key = admitted[0].key;
  host.deliver({ key, answer: { id: admitted[0].id, version: 9, took: true, outcome: { kind: "applied" } },
    notice: { conflictReported: true }, page: state(9, true, null) });
  await applyGraphChange(event({ removed: true }));
  expect(isConflicted(NAME)).toBe(true);
  expect(left).toEqual([]);
  expect(await flushAll()).toBe(false);
}

describe("a transient removal's conflict (og I1c, master c68c0b6e7)", () => {
  it("is lifted when the file returns to the baseline, and the frozen edit publishes without being sent again", async () => {
    await dirtyThenRemoved();
    disk = dto("rev-1", "original");
    host.deliver({ key, answer: null, page: state(10, false, "rev-1") });
    await applyGraphChange(event({ created: true }));

    expect(isConflicted(NAME)).toBe(false);
    expect(raws()).toEqual(["mine"]);
    await expect(flushAll()).resolves.toBe(true);
    // The host kept the edit through the removal: the window never dropped or resent it.
    expect(submit).toHaveBeenCalledTimes(1);
  });

  it("stays when the file returns with different bytes", async () => {
    await dirtyThenRemoved();
    disk = dto("rev-9", "theirs");
    host.deliver({ key, answer: null, page: state(10, true, "rev-9") });
    await applyGraphChange(event({ created: true }));

    expect(isConflicted(NAME)).toBe(true);
    expect(raws()).toEqual(["mine"]);
    expect(await flushAll()).toBe(false);
    expect(submit).toHaveBeenCalledTimes(1);
  });
});
