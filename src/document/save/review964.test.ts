import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { backend } from "../../backend";
import { startEditing } from "../../editorController";
import { flushPage, liveConflictDraft, loadFeed, resetStore, setRaw } from "../index";
import { pageToDto } from "../convert";
import { pageInstanceGeneration } from "../instance";
import { acceptHeldPush, baseRevFor } from "../host/wiring";
import { answerOpensFromDocument } from "../host/documentHost.test.support";
import { bindTestHost, mailPage } from "../host/wiring.test.support";

const KEY = "pages/Note.md";
type Proof = { key: string; version: number; rev: string | null }[];

beforeEach(() => resetStore());
afterEach(() => { resetStore(); vi.restoreAllMocks(); });

async function submissions(count: number) {
  loadFeed([{ id: KEY, name: "Note", title: "Note", kind: "page", pre_block: null, rev: "disk-0",
    blocks: [{ id: "body", raw: "original", collapsed: false, children: [] }] }]);
  const host = await bindTestHost();
  answerOpensFromDocument(host);
  const submit = vi.spyOn(backend(), "pageSubmit").mockResolvedValue(null);
  const waits: { version: number; finish: (proof: Proof) => void }[] = [];
  const results = new Map<number, Proof>();
  vi.spyOn(backend(), "pageWait").mockImplementation((_session, needs) => {
    const version = needs[0].version;
    return (results.has(version) ? Promise.resolve(results.get(version)!)
      : new Promise<Proof>(finish => waits.push({ version, finish })));
  });
  const flushes: Promise<boolean>[] = [];
  startEditing("body");
  for (let i = 0; i < count; i++) {
    setRaw("body", `typed ${i + 1}`);
    flushes.push(flushPage("Note"));
    await vi.waitFor(() => expect(submit).toHaveBeenCalledTimes(i + 1));
    host.deliver({ key: KEY, answer: { id: submit.mock.calls[i][1], version: i + 4, took: true,
      outcome: { kind: "applied" } }, page: { version: i + 4, conflict: false, risk: false,
      disk: { kind: "file", rev: "disk-0" }, text: { kind: "unchanged", rev: `disk-${i + 1}` } } });
    await vi.waitFor(() => expect(waits.some(wait => wait.version === i + 4)).toBe(true));
  }
  return { host, flushes, finish(version: number, publishedVersion: number) {
    const proof = [{ key: KEY, version: publishedVersion, rev: `disk-${publishedVersion - 3}` }];
    results.set(version, proof);
    for (const wait of waits.filter(wait => wait.version === version)) wait.finish(proof);
  } };
}

it.each([2, 3].flatMap(count => [true, false].map(olderFirst => [count, olderFirst] as const)))(
  "coalescing %i submissions uses the witnessed ancestor (older reply first=%s)", async (count, olderFirst) => {
    const { host, flushes, finish } = await submissions(count);
    const latest = count + 3;
    const first = olderFirst ? 4 : latest;
    finish(first, latest);
    await vi.waitFor(() => expect(baseRevFor("Note")).toBe(`disk-${count}`));
    host.notice(KEY, { conflictReported: true }, { version: latest, conflict: true, risk: true,
      disk: { kind: "file", rev: "cli-disk" } });
    expect(liveConflictDraft("Note")?.page.blocks[0].raw).toBe(`typed ${count}`);
    expect(liveConflictDraft("Note")?.baseRev).toBe(`disk-${count}`);
    for (let version = 4; version <= latest; version++) finish(version, latest);
    await Promise.all(flushes);
    expect(liveConflictDraft("Note")?.baseRev).toBe(`disk-${count}`);
  });

it.each([true, false].flatMap(identical => [4, 5].map(delayed => [identical, delayed] as const)))(
  "installed disk content (identical=%s) survives delayed v%i evidence", async (identical, delayed) => {
    const { host, flushes, finish } = await submissions(2);
    const instance = pageInstanceGeneration("Note");
    const installed = pageToDto("Note")!;
    if (!identical) installed.blocks[0].raw = "external text";
    installed.rev = "external-rev";
    host.deliver({ key: KEY, answer: null, page: mailPage(6, installed) });
    acceptHeldPush("Note");
    expect(baseRevFor("Note")).toBe("external-rev");
    expect(pageInstanceGeneration("Note") === instance).toBe(identical);
    finish(delayed, delayed);
    for (let i = 0; i < 16; i++) await Promise.resolve();
    expect(baseRevFor("Note")).toBe("external-rev");
    finish(delayed === 4 ? 5 : 4, delayed === 4 ? 5 : 4);
    await Promise.all(flushes);
    const block = pageToDto("Note")!.blocks[0].id;
    startEditing(block);
    setRaw(block, "typed after reload");
    host.notice(KEY, { conflictReported: true }, { version: 6, conflict: true, risk: true,
      disk: { kind: "file", rev: "cli-disk" } });
    expect(liveConflictDraft("Note")?.page.blocks[0].raw).toBe("typed after reload");
    expect(liveConflictDraft("Note")?.baseRev).toBe("external-rev");
  });
