import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { renameOrMergePage } from "./graph";
import { flushAll, loadFeed, resetStore } from "./document";
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
});

let files: Map<string, PageDto>;
let writes: string[];
async function bindDisk(pages: PageRead[]) {
  files = new Map(pages.map((p) => [p.id, structuredClone(p)]));
  writes = [];
  let n = 0;
  return installDiskHost(await bindTestHost(), {
    read: (key) => files.get(key) ?? null,
    write: (key, dto) => {
      writes.push(key);
      const rev = `w${++n}`;
      files.set(key, { ...structuredClone(dto), rev });
      return rev;
    },
  });
}
const saved = (path: string) => files.get(path)?.blocks.map((b) => b.raw) ?? null;
const header = (path: string) => files.get(path)?.pre_block ?? null;
const errors = () => toasts().filter((t) => t.kind === "error").map((t) => renderEnglish(t.text));
const dayPage = (pre: string | null = "alias:: Launch day") => page(DAY, DAY_FILE, [block("day note"), block("TODO call")], pre);
const dayMove = (over: Partial<DayMoveOffer> = {}): DayMoveOffer => ({
  kind: "dayMove", reason: { kind: "day", name: DAY }, message: "",
  day: { name: DAY, path: DAY_FILE, journal: true, aliases: ["Launch day"] },
  name: "Trip", journal: false, into: null, references: ["diary"], referenceCount: 1, release: null, retargeted: null,
  ...over,
});
const offer = (...values: RenameOffer[]) => {
  const spy = vi.spyOn(backend(), "renameOffer");
  for (const value of values) spy.mockResolvedValueOnce(value);
  return spy;
};
/** Files that do not exist yet: the name is free and its file would be `pages/<name>.md`. */
const absent = () => {
  vi.spyOn(backend(), "getPage").mockImplementation(async (name) => {
    const found = [...files.entries()].find(([, p]) => p.name === name);
    return found ? { ...structuredClone(found[1]), id: found[0] } : null;
  });
  vi.spyOn(backend(), "resolvePage").mockImplementation(async (name) => ({ kind: "absent", id: `pages/${name}.md` }));
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
