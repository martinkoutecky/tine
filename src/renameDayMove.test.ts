import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { renameOrMergePage } from "./graph";
import { ensurePageLoaded, flushAll, loadFeed, pageByName, resetStore, setPageProperty } from "./document";
import { bindTestHost } from "./document/host/wiring.test.support";
import { installDiskHost } from "./diskHost.test.support";
import { initParser } from "./render/parse";
import { setGraphMeta } from "./graphSession";
import { renderEnglish } from "./i18n";
import { renameHelperRequest } from "./renameHelperDialogs";
import { setToasts, toasts } from "./toasts";
import type { BlockDto, ContestedOffer, DayMoveOffer, GraphMeta, PageDto, PageRead, RenameOffer } from "./types";

// The day move and the contested-name helper (D-20261111-CMDF, -MA, -MOA,
// D-20261019-M2(6), D-20261115-HELP), driven through the rename the window
// performs (`renameOrMergePage`), with the page host over a fake disk:
// assertions read the files, what the user would find on disk.

const DAY = "Nov 18th, 2026";
const DAY_FILE = "journals/2026_11_18.md";
let serial = 0;
const block = (raw: string): BlockDto => ({ id: `dm-${++serial}`, raw, collapsed: false, children: [] });
const page = (name: string, path: string, blocks: BlockDto[], pre_block: string | null = null): PageRead => ({
  name, title: name, kind: path.startsWith("journals/") ? "journal" : "page", id: path, rev: `initial-${name}`, pre_block, blocks,
  format: path.endsWith(".org") ? "org" : "md",
});

let files: Map<string, PageDto>;
let writes: string[];
let saveFails = false;
async function bindDisk(pages: PageRead[]) {
  files = new Map(pages.map((p) => [p.id, structuredClone(p)]));
  writes = [];
  saveFails = false;
  let n = 0;
  return installDiskHost(await bindTestHost(), {
    read: (key) => files.get(key) ?? null,
    write: (key, dto) => {
      if (saveFails) throw new Error("disk full");
      writes.push(key);
      const rev = `w${++n}`;
      files.set(key, { ...structuredClone(dto), rev });
      return rev;
    },
  });
}
const saved = (path: string) => files.get(path)?.blocks.map((b) => b.raw) ?? null;
const header = (path: string) => files.get(path)?.pre_block ?? null;
/** The file's text as the user reads it: its preamble, then every block's text in outline order. */
const text = (path: string): string => {
  const raws = (blocks: BlockDto[]): string[] => blocks.flatMap((b) => [b.raw, ...raws(b.children)]);
  const file = files.get(path);
  return file ? [file.pre_block ?? "", ...raws(file.blocks)].join("\n") : "";
};
const errors = () => toasts().filter((t) => t.kind === "error").map((t) => renderEnglish(t.text));
const dayPage = (pre: string | null = "alias:: Launch day") => page(DAY, DAY_FILE, [block("day note"), block("TODO call")], pre);
const dayMove = (over: Partial<DayMoveOffer> = {}): DayMoveOffer => ({
  kind: "dayMove", reason: { kind: "day", name: DAY }, message: "",
  day: { name: DAY, path: DAY_FILE, journal: true, aliases: ["Launch day"] },
  name: "Trip", journal: false, into: null, path: "pages/Trip.md", references: ["diary"], referenceCount: 1, release: null, retargeted: null,
  ...over,
});
const offer = (...values: RenameOffer[]) => {
  const spy = vi.spyOn(backend(), "renameOffer");
  for (const value of values) spy.mockResolvedValueOnce(value);
  return spy;
};
/** Files that do not exist yet: the name is free and its file would be `pages/<name>.<ext>`. */
const absent = (ext = "md") => {
  vi.spyOn(backend(), "getPage").mockImplementation(async (name) => {
    const found = [...files.entries()].find(([, p]) => p.name === name);
    return found ? { ...structuredClone(found[1]), id: found[0] } : null;
  });
  vi.spyOn(backend(), "resolvePage").mockImplementation(async (name) => {
    const found = [...files.entries()].find(([, p]) => p.name === name);
    return found ? { kind: "existing", id: found[0], others: [] } : { kind: "absent", id: `pages/${name}.${ext}` };
  });
};
const answer = async (value: true | string | null) => {
  await vi.waitFor(() => expect(renameHelperRequest()).not.toBeNull());
  const request = renameHelperRequest()!;
  (request.finish as (answer: typeof value) => void)(value);
};
const target = { name: DAY, pageKind: "journal" as const, path: DAY_FILE };

beforeAll(() => initParser());
beforeEach(() => {
  serial = 0;
  resetStore();
  setToasts([]);
  setGraphMeta({ root: "/g", pages_dir: "pages", journals_dir: "journals", assets_dir: "assets", preferred_format: "md" } as unknown as GraphMeta);
});
afterEach(() => { vi.restoreAllMocks(); resetStore(); setGraphMeta(null); setToasts([]); });

describe("renaming a day moves its notes to a page", () => {
  it("to a new page: the page holds the notes, the day keeps its header and one Moved to block", async () => {
    const day = dayPage();
    loadFeed([day]);
    await bindDisk([day]);
    absent();
    offer(dayMove());
    const done = renameOrMergePage(DAY, "Trip", target);
    await answer(true);
    expect(await done).toEqual({ moved: "Trip", kind: "page" });
    expect(await flushAll()).toBe(true);
    expect(saved("pages/Trip.md")).toEqual(["day note", "TODO call"]);
    expect(saved(DAY_FILE)).toEqual(["Moved to [[Trip]]"]);
    expect(header(DAY_FILE)).toBe("alias:: Launch day");
    expect(errors()).toEqual([]);
  });

  it("into an existing page: the notes follow that page's own blocks", async () => {
    const day = dayPage(null), trip = page("Trip", "pages/Trip.md", [block("trip plan")]);
    loadFeed([day, trip]);
    await bindDisk([day, trip]);
    absent();
    offer(dayMove({ into: { name: "Trip", path: "pages/Trip.md", journal: false, aliases: [] }, day: { name: DAY, path: DAY_FILE, journal: true, aliases: [] } }));
    const done = renameOrMergePage(DAY, "trip", target);
    await answer(true);
    expect(await done).toEqual({ moved: "Trip", kind: "page" });
    expect(await flushAll()).toBe(true);
    expect(saved("pages/Trip.md")).toEqual(["trip plan", "day note", "TODO call"]);
    expect(saved(DAY_FILE)).toEqual(["Moved to [[Trip]]"]);
  });

  it("cancelling writes nothing", async () => {
    const day = dayPage();
    loadFeed([day]);
    await bindDisk([day]);
    absent();
    offer(dayMove());
    const done = renameOrMergePage(DAY, "Trip", target);
    await answer(null);
    expect(await done).toBe("cancelled");
    expect(await flushAll()).toBe(true);
    expect(writes).toEqual([]);
  });

  it("a move the host refuses leaves both pages as they were and says so", async () => {
    const day = dayPage();
    loadFeed([day]);
    const spies = await bindDisk([day]);
    absent();
    spies.move.mockResolvedValueOnce({ reason: "failed", message: "host stopped" });
    offer(dayMove());
    const done = renameOrMergePage(DAY, "Trip", target);
    await answer(true);
    expect(await done).toBe("cancelled");
    expect(await flushAll()).toBe(true);
    expect(saved(DAY_FILE)).toEqual(["day note", "TODO call"]);
    expect(saved("pages/Trip.md")).toBeNull();
    expect(errors().length).toBeGreaterThan(0);
    expect(toasts().filter((t) => renderEnglish(t.text).startsWith("Moved the notes"))).toEqual([]);
  });

  it("to the day's own alias: the alias is released for the move and is back on the day after it", async () => {
    const day = dayPage("alias:: Launch day");
    loadFeed([day]);
    await bindDisk([day]);
    absent();
    const release = { page: DAY, path: DAY_FILE, journal: true, key: "alias", before: "Launch day", after: null };
    offer(dayMove({ name: "Launch day", release }));
    const done = renameOrMergePage(DAY, "Launch day", target);
    await answer(true);
    expect(await done).toEqual({ moved: "Launch day", kind: "page" });
    expect(await flushAll()).toBe(true);
    expect(saved("pages/Launch day.md")).toEqual(["day note", "TODO call"]);
    expect(saved(DAY_FILE)).toEqual(["Moved to [[Launch day]]"]);
    expect(header(DAY_FILE)).toBe("alias:: Launch day");
  });

  it("to the day's own alias, a refused move puts the alias line back", async () => {
    const day = dayPage("alias:: Launch day");
    loadFeed([day]);
    const spies = await bindDisk([day]);
    absent();
    spies.move.mockResolvedValueOnce({ reason: "failed", message: "host stopped" });
    const release = { page: DAY, path: DAY_FILE, journal: true, key: "alias", before: "Launch day", after: null };
    offer(dayMove({ name: "Launch day", release }));
    const done = renameOrMergePage(DAY, "Launch day", target);
    await answer(true);
    expect(await done).toBe("cancelled");
    expect(await flushAll()).toBe(true);
    expect(header(DAY_FILE)).toBe("alias:: Launch day");
    expect(saved(DAY_FILE)).toEqual(["day note", "TODO call"]);
    expect(saved("pages/Launch day.md")).toBeNull();
  });

  /** Load `count` unrelated pages, as browsing does: more than the working
   *  set holds (80), so unpinned pages are evicted. */
  const browse = (count: number) => {
    for (let i = 0; i < count; i++) {
      ensurePageLoaded(page(`Filler ${i}`, `pages/Filler ${i}.md`, [block("filler")]));
    }
  };
  const release = { page: DAY, path: DAY_FILE, journal: true, key: "alias", before: "Launch day", after: null };

  it("to the day's own alias, loading the destination page cannot evict the day under its helper edit", async () => {
    const day = dayPage("alias:: Launch day");
    loadFeed([]);
    await bindDisk([day]);
    absent();
    const read = backend().getPage.bind(backend());
    vi.spyOn(backend(), "getPage").mockImplementation(async (name, kind) => { if (name === "Launch day") browse(90); return read(name, kind); });
    offer(dayMove({ name: "Launch day", release }));
    const done = renameOrMergePage(DAY, "Launch day", target);
    await answer(true);
    expect(await done).toEqual({ moved: "Launch day", kind: "page" });
    expect(await flushAll()).toBe(true);
    expect(saved("pages/Launch day.md")).toEqual(["day note", "TODO call"]);
    expect(saved(DAY_FILE)).toEqual(["Moved to [[Launch day]]"]);
    expect(header(DAY_FILE)).toBe("alias:: Launch day");
    expect(errors()).toEqual([]);
  });

  it.each([
    ["a move", () => {}],
    ["a refused move", (spies: Awaited<ReturnType<typeof bindDisk>>) => spies.move.mockResolvedValueOnce({ reason: "failed", message: "host stopped" })],
  ] as const)("after %s to the day's own alias the day can be evicted again", async (_, arrange) => {
    const day = dayPage("alias:: Launch day");
    loadFeed([]);
    const spies = await bindDisk([day]);
    absent();
    arrange(spies);
    offer(dayMove({ name: "Launch day", release }));
    const done = renameOrMergePage(DAY, "Launch day", target);
    await answer(true);
    await done;
    expect(await flushAll()).toBe(true);
    expect(pageByName(DAY)).toBeTruthy();
    browse(90);
    expect(pageByName(DAY)).toBeFalsy();
  });

  it("to the day's own alias, a failed alias save that cannot be put back says the line was not restored", async () => {
    const day = dayPage("alias:: Launch day");
    loadFeed([day]);
    await bindDisk([day]);
    absent();
    saveFails = true;
    offer(dayMove({ name: "Launch day", release }));
    const done = renameOrMergePage(DAY, "Launch day", target);
    await answer(true);
    expect(await done).toBe("cancelled");
    expect(errors().some((text) => text.includes("could not save the restored text"))).toBe(true);
    expect(errors().some((text) => text.includes("so it was put back"))).toBe(false);
  });

  // Round 2 (merge review d326456c6). Finding 1: an unrelated property typed
  // while the destination loads survives; only the released alias comes back.
  it.each([
    ["a Markdown preamble", () => dayPage("alias:: Launch day\nmood:: calm")],
    ["a first-root property header", () => page(DAY, DAY_FILE, [block("alias:: Launch day\nmood:: calm"), block("day note"), block("TODO call")])],
  ] as const)("to the day's own alias with %s, a property edited during the move survives", async (_, make) => {
    const day = make();
    loadFeed([day]);
    await bindDisk([day]);
    absent();
    const read = backend().getPage.bind(backend());
    vi.spyOn(backend(), "getPage").mockImplementation(async (name, kind) => {
      if (name === "Launch day") setPageProperty(DAY, "mood", "happy");
      return read(name, kind);
    });
    offer(dayMove({ name: "Launch day", path: "pages/Launch day.md", release }));
    const done = renameOrMergePage(DAY, "Launch day", target);
    await answer(true);
    expect(await done).toEqual({ moved: "Launch day", kind: "page" });
    expect(await flushAll()).toBe(true);
    expect(saved("pages/Launch day.md")).toEqual(["day note", "TODO call"]);
    const dayText = text(DAY_FILE);
    expect(dayText).toContain("mood:: happy");
    expect(dayText).toContain("alias:: Launch day");
    expect(dayText).not.toContain("mood:: calm");
    expect(dayText).toContain("Moved to [[Launch day]]");
    expect(dayText).not.toContain("day note");
  });

  // Finding 2: the file that receives the notes is the one the user confirmed.
  it.each([
    ["a new page became an existing one", dayMove(),
      () => files.set("pages/Trip.md", page("Trip", "pages/Trip.md", [block("trip plan")])), "pages/Trip.md", ["trip plan"]],
    ["another file now claims the name", dayMove({ into: { name: "Trip", path: "pages/Trip.md", journal: false, aliases: [] } }),
      () => { files.delete("pages/Trip.md"); files.set("pages/trip.md", page("Trip", "pages/trip.md", [block("other trip")])); }, "pages/trip.md", ["other trip"]],
  ] as const)("when %s while the dialog was open, the move is asked again", async (_, first, change, file, kept) => {
    const day = dayPage(), trip = page("Trip", "pages/Trip.md", [block("trip plan")]);
    loadFeed([day]);
    await bindDisk(first.into ? [day, trip] : [day]);
    absent();
    const offers = offer(first, dayMove({ into: { name: "Trip", path: file, journal: false, aliases: [] }, path: file }));
    const done = renameOrMergePage(DAY, "Trip", target);
    await vi.waitFor(() => expect(renameHelperRequest()).not.toBeNull());
    change();
    await answer(true);
    await vi.waitFor(() => expect(offers).toHaveBeenCalledTimes(2));
    await answer(null);
    expect(await done).toBe("cancelled");
    expect(await flushAll()).toBe(true);
    expect(saved(file)).toEqual(kept);
    expect(saved(DAY_FILE)).toEqual(["day note", "TODO call"]);
  });

  it("a page the first move created receives a second day's notes (its file is the confirmed one)", async () => {
    const day = dayPage(null), next = page("Nov 19th, 2026", "journals/2026_11_19.md", [block("third note")]);
    loadFeed([day, next]);
    await bindDisk([day, next]);
    absent();
    offer(dayMove());
    const first = renameOrMergePage(DAY, "Trip", target);
    await answer(true);
    expect(await first).toEqual({ moved: "Trip", kind: "page" });
    expect(await flushAll()).toBe(true);
    const offers = offer(dayMove({ day: { name: "Nov 19th, 2026", path: "journals/2026_11_19.md", journal: true, aliases: [] },
      into: { name: "Trip", path: "pages/Trip.md", journal: false, aliases: [] }, path: "pages/Trip.md" }));
    const second = renameOrMergePage("Nov 19th, 2026", "Trip", { name: "Nov 19th, 2026", pageKind: "journal", path: "journals/2026_11_19.md" });
    await answer(true);
    expect(await second).toEqual({ moved: "Trip", kind: "page" });
    expect(offers).toHaveBeenCalledTimes(1);
    expect(await flushAll()).toBe(true);
    expect(saved("pages/Trip.md")).toEqual(["day note", "TODO call", "third note"]);
  });

  // Finding 3: every format pair, to a new page and into an existing one.
  it.each([
    ["md", "md", false], ["md", "md", true], ["org", "org", false], ["org", "org", true],
    ["md", "org", false], ["md", "org", true], ["org", "md", false], ["org", "md", true],
  ] as const)("a %s day to a %s page (existing: %s) moves only when the formats match", async (from, to, existing) => {
    const dayFile = `journals/2026_11_18.${from}`, dest = `pages/Trip.${to}`;
    const day = page(DAY, dayFile, [block("day note"), block("TODO call")]);
    const trip = page("Trip", dest, [block("trip plan")]);
    loadFeed([day]);
    await bindDisk(existing ? [day, trip] : [day]);
    absent(to);
    offer(dayMove({
      day: { name: DAY, path: dayFile, journal: true, aliases: [] },
      into: existing ? { name: "Trip", path: dest, journal: false, aliases: [] } : null, path: dest,
    }));
    const done = renameOrMergePage(DAY, "Trip", { ...target, path: dayFile });
    await answer(true);
    const before = existing ? ["trip plan"] : [];
    if (from === to) {
      expect(await done).toEqual({ moved: "Trip", kind: "page" });
      expect(await flushAll()).toBe(true);
      expect(saved(dest)).toEqual([...before, "day note", "TODO call"]);
      expect(saved(dayFile)).toEqual(["Moved to [[Trip]]"]);
      expect(errors()).toEqual([]);
    } else {
      expect(await done).toBe("cancelled");
      expect(await flushAll()).toBe(true);
      expect(saved(dest)).toEqual(existing ? before : null);
      expect(saved(dayFile)).toEqual(["day note", "TODO call"]);
      expect(errors().some((t) => t.includes("use different formats"))).toBe(true);
    }
  });

  // Finding 4: notes nested under the page-header block move too; the header stays.
  it.each([
    ["only under the header", [] as string[]],
    ["under the header and after it", ["day note"]],
  ] as const)("notes %s move to the page; the header stays on the day", async (_, after) => {
    const head: BlockDto = { ...block("mood:: calm"), children: [block("nested note")] };
    const day = page(DAY, DAY_FILE, [head, ...after.map(block)]);
    loadFeed([day]);
    await bindDisk([day]);
    absent();
    offer(dayMove());
    const done = renameOrMergePage(DAY, "Trip", target);
    await answer(true);
    expect(await done).toEqual({ moved: "Trip", kind: "page" });
    expect(await flushAll()).toBe(true);
    expect(saved("pages/Trip.md")).toEqual(["nested note", ...after]);
    const dayText = text(DAY_FILE);
    expect(dayText).toContain("mood:: calm");
    expect(dayText).toContain("Moved to [[Trip]]");
    expect(dayText).not.toContain("nested note");
  });

  // Follow-up (finding 6): the move's operations, counted (PC-3). The lower
  // bound: one read of a destination not loaded, one host move of the two
  // pages, each file written once; the own-alias case adds the alias save.
  // One name resolution checks the receiving file (finding 2).
  it.each([
    ["to a new page", dayMove(), 0, "pages/Trip.md"],
    ["to the day's own alias", dayMove({ name: "Launch day", path: "pages/Launch day.md", release }), 1, "pages/Launch day.md"],
  ] as const)("a move %s costs one destination read, one host move and one write per file", async (_, made, aliasSaves, dest) => {
    const day = dayPage();
    loadFeed([day]);
    const spies = await bindDisk([day]);
    absent();
    const reads = vi.mocked(backend().getPage);
    offer(made);
    const done = renameOrMergePage(DAY, made.name, target);
    await answer(true);
    expect(await done).toEqual({ moved: made.name, kind: "page" });
    expect(await flushAll()).toBe(true);
    expect(reads).toHaveBeenCalledTimes(1);
    // The helper's check, and the page host opening the page that has no file yet.
    expect(backend().resolvePage).toHaveBeenCalledTimes(2);
    expect(spies.move).toHaveBeenCalledTimes(1);
    expect(spies.submit).toHaveBeenCalledTimes(aliasSaves);
    expect(writes.filter((key) => key === dest)).toHaveLength(1);
    expect(writes.filter((key) => key === DAY_FILE)).toHaveLength(1 + aliasSaves);
  });

  it("a day with nothing to move says so and writes nothing", async () => {
    const day = page(DAY, DAY_FILE, [block("")]);
    loadFeed([day]);
    await bindDisk([day]);
    absent();
    offer(dayMove());
    expect(await renameOrMergePage(DAY, "Trip", target)).toBe("cancelled");
    expect(renameHelperRequest()).toBeNull();
    expect(writes).toEqual([]);
  });
});

describe("a name several files claim", () => {
  const contested: ContestedOffer = {
    kind: "contested", reason: { kind: "contested", name: "Twin", files: ["pages/Twin.md", "pages/twin copy.md"] },
    message: "", name: "Twin", files: ["pages/Twin.md", "pages/twin copy.md"], opens: "pages/Twin.md",
  };

  it("keeping one file merges the others into it, then the rename is asked again", async () => {
    const offers = offer(contested, { kind: "rename" });
    const merge = vi.spyOn(backend(), "mergePages").mockResolvedValue(undefined as never);
    const rename = vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "renamed", renamed: [{ from: "Src", to: "Twin", path: "pages/Src.md" }], touched: [] });
    vi.spyOn(backend(), "confirm").mockResolvedValue(true);
    const done = renameOrMergePage("Src", "Twin", { name: "Src", pageKind: "page", path: "pages/Src.md" });
    await answer("pages/twin copy.md");
    expect(await done).toBe("renamed");
    expect(merge).toHaveBeenCalledExactlyOnceWith("pages/Twin.md", "pages/twin copy.md", ["insert-blocks", "delete-page"], undefined);
    expect(offers).toHaveBeenCalledTimes(2);
    expect(rename).toHaveBeenCalledOnce();
  });

  it("only a listed file can be kept; cancelling merges nothing", async () => {
    offer(contested);
    const merge = vi.spyOn(backend(), "mergePages");
    const done = renameOrMergePage("Src", "Twin");
    await answer("pages/elsewhere.md");
    expect(renameHelperRequest()).not.toBeNull();
    renameHelperRequest()!.finish(null);
    expect(await done).toBe("cancelled");
    expect(merge).not.toHaveBeenCalled();
  });
});
