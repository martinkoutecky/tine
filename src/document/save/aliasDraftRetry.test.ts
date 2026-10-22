import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../../backend";
import { loadFeed, pageByName, resetStore, setRaw, unsavedDrafts } from "../index";
import { doc } from "../model";
import { initParser } from "../../render/parse";
import { setToasts } from "../../toasts";
import { bindTestHost, mailPage, submittedPages, type TestHost } from "../host/wiring.test.support";
import type { BlockDto, PageDto, PageRead } from "../../types";

// Audit L9-05 repro; helpers copied from src/document/save/aliasDraft.test.ts.
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

describe("L9-05: alias draft landing whose owner settle returns false", () => {
  it("appends the draft to its owner only once when the first landing's publication is not proved", async () => {
    loadFeed([draft(["a"])]);
    const host = await bindTestHost();
    const [first] = pageByName("Draft")!.roots;
    const { disk, ownerSubmit } = ownerFile(host, ["owner"]);
    // The owner's first save is taken but its publication is not proved yet
    // (a save backoff on a disk error, REVIEW-3b-P2b F2): settle returns false.
    let ownerWaits = 0;
    vi.mocked(backend().pageWait).mockImplementation(async (_session, needs) => {
      if (needs.some((n) => n.key === OWNER)) return ++ownerWaits > 1;
      return true;
    });
    setRaw(first, "a1");
    await vi.waitFor(() => expect(submittedPages(ownerSubmit, "Owner")).toHaveLength(1), { timeout: 2000 });
    expect(raws(submittedPages(ownerSubmit, "Owner")[0])).toEqual(["owner", "a1"]);
    await new Promise((done) => setTimeout(done, 100));
    // The draft stays (its landing did not complete); the user edits it again.
    expect(pageByName("Draft")).toBeDefined();
    setRaw(pageByName("Draft")!.roots[0], "a2");
    await vi.waitFor(() => expect(submittedPages(ownerSubmit, "Owner").length).toBeGreaterThanOrEqual(2), { timeout: 2000 });
    await vi.waitFor(() => expect(pageByName("Draft")).toBeUndefined());
    // The owner must hold ONE copy of the draft (its current text), not two.
    expect(disk.raws).toEqual(["owner", "a2"]);
  });

  it("keeps a refused landing recoverable and retries with one current copy", async () => {
    loadFeed([draft(["a"])]);
    const host = await bindTestHost();
    const [first] = pageByName("Draft")!.roots;
    const { disk, ownerSubmit } = ownerFile(host, ["owner"]);
    const accept = ownerSubmit.getMockImplementation()!;
    let refusing = true;
    ownerSubmit.mockImplementation((...args) => refusing
      ? Promise.resolve({ reason: "failed", message: "EIO" }) : accept(...args));
    setRaw(first, "a1");
    await vi.waitFor(() => expect(ownerSubmit).toHaveBeenCalled());
    await new Promise((done) => setTimeout(done, 100));
    expect(disk.raws).toEqual(["owner"]);
    expect(doc.byId[first].raw).toBe("a1");
    expect(unsavedDrafts().some((entry) => entry.name === "Draft")).toBe(true);
    refusing = false;
    setRaw(first, "a2");
    await vi.waitFor(() => expect(pageByName("Draft")).toBeUndefined(), { timeout: 2000 });
    expect(disk.raws).toEqual(["owner", "a2"]);
  });
});
