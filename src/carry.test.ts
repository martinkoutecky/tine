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
    const save = vi.spyOn(backend(), "savePages");
    const carrying = carryDay("2026-09-25");
    await vi.waitFor(() => expect(read).toHaveBeenCalledWith(journalTitle(new Date()), "journal"));
    resetStore();
    loadSingle({ name: "New graph", kind: "page", title: "New graph", pre_block: null, blocks: [] });
    finish({ name: journalTitle(new Date()), kind: "journal", title: "Today", id: "journals/old.md", pre_block: null, blocks: [] });
    await carrying;
    expect(doc.feed).toEqual(["New graph"]);
    expect(pageByName(journalTitle(new Date()))).toBeUndefined();
    expect(save).not.toHaveBeenCalled();
    read.mockRestore();
    save.mockRestore();
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
    loadSingle({ id: `pages/${today}.md`, rev: "s1", name: today, kind: "journal", title: today, pre_block: null,
      blocks: [{ id: "stray", raw: "stray text", collapsed: false, children: [] }] });
    setRaw(pageByName(today)!.roots[0], "stray edited");
    ensurePageLoaded({ id: "journals/source.md", rev: "r1", name: source, kind: "journal", title: source, pre_block: null,
      blocks: [{ id: "task", raw: "TODO carry me", collapsed: false, children: [] }] });
    vi.spyOn(backend(), "getPage").mockImplementation(async (name) => (name === today
      ? { id: "journals/canonical.md", rev: "c1", name: today, kind: "journal", title: today, pre_block: null, blocks: [] }
      : null) as never);
    const save = vi.spyOn(backend(), "savePages").mockResolvedValue({ ok: ["r2", "r3"] });

    await carryDay(source);

    const wrote = JSON.stringify(save.mock.calls);
    expect(wrote).not.toContain("TODO carry me");
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
    loadSingle({ id: `pages/${today}.md`, rev: "s1", name: today, kind: "journal", title: today, pre_block: null,
      blocks: [{ id: "stray", raw: "stray text", collapsed: false, children: [] }] });
    ensurePageLoaded({ id: "journals/source.md", rev: "r1", name: source, kind: "journal", title: source, pre_block: null,
      blocks: [{ id: "task", raw: "TODO carry me", collapsed: false, children: [] }] });
    vi.spyOn(backend(), "getPage").mockImplementation(async (name) => (name === today
      ? { id: "journals/canonical.md", rev: "c1", name: today, kind: "journal", title: today, pre_block: null,
          blocks: [{ id: "morning", raw: "morning", collapsed: false, children: [] }] }
      : null) as never);
    vi.spyOn(backend(), "journalFeedPage").mockResolvedValue({ pages: [], next_before_day: null, done: true, as_of_day: 0 } as never);
    const save = vi.spyOn(backend(), "savePages").mockImplementation(async (entries) => ({ ok: entries.map((_, i) => `saved-${i}`) }));

    await carryDay(source);

    const entries = save.mock.calls.flatMap(([batch]) => batch);
    const written = (path: string) => entries.filter((entry) => JSON.stringify(entry).includes(path));
    expect(written("journals/canonical.md").some((entry) => JSON.stringify(entry).includes("TODO carry me"))).toBe(true);
    expect(JSON.stringify(written(`pages/${today}.md`))).not.toContain("TODO carry me");
    const todayRaws = pageByName(today)!.roots.map((id) => doc.byId[id].raw);
    expect(todayRaws.filter((raw) => raw.includes("TODO carry me"))).toHaveLength(1);
    expect(todayRaws).toContain("morning");
    vi.restoreAllMocks();
  });
});
