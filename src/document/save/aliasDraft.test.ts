import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../../backend";
import { isConflicted, loadFeed, pageByName, resetStore, setRaw } from "../index";
import { doc } from "../model";
import { initParser } from "../../render/parse";
import { setToasts, toasts } from "../../toasts";
import { startEditing, endEdit } from "../../editorController";
import { bindTestHost, mailPage, submittedPages, type TestHost } from "../host/wiring.test.support";
import type { BlockDto, PageDto, PageRead } from "../../types";

// L13 (B): a page with no file whose name is an alias lands its draft in the
// alias owner (STEP3 §8, wiring `landAliasDraft`): the owner's live text gets
// the draft appended and saved, then the draft leaves. An edit typed while
// that save was in flight keeps the draft; the next landing must write the
// draft's CURRENT content in place of the landed copy, never append the whole
// draft a second time.

const OWNER = "pages/Owner.md";
let serial = 0;
const block = (raw: string): BlockDto => ({ id: `alias-${++serial}`, raw, collapsed: false, children: [] });
const page = (name: string, raws: string[], rev: string): PageRead => ({
  id: `pages/${name}.md`, name, title: name, kind: "page", pre_block: null, rev, blocks: raws.map(block),
});
const raws = (dto: PageDto) => dto.blocks.map((b) => b.raw);
const draft = (raws: string[]) => ({ ...page("Draft", raws, "x"), id: undefined, rev: undefined });

/** The owner's file as the host holds it. "Draft" resolves as an alias of
 * Owner; opening Owner answers with its file; a taken Owner text becomes the
 * file. `during` runs while the owner's save is being submitted. */
function ownerFile(host: TestHost, initial: string[], during?: (writes: number) => void) {
  const disk = { raws: initial.slice(), rev: "owner-0", writes: 0, version: 1, conflict: false };
  const api = backend();
  const submit = api.pageSubmit.bind(api);
  // A conflicted page is never published (`page_wait` is false on a conflict).
  vi.spyOn(api, "pageWait").mockImplementation(async (_session, needs) => !(disk.conflict && needs.some((n) => n.key === OWNER)));
  vi.spyOn(api, "getPageByPath").mockImplementation(async () => page("Owner", disk.raws, disk.rev));
  vi.spyOn(api, "pageOpen").mockImplementation(async (_session, id, target) => {
    if (target.path === null && target.name === "Draft") return { reason: "alias" as const, owners: [OWNER] };
    const key = target.path ?? `pages/${target.name}.md`;
    if (key === OWNER) {
      queueMicrotask(() => host.deliver({ key, page: mailPage(disk.version, page("Owner", disk.raws, disk.rev)),
        answer: { id, version: disk.version, took: false, outcome: { kind: "applied" } } }));
      return { key, baselineEntry: true };
    }
    return { reason: "failed" as const, message: `no file ${key}` };
  });
  const ownerSubmit = vi.spyOn(api, "pageSubmit").mockImplementation(async (session, id, key, dto, version, resolve, kinds) => {
    if (key !== OWNER) return submit(session, id, key, dto, version, resolve, kinds);
    if (version !== disk.version) {
      disk.conflict = true;
      // Text typed on a version the host has moved past: the host takes it as
      // the page's input and reports a conflict with the file (STEP3 §4.3).
      queueMicrotask(() => host.deliver({ key, notice: { conflictReported: true },
        page: { version: disk.version + 1, conflict: true, risk: true, disk: { kind: "file", rev: disk.rev }, text: { kind: "unchanged" } },
        answer: { id, version: disk.version + 1, took: true, outcome: { kind: "applied" } } }));
      return null;
    }
    disk.raws = raws(dto);
    disk.rev = `owner-${++disk.writes}`;
    disk.version += 1;
    during?.(disk.writes);
    queueMicrotask(() => host.deliver({ key, page: { version: disk.version, conflict: false, risk: false, text: { kind: "unchanged" } },
      answer: { id, version: disk.version, took: true, outcome: { kind: "applied" } } }));
    return null;
  });
  return { disk, ownerSubmit };
}

beforeAll(() => initParser());
beforeEach(() => { serial = 0; resetStore(); setToasts([]); });
afterEach(() => { vi.restoreAllMocks(); resetStore(); });

describe("alias draft landing", () => {
  it("a retry after a mid-save edit replaces the landed copy instead of appending the draft again", async () => {
    loadFeed([draft(["a", "b"])]);
    const host = await bindTestHost();
    const [, second] = pageByName("Draft")!.roots;
    // The user keeps typing in the draft while its first landing is saved.
    const { disk, ownerSubmit } = ownerFile(host, ["owner"], (writes) => { if (writes === 1) setRaw(second, "b2"); });
    setRaw(second, "b1");
    await vi.waitFor(() => expect(submittedPages(ownerSubmit, "Owner")).toHaveLength(1), { timeout: 2000 });
    expect(raws(submittedPages(ownerSubmit, "Owner")[0])).toEqual(["owner", "a", "b1"]);
    // The draft kept its later text; the next landing (the user's next edit
    // re-opens it) replaces the landed copy.
    await vi.waitFor(() => expect(vi.mocked(backend().pageOpen).mock.calls.filter((c) => c[2].name === "Draft")).toHaveLength(2));
    expect(pageByName("Draft")!.roots.map((id) => doc.byId[id].raw)).toEqual(["a", "b2"]);
    setRaw(second, "b3");
    await vi.waitFor(() => expect(pageByName("Draft")).toBeUndefined(), { timeout: 2000 });
    expect(submittedPages(ownerSubmit, "Owner")).toHaveLength(2);
    expect(disk.raws).toEqual(["owner", "a", "b3"]);
    expect(pageByName("Owner")!.roots.map((id) => doc.byId[id].raw)).toEqual(["owner", "a", "b3"]);
  });

  it("a landed draft whose owner changed since never writes over that change, and the draft keeps its text", async () => {
    loadFeed([draft(["a"])]);
    const host = await bindTestHost();
    const [first] = pageByName("Draft")!.roots;
    const { disk, ownerSubmit } = ownerFile(host, ["owner"], (writes) => { if (writes === 1) setRaw(first, "a2"); });
    setRaw(first, "a1");
    await vi.waitFor(() => expect(vi.mocked(backend().pageOpen).mock.calls.filter((c) => c[2].name === "Draft")).toHaveLength(2),
      { timeout: 2000 });
    expect(submittedPages(ownerSubmit, "Owner")).toHaveLength(1);
    // Another tool rewrote the owner's tail after the draft landed.
    disk.raws = ["owner", "rewritten elsewhere"];
    disk.rev = "external";
    disk.version += 1;
    host.deliver({ key: OWNER, answer: null, page: mailPage(disk.version, page("Owner", disk.raws, disk.rev)) });
    setRaw(first, "a3");
    // Either the window refuses the landing (the owner's tail is no longer the
    // landed copy) or the host reports the owner conflicted; the file keeps the
    // other tool's text and the draft is not dropped.
    await vi.waitFor(() => expect(toasts().some((t) => t.message
      === "“Owner” changed since “Draft” was added to it; copy the rest of “Draft” over by hand.") || isConflicted("Owner")).toBe(true),
    { timeout: 2000 });
    expect(disk.raws).toEqual(["owner", "rewritten elsewhere"]);
    expect(pageByName("Draft")!.roots.map((id) => doc.byId[id].raw)).toEqual(["a3"]);
  });
});

describe("alias owner replacement protects uncommitted input", () => {
  it("refuses an owner already held by an IME editor", async () => {
    loadFeed([draft(["draft"]), page("Owner", ["owner"], "owner-0")]);
    const host = await bindTestHost();
    const { disk, ownerSubmit } = ownerFile(host, ["owner"]);
    const ownerRoot = pageByName("Owner")!.roots[0];
    const getOwner = vi.mocked(backend().getPageByPath);
    startEditing(ownerRoot, 0, null); // IME value is still DOM-local; no setRaw.
    try {
      setRaw(pageByName("Draft")!.roots[0], "draft edited");
      await vi.waitFor(() => expect(getOwner).toHaveBeenCalled(), { timeout: 2000 });
      expect(doc.byId[ownerRoot]?.raw).toBe("owner");
    } finally { endEdit("page-navigation"); }
    // The anchor: once the owner's editor is gone, the next draft edit lands.
    // Its save is the owner's first, so the refused landing wrote nothing.
    expect(pageByName("Draft"), "the landing under the owner's editor must be refused, keeping the draft").toBeDefined();
    setRaw(pageByName("Draft")!.roots[0], "draft edited again");
    await vi.waitFor(() => expect(submittedPages(ownerSubmit, "Owner")).toHaveLength(1), { timeout: 2000 });
    expect(raws(submittedPages(ownerSubmit, "Owner")[0])).toEqual(["owner", "draft edited again"]);
    expect(disk.raws).toEqual(["owner", "draft edited again"]);
  });
});
