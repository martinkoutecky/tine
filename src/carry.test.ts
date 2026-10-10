import { describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { carryDay } from "./carry";
import { journalTitle } from "./journal";
import { ensurePageLoaded, pageByName, resetStore, setRaw } from "./document";
import { initParser } from "./render/parse";
import { loadSingle } from "./document/workingSet";
import { doc } from "./document/model";
import type { PageRead } from "./types";
import { setToasts, toasts } from "./toasts";
import { flushAll } from "./document/host/wiring";
import { bindTestHost } from "./document/host/wiring.test.support";
import { installDiskHost } from "./diskHost.test.support";
import { expectNoPageWrites, spyPageWrites } from "./pageWrites.test.support";
import type { PageDto } from "./types";

/** A fake disk holding `files` (path → page); every write is recorded as [path, page]. */
function diskOf(files: Record<string, PageDto>) {
  const writes: [string, PageDto][] = [];
  let rev = 0;
  return {
    writes,
    disk: {
      read: (key: string) => files[key] ?? null,
      write: (key: string, dto: PageDto) => {
        writes.push([key, dto]);
        files[key] = { ...dto, id: key, rev: `w${++rev}` } as PageDto;
        return `w${rev}`;
      },
    },
  };
}

describe("carry binding", () => {
  it("reports a source-day read failure through the carry action", async () => {
    resetStore();
    setToasts([]);
    loadSingle({ name: journalTitle(new Date()), kind: "journal", title: "Today", pre_block: null, blocks: [] });
    // Today's own read (the carry destination check) succeeds; only the source day fails.
    const read = vi.spyOn(backend(), "getPage").mockImplementation(async (name) => {
      if (name === journalTitle(new Date())) return null;
      throw new Error("source unreadable");
    });
    await expect(carryDay("Sep 25th, 2026")).resolves.toBeUndefined();
    expect(toasts().some((toast) => toast.kind === "error" && toast.message.includes("source unreadable"))).toBe(true);
    read.mockRestore();
  });
  it("does not load or write an old day when today's read finishes after a graph switch (I-20)", async () => {
    resetStore();
    let finish!: (page: PageRead | null) => void;
    const read = vi.spyOn(backend(), "getPage").mockImplementationOnce(() =>
      new Promise((resolve) => { finish = resolve; })
    );
    const carrying = carryDay("2026-09-25");
    await vi.waitFor(() => expect(read).toHaveBeenCalledWith(journalTitle(new Date()), "journal"));
    resetStore();
    loadSingle({ name: "New graph", kind: "page", title: "New graph", pre_block: null, blocks: [] });
    // The new graph's window is bound, so any write the old carry started would reach the host.
    await bindTestHost();
    const writes = spyPageWrites();
    finish({ name: journalTitle(new Date()), kind: "journal", title: "Today", id: "journals/old.md", pre_block: null, blocks: [] });
    await carrying;
    expect(doc.feed).toEqual(["New graph"]);
    expect(pageByName(journalTitle(new Date()))).toBeUndefined();
    expect(await flushAll()).toBe(true);
    expectNoPageWrites(writes);
    vi.restoreAllMocks();
  });

  // og I1e/J1 (GH #254 family, master 7bd793bd0): today's name slot can be held
  // by a second file for the same day (a duplicate day left by sync delivery or
  // a journal date-format change, opened path-pinned). Carry used to move the
  // tasks into that file with a success toast while the canonical journal the
  // feed shows for today never received them. With unsaved input in it, carry
  // refuses (replacing it would discard that input).
  it("refuses to carry while a second file holding today's name has unsaved input, and says so", async () => {
    await initParser();
    resetStore();
    setToasts([]);
    const today = journalTitle(new Date());
    const y = new Date();
    y.setDate(y.getDate() - 1);
    const source = journalTitle(y);
    const stray: PageRead = { id: `pages/${today}.md`, rev: "s1", name: today, kind: "journal", title: today, pre_block: null,
      blocks: [{ id: "stray", raw: "stray text", collapsed: false, children: [] }] };
    const sourceDto: PageRead = { id: "journals/source.md", rev: "r1", name: source, kind: "journal", title: source, pre_block: null,
      blocks: [{ id: "task", raw: "TODO carry me", collapsed: false, children: [] }] };
    const canonical: PageRead = { id: "journals/canonical.md", rev: "c1", name: today, kind: "journal", title: today, pre_block: null, blocks: [] };
    const { disk, writes } = diskOf({ [stray.id]: structuredClone(stray), [sourceDto.id]: structuredClone(sourceDto), [canonical.id]: structuredClone(canonical) });
    loadSingle(stray);
    ensurePageLoaded(sourceDto);
    installDiskHost(await bindTestHost(), disk);
    setRaw(pageByName(today)!.roots[0], "stray edited");
    vi.spyOn(backend(), "getPage").mockImplementation(async (name) => (name === today ? structuredClone(canonical) : null) as never);

    await carryDay(source);
    expect(await flushAll()).toBe(true);

    // The unsaved input itself is written, to its own file; the tasks reach no file.
    expect(writes.some(([path, dto]) => path === `pages/${today}.md` && JSON.stringify(dto).includes("stray edited"))).toBe(true);
    expect(JSON.stringify(writes)).not.toContain("TODO carry me");
    expect(pageByName(source)!.roots.map((id) => doc.byId[id].raw)).toEqual(["TODO carry me"]);
    expect(toasts().some((toast) => toast.kind === "error" && toast.message.includes(`pages/${today}.md`))).toBe(true);
    expect(pageByName(today)!.id).toBe(`pages/${today}.md`);
    expect(pageByName(today)!.roots.map((id) => doc.byId[id].raw)).toEqual(["stray edited"]);
    vi.restoreAllMocks();
  });

  // og J1 (manager decision): one rule for the family — a second file holding
  // today's name with NO unsaved input is replaced by today's real file, and the
  // tasks land there, once, and not in the second file.
  it("carries into today's real file when a second file holding its name has no unsaved input", async () => {
    await initParser();
    resetStore();
    setToasts([]);
    const today = journalTitle(new Date());
    const y = new Date();
    y.setDate(y.getDate() - 1);
    const source = journalTitle(y);
    const stray: PageRead = { id: `pages/${today}.md`, rev: "s1", name: today, kind: "journal", title: today, pre_block: null,
      blocks: [{ id: "stray", raw: "stray text", collapsed: false, children: [] }] };
    const sourceDto: PageRead = { id: "journals/source.md", rev: "r1", name: source, kind: "journal", title: source, pre_block: null,
      blocks: [{ id: "task", raw: "TODO carry me", collapsed: false, children: [] }] };
    const canonical: PageRead = { id: "journals/canonical.md", rev: "c1", name: today, kind: "journal", title: today, pre_block: null,
      blocks: [{ id: "morning", raw: "morning", collapsed: false, children: [] }] };
    const { disk, writes } = diskOf({ [stray.id]: structuredClone(stray), [sourceDto.id]: structuredClone(sourceDto), [canonical.id]: structuredClone(canonical) });
    loadSingle(stray);
    ensurePageLoaded(sourceDto);
    installDiskHost(await bindTestHost(), disk);
    vi.spyOn(backend(), "getPage").mockImplementation(async (name) => (name === today ? structuredClone(canonical) : null) as never);
    vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({ pages: [], next_before_day: null, done: true, as_of_day: 0 } as never);

    await carryDay(source);
    expect(await flushAll()).toBe(true);
    const written = (path: string) => writes.filter(([at]) => at === path).map(([, dto]) => dto);
    expect(written("journals/canonical.md").some((dto) => JSON.stringify(dto).includes("TODO carry me"))).toBe(true);
    expect(JSON.stringify(written(`pages/${today}.md`))).not.toContain("TODO carry me");
    // The source file no longer holds the carried task.
    expect(JSON.stringify(written("journals/source.md").at(-1))).not.toContain("TODO carry me");
    const todayRaws = pageByName(today)!.roots.map((id) => doc.byId[id].raw);
    expect(todayRaws.filter((raw) => raw.includes("TODO carry me"))).toHaveLength(1);
    expect(todayRaws).toContain("morning");
    vi.restoreAllMocks();
  });
});
