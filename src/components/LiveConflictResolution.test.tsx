// og 21a (Concord 8e): a live editor draft the page host could not save because
// its file changed on disk is reviewed and resolved at the page: the read-only
// native merge produces the text and the host takes it on the reviewed disk
// state only (STEP3 §9). Real document store and wired page host over a small
// fake of the native host (one file); the backend is stubbed. Synthetic content.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi, type MockInstance } from "vitest";
import { render } from "solid-js/web";
import { backend, type Backend } from "../backend";
import { initParser } from "../render/parse";
import { conflictReason, flushAll, flushPage, isConflicted, isDirty, loadFeed, pageByName, resetStore, setRaw } from "../document";
import { doc } from "../document/model";
import { bindTestHost, type TestHost } from "../document/host/wiring.test.support";
import { STALE_VERSION, type MailPage, type MailText } from "../document/host/protocol";
import { liveConflictForPage, liveConflictObjects } from "../liveConflicts";
import { ConflictOverview } from "./ConflictOverview";
import { ConflictQueueBadge } from "./Sidebar";
import type { PaneRouter } from "../router";
import { setToasts, toasts } from "../toasts";
import { PageConflictResolution } from "./ConflictResolution";
import type { BlockDto, ConflictObject, DiffRow, PageDto, SyncConflictDiff } from "../types";

const PATH = "pages/P.md";
const block = (id: string, raw: string): BlockDto => ({ id, raw, collapsed: false, children: [] });
const page = (rev: string, raw: string) => ({ id: PATH, name: "P", title: "P", kind: "page" as const, pre_block: null, rev, blocks: [block("p1", raw)] });
const view = (text: string) => ({ uuid: "", text, child_count: 0 });
const rows: DiffRow[] = [{ id: "0", kind: "modified", mine: view("mine"), theirs: view("theirs"), children: [] }];
const review = (conflictRev: string): SyncConflictDiff => ({ base_rev: "r1", conflict_rev: conflictRev, rows,
  mine_pre: null, theirs_pre: null, pre_differs: false, blocks_identical: false });
const raw = () => doc.byId[pageByName("P")!.roots[0]]?.raw;

// The native host for PATH: the file on disk, the buffer's version, and whether
// the window's input conflicts with the file. A submit authored on the current
// version writes; one on an older version (or over a conflict) is taken as
// input that conflicts; a reviewed submit writes only on the reviewed file.
let host: TestHost;
let disk: { rev: string; dto: PageDto };
let version = 1;
let conflict = false;
let nextRev = 3;
let hold: ((release: () => void) => void) | null = null;
let api: Required<Backend>;
let submit: MockInstance<Backend["pageSubmit"]>;
const replaceSubmits = () => submit.mock.calls.filter((call) => call[6].includes("replace-page"));

function mail(id: number, took: boolean, text: MailText) {
  const page: MailPage = { version, conflict, risk: false, disk: { kind: "file", rev: disk.rev }, text };
  host.deliver({ key: PATH, page, answer: { id, version, took, outcome: { kind: "applied" } }, notice: { conflictReported: conflict } });
}

beforeAll(() => initParser());
beforeEach(async () => {
  vi.useFakeTimers();
  resetStore();
  setToasts([]);
  // Another editor wrote r2 after the page loaded at r1.
  disk = { rev: "r2", dto: page("r2", "theirs") };
  version = 1;
  conflict = false;
  nextRev = 3;
  hold = null;
  api = backend() as Required<Backend>;
  loadFeed([page("r1", "first")] as never);
  host = await bindTestHost();
  vi.spyOn(api, "pageOpen").mockImplementation(async (_session, id) => {
    queueMicrotask(() => mail(id, false, { kind: "page", dto: structuredClone(disk.dto) }));
    return { key: PATH, baselineEntry: true };
  });
  submit = vi.spyOn(api, "pageSubmit").mockImplementation(async (_session, id, _key, dto, at, resolve) => {
    const reviewed = resolve?.kind === "file" && resolve.rev === disk.rev;
    const answer = () => {
      if (reviewed || (!conflict && at === version && at !== STALE_VERSION)) {
        disk = { rev: `r${nextRev++}`, dto: structuredClone(dto) };
        version += 1;
        conflict = false;
        mail(id, true, { kind: "unchanged", rev: disk.rev });
      } else {
        version += 1;
        conflict = true;
        mail(id, true, { kind: "unchanged" });
      }
    };
    if (hold) hold(answer);
    else queueMicrotask(answer);
    return null;
  });
  vi.spyOn(api, "liveConflictDiff").mockImplementation(async () => review("r2"));
});
afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); document.body.innerHTML = ""; });

const tick = async () => { for (let i = 0; i < 8; i++) await vi.advanceTimersByTimeAsync(0); };

async function conflictedDraft(text = "mine") {
  setRaw("p1", text);
  expect(await flushPage("P")).toBe(false);
  expect(conflictReason("P")).toMatchObject({ kind: "disk-changed", observedRev: "r2" });
}

function mount(conflict: ConflictObject) {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(() => <PageConflictResolution conflict={conflict} />, root);
  return { root, dispose };
}
const apply = (root: HTMLElement) => (root.querySelector(".settings-btn-primary") as HTMLButtonElement).click();

describe("Concord live-draft conflicts (og 8e)", () => {
  it("reviews the draft against disk and resolves on the reviewed file, then installs the result", async () => {
    await conflictedDraft();
    const conflict = liveConflictForPage("P", PATH)!;
    expect(conflict).toMatchObject({ source: "live-save", page_path: PATH, live: { base_rev: "r1" } });
    const merge = vi.spyOn(api, "mergeLiveConflict").mockImplementation(async (_p, draft) =>
      ({ ...draft, blocks: [block("p1", "mine + theirs")] }));
    const { root, dispose } = mount(conflict);
    await tick();
    expect(api.liveConflictDiff).toHaveBeenCalledWith(PATH, expect.objectContaining({ name: "P" }), "r1");
    expect(vi.mocked(api.liveConflictDiff).mock.calls[0][1].blocks[0].raw).toBe("mine");
    apply(root);
    await tick();
    expect(merge).toHaveBeenCalledTimes(1);
    const [path, draft, conflictRev] = merge.mock.calls[0];
    expect([path, draft.blocks[0].raw, conflictRev]).toEqual([PATH, "mine", "r2"]);
    // The merged text is submitted as a replacement on exactly the reviewed file.
    expect(replaceSubmits()).toHaveLength(1);
    expect(replaceSubmits()[0][5]).toEqual({ kind: "file", rev: "r2" });
    expect(replaceSubmits()[0][3].blocks[0].raw).toBe("mine + theirs");
    expect(disk.rev).toBe("r3");
    expect(raw()).toBe("mine + theirs");
    expect(isConflicted("P") || isDirty("P")).toBe(false);
    // The written revision is the new baseline: the next edit saves on it.
    const resolved = version;
    setRaw("p1", "after");
    await flushAll();
    expect(submit.mock.calls.at(-1)![4]).toBe(resolved);
    expect(disk.dto.blocks[0].raw).toBe("after");
    expect(isConflicted("P")).toBe(false);
    dispose();
  });

  it("a draft typed after the review needs a fresh review and is never replaced", async () => {
    await conflictedDraft();
    const merge = vi.spyOn(api, "mergeLiveConflict");
    const { root, dispose } = mount(liveConflictForPage("P", PATH)!);
    await tick();
    setRaw("p1", "mine, typed after the review");
    apply(root);
    await tick();
    expect(merge).not.toHaveBeenCalled();
    expect(replaceSubmits()).toEqual([]);
    expect(api.liveConflictDiff).toHaveBeenCalledTimes(2);
    expect(vi.mocked(api.liveConflictDiff).mock.calls[1][1].blocks[0].raw).toBe("mine, typed after the review");
    expect(raw()).toBe("mine, typed after the review");
    expect(toasts().some((t) => /draft or the file changed/.test(t.message))).toBe(true);
    dispose();
  });

  it("a newer disk write refuses the apply, writes nothing and refreshes the review", async () => {
    await conflictedDraft();
    vi.spyOn(api, "mergeLiveConflict").mockRejectedValue("conflict");
    const { root, dispose } = mount(liveConflictForPage("P", PATH)!);
    await tick();
    apply(root);
    await tick();
    expect(api.liveConflictDiff).toHaveBeenCalledTimes(2);
    expect(replaceSubmits()).toEqual([]);
    expect(disk.rev).toBe("r2");
    expect(raw()).toBe("mine");
    expect(isConflicted("P")).toBe(true);
    dispose();
  });

  it("edits typed while the merge was computed are kept and never replaced by it", async () => {
    await conflictedDraft();
    vi.spyOn(api, "mergeLiveConflict").mockImplementation(async (_p, draft) => {
      setRaw("p1", "typed during the merge");
      return { ...draft, blocks: [block("p1", "resolved")] };
    });
    const { root, dispose } = mount(liveConflictForPage("P", PATH)!);
    await tick();
    apply(root);
    await tick();
    expect(replaceSubmits()).toEqual([]);
    expect(raw()).toBe("typed during the merge");
    expect(conflictReason("P")).toMatchObject({ kind: "disk-changed", observedRev: "r2" });
    expect(toasts().some((t) => /draft or the file changed/.test(t.message))).toBe(true);
    dispose();
  });

  it("a merge that finishes after a graph switch writes nothing in the next graph", async () => {
    await conflictedDraft("graph A draft");
    let finish!: () => void;
    vi.spyOn(api, "mergeLiveConflict").mockImplementation((_p, draft) =>
      new Promise((resolve) => { finish = () => resolve({ ...draft, blocks: [block("p1", "merged A")] }); }));
    const { root, dispose } = mount(liveConflictForPage("P", PATH)!);
    await tick();
    apply(root);
    await tick();
    expect(finish).toBeTypeOf("function");
    // Graph B has a page with the same name.
    resetStore();
    loadFeed([page("r1", "graph B")] as never);
    finish();
    await tick();
    expect(replaceSubmits()).toEqual([]);
    expect(raw()).toBe("graph B");
    expect(isConflicted("P")).toBe(false);
    dispose();
  });

  it("L10:63: a resolution answered after a switch never marks graph B conflicted", async () => {
    await conflictedDraft("graph A draft");
    vi.spyOn(api, "mergeLiveConflict").mockImplementation(async (_p, draft) => ({ ...draft, blocks: [block("p1", "merged A")] }));
    let release: (() => void) | null = null;
    hold = (answer) => { release = answer; };
    const read = vi.spyOn(api, "getPage");
    const { root, dispose } = mount(liveConflictForPage("P", PATH)!);
    await tick();
    apply(root);
    await tick();
    expect(replaceSubmits()).toHaveLength(1);
    expect(release).toBeTypeOf("function");
    resetStore();
    loadFeed([page("r2", "B disk")] as never);
    setRaw("p1", "B edit");
    release!();
    await tick();
    expect(raw()).toBe("B edit");
    expect(isConflicted("P"), "the old graph's answer cannot adopt graph B's page").toBe(false);
    expect(read).not.toHaveBeenCalled();
    dispose();
  });

  it("the sidebar badge counts an open live draft (master: one combined queue)", async () => {
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <ConflictQueueBadge />, root);
    const badge = () => root.querySelector(".conflict-queue-badge")?.textContent ?? null;
    expect(badge()).toBeNull();
    await conflictedDraft("open draft");
    await tick();
    expect(badge()).toBe("1 conflict");
    dispose();
  });

  it("22a: the overview lists an open draft under Unsaved drafts, and opens the page to review", async () => {
    await conflictedDraft("open draft");
    expect(liveConflictObjects().map((c) => [c.page_name, c.live?.base_rev])).toEqual([["P", "r1"]]);
    const opened: unknown[] = [];
    vi.spyOn(api, "listSyncConflicts").mockResolvedValue([]);
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <ConflictOverview router={{ openPageTarget: (t: unknown) => opened.push(t) } as unknown as PaneRouter} />, root);
    const group = root.querySelector('[aria-label="Unsaved drafts"]')!;
    expect(group.textContent).toContain("unsaved draft");
    (group.querySelector(".conflict-overview-open") as HTMLButtonElement).click();
    expect(opened).toEqual([{ name: "P", pageKind: "page", path: PATH }]);
    dispose();
  });
});
