// GH #337 (master 9869c1cfe): an external change declined while its page is held
// (block move in flight, component draft) is replayed when the hold ends, through
// the same `applyGraphChange` entry point a live watcher event uses. Each hold
// kind is driven through its real release. A page being edited is open in the
// page host (STEP3 §4.2, B-Q2): its disk change arrives as host mail, never as a
// window re-read, and is installed with the editor kept on its block.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "../render/parse";
import { backend, type GraphChange } from "../backend";
import { applyGraphChange, ensurePageLoaded, loadFeed, pageByName, pinPageWhileDrafting, resetStore, setRaw, withBlockMoving } from "./index";
import { registerPaneRouteProvider } from "./workingSet";
import { doc } from "./model";
import { editingId, endEdit, startEditing } from "../editorController";
import type { BlockDto, PageDto } from "../types";
import { answerOpensFromDocument } from "./host/documentHost.test.support";
import { bindTestHost, mailPage, type TestHost } from "./host/wiring.test.support";

let serial = 0;
const block = (raw: string): BlockDto => ({ id: `dr-${++serial}`, raw, collapsed: false, children: [] });
const page = (name: string, raws: string[]): PageDto & { id: string; rev: string } => ({
  id: `pages/${name}.md`, name, title: name, kind: "page", pre_block: null, rev: `rev-${serial}`, blocks: raws.map(block),
});
const raws = (name: string) => pageByName(name)?.roots.map((id) => doc.byId[id].raw) ?? [];
const changed = (name: string): GraphChange => ({ name, kind: "page", created: false, removed: false });

let disk: PageDto & { id: string; rev: string };
let reads = 0;
let host: TestHost;
beforeAll(() => initParser());
beforeEach(async () => {
  serial = 0;
  resetStore();
  reads = 0;
  vi.spyOn(backend(), "getPage").mockImplementation(async () => { reads++; return disk as never; });
  host = await bindTestHost();
  answerOpensFromDocument(host);
});
afterEach(() => {
  endEdit("graph-switch");
  registerPaneRouteProvider(() => []);
  vi.restoreAllMocks();
});

function load(name: string, mine: string[]) {
  loadFeed([page("Journal", ["j"])]);
  ensurePageLoaded(page(name, mine));
}

describe("an external change declined mid-edit is replayed (GH #337)", () => {
  let version = 5000;
  /** The host's push of P's new disk text, as the native host mails it. */
  const pushed = (raws: string[]) => {
    const dto = page("P", raws);
    host.deliver({ key: dto.id, answer: null, page: mailPage(++version, dto) });
  };
  /** P open in the host for the editor (its Open answered). */
  async function editing(): Promise<string> {
    const id = pageByName("P")!.roots[0];
    startEditing(id, 0);
    await vi.waitFor(() => expect(backend().pageOpen).toHaveBeenCalled());
    await new Promise((resolve) => setTimeout(resolve, 0));
    return id;
  }

  it("a page being edited takes its disk change as host mail, with the editor kept on its block", async () => {
    load("P", ["mine"]);
    await editing();
    disk = page("P", ["from disk"]);
    await applyGraphChange(changed("P"));
    // The watcher event does not re-read a page the host holds.
    expect([raws("P"), reads]).toEqual([["mine"], 0]);
    pushed(["from disk"]);
    await vi.waitFor(() => expect(raws("P")).toEqual(["from disk"]));
    // The caret is never stolen: the editor is still on P's (only) block.
    expect(editingId()).toBe(pageByName("P")!.roots[0]);
    expect(reads).toBe(0);
  });

  it("replays after a block move settles", async () => {
    load("P", ["mine"]);
    let settle!: () => void;
    const moving = withBlockMoving("P", () => new Promise<void>((resolve) => { settle = resolve; }));
    disk = page("P", ["from disk"]);
    await applyGraphChange(changed("P"));
    expect(raws("P")).toEqual(["mine"]);
    settle();
    await moving;
    await vi.waitFor(() => expect(raws("P")).toEqual(["from disk"]));
  });

  it("replays after a component draft (title rename, sheet cell) is released", async () => {
    load("P", ["mine"]);
    const unpin = pinPageWhileDrafting(() => "P");
    disk = page("P", ["from disk"]);
    await applyGraphChange(changed("P"));
    expect(raws("P")).toEqual(["mine"]);
    unpin();
    await vi.waitFor(() => expect(raws("P")).toEqual(["from disk"]));
  });

  it("the latest host observation wins and the window reads the page no time", async () => {
    load("P", ["mine"]);
    await editing();
    pushed(["first"]);
    pushed(["second"]);
    endEdit("blur");
    await vi.waitFor(() => expect(raws("P")).toEqual(["second"]));
    expect(reads).toBe(0);
  });

  it("host content mailed under a component draft stays held until the draft is released", async () => {
    load("P", ["mine"]);
    await editing();
    const unpin = pinPageWhileDrafting(() => "P");
    pushed(["from disk"]);
    endEdit("blur");
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(raws("P")).toEqual(["mine"]); // the draft still holds the page
    unpin();
    await vi.waitFor(() => expect(raws("P")).toEqual(["from disk"]));
  });

  it("never clobbers: a page that became dirty meanwhile keeps its edit", async () => {
    load("P", ["mine"]);
    startEditing(pageByName("P")!.roots[0], 0);
    disk = page("P", ["from disk"]);
    await applyGraphChange(changed("P"));
    setRaw(pageByName("P")!.roots[0], "typed but not saved");
    pushed(["from disk"]);
    endEdit("blur");
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(raws("P")).toEqual(["typed but not saved"]);
  });

  it("a graph switch discards the deferred change", async () => {
    load("P", ["mine"]);
    let settle!: () => void;
    const moving = withBlockMoving("P", () => new Promise<void>((resolve) => { settle = resolve; }));
    disk = page("P", ["from disk"]);
    await applyGraphChange(changed("P"));
    resetStore();
    load("P", ["other graph"]);
    settle();
    await moving;
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(raws("P")).toEqual(["other graph"]);
    expect(reads).toBe(0);
  });

  it("a graph switch while the edited page's Open is in flight never shows the old graph's text", async () => {
    load("P", ["mine"]);
    startEditing(pageByName("P")!.roots[0], 0);
    disk = page("P", ["from disk"]);
    await applyGraphChange(changed("P"));
    resetStore();
    load("P", ["other graph"]);
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(raws("P")).toEqual(["other graph"]);
    expect(reads).toBe(0);
  });
});
