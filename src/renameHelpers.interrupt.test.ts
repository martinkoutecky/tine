import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { renameOrMergePage } from "./graph";
import { flushAll, loadFeed, resetStore } from "./document";
import { bindTestHost } from "./document/host/wiring.test.support";
import { installDiskHost } from "./diskHost.test.support";
import { initParser } from "./render/parse";
import { setGraphMeta } from "./graphSession";
import { renameHelperRequest } from "./renameHelperDialogs";
import { aliasDialogRequest } from "./renameAliasDialog";
import { setToasts } from "./toasts";
import type { AliasDialogOffer, BlockDto, DayMoveOffer, GraphMeta, PageDto, PageRead, RenameOffer } from "./types";

// Crash and reopen across the rename helpers and the day move (card KC2SE,
// reviews 2171d5664 / d4f72086d follow-up 1; contract rename-merge-domain.md
// "Rename helpers", "Day move"). Each helper flow writes several files in
// turn: its helper edits (one guarded save each), then one request (a rename
// or merge, one host operation) or one two-page host move. A crash can stop
// the window after any of those writes. Every file write is whole (temp,
// fsync, rename), and the host's move drafts its receiver first, so a reopen
// finds the files after the first k writes, for every k, or (a move killed
// between its halves) the receiver recovered beside the unchanged day: the
// duplicate SPEC-s2 accepts (Rust: page_host `production_move_crash_tests.rs`,
// a real kill at every boundary, then a cold relaunch).
// For every k: no block text and no tag is lost, and no two files claim one
// name. The contract's recovery state for a crash before the request is the
// helper edits saved, the request not done (an ordinary property edit).

const DAY = "Nov 18th, 2026";
const DAY_FILE = "journals/2026_11_18.md";
let serial = 0;
const block = (raw: string): BlockDto => ({ id: `ir-${++serial}`, raw, collapsed: false, children: [] });
const page = (name: string, path: string, blocks: BlockDto[], pre_block: string | null = null): PageRead => ({
  name, title: name, kind: path.startsWith("journals/") ? "journal" : "page", id: path, rev: `initial-${name}`, pre_block, blocks,
  format: "md",
});

/** The files on disk, and the graph after each write (index k: after k writes). */
let files: Map<string, PageDto>;
let states: Map<string, PageDto>[];
let n = 0;
const snapshot = () => states.push(new Map([...files].map(([key, dto]) => [key, structuredClone(dto)])));
/** One write the window or the host makes, whole: its state is one boundary. */
const write = (key: string, dto: PageDto | null) => {
  if (dto) files.set(key, { ...structuredClone(dto), rev: `w${++n}` });
  else files.delete(key);
  snapshot();
  return `w${n}`;
};
async function bindDisk(pages: PageRead[]) {
  files = new Map(pages.map((p) => [p.id, structuredClone(p)]));
  states = [];
  snapshot();
  return installDiskHost(await bindTestHost(), {
    read: (key) => files.get(key) ?? null,
    write: (key, dto) => write(key, dto),
  });
}
const absent = () => {
  vi.spyOn(backend(), "getPage").mockImplementation(async (name) => {
    const found = [...files.entries()].find(([, p]) => p.name === name);
    return found ? { ...structuredClone(found[1]), id: found[0] } : null;
  });
  vi.spyOn(backend(), "resolvePage").mockImplementation(async (name) => {
    const found = [...files.entries()].find(([, p]) => p.name === name);
    return found ? { kind: "existing", id: found[0], others: [] } : { kind: "absent", id: `pages/${name}.md` };
  });
};
const offer = (value: RenameOffer) => vi.spyOn(backend(), "renameOffer").mockResolvedValue(value);

/** Every line a file holds: its header, then every block in outline order. */
const lines = (dto: PageDto): string[] => {
  const raws = (blocks: BlockDto[]): string[] => blocks.flatMap((b) => [b.raw, ...raws(b.children)]);
  return [...(dto.pre_block ?? "").split("\n"), ...raws(dto.blocks).flatMap((raw) => raw.split("\n"))];
};
const tagsOf = (dto: PageDto) => lines(dto).flatMap((line) => line.startsWith("tags:: ") ? line.slice(7).split(",").map((t) => t.trim()) : []);

/** The reopen check at every write boundary: `notes` each in some file, every
 *  tag in `tags` on some file, no two files under one name. */
function everyBoundaryKeeps(notes: string[], tags: string[] = []) {
  expect(states.length).toBeGreaterThan(1);
  states.forEach((graph, k) => {
    const all = [...graph.values()];
    for (const note of notes) {
      expect(all.some((dto) => lines(dto).includes(note)), `after ${k} writes, “${note}” is in no file`).toBe(true);
    }
    for (const tag of tags) {
      expect(all.some((dto) => tagsOf(dto).includes(tag)), `after ${k} writes, tag “${tag}” is on no page`).toBe(true);
    }
    const names = all.map((dto) => dto.name.toLowerCase());
    expect(new Set(names).size, `after ${k} writes, two files claim one name: ${names}`).toBe(names.length);
  });
}

beforeAll(() => initParser());
beforeEach(() => {
  serial = 0;
  n = 0;
  resetStore();
  setToasts([]);
  setGraphMeta({ root: "/g", pages_dir: "pages", journals_dir: "journals", assets_dir: "assets", preferred_format: "md" } as unknown as GraphMeta);
});
afterEach(() => { vi.restoreAllMocks(); resetStore(); setGraphMeta(null); setToasts([]); });

/** The merge request as the host performs it, one operation: the source's
 *  blocks join the survivor and the source file goes (to the trash). */
const mergeOnDisk = (from: string, into: string) => {
  const survivor = files.get(into)!, source = files.get(from)!;
  files.delete(from);
  write(into, { ...survivor, blocks: [...survivor.blocks, ...source.blocks] });
};

describe("a crash at any write boundary of a rename helper loses nothing on reopen", () => {
  const tagged = { kind: "merge", into: "pages/New.md", name: "New", journal: false,
    tags: { survivor: "b, c", source: "a, b", united: "b, c, a" } } as const;

  it("tag union, then the merge", async () => {
    const survivor = page("New", "pages/New.md", [block("kept note")], "tags:: b, c");
    const source = page("Old", "pages/Old.md", [block("gone note")], "tags:: a, b");
    loadFeed([survivor, source]);
    await bindDisk([survivor, source]);
    offer(tagged);
    vi.spyOn(backend(), "confirm").mockResolvedValue(true);
    vi.spyOn(backend(), "renamePage").mockImplementation(async () => {
      mergeOnDisk("pages/Old.md", "pages/New.md");
      return { outcome: "merged", renamed: [{ from: "Old", to: "New", path: "pages/Old.md" }], touched: [] };
    });
    expect(await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("merged");
    expect(await flushAll()).toBe(true);
    // The union is saved before the source's tag line goes, then the merge.
    expect(states.map((graph) => [...graph.keys()].sort().join(" ") + " | " + [...graph.values()].map((d) => d.pre_block ?? "-").join(" / "))).toEqual([
      "pages/New.md pages/Old.md | tags:: b, c / tags:: a, b",
      "pages/New.md pages/Old.md | tags:: b, c, a / tags:: a, b",
      "pages/New.md pages/Old.md | tags:: b, c, a / -",
      "pages/New.md | tags:: b, c, a",
    ]);
    everyBoundaryKeeps(["kept note", "gone note"], ["a", "b", "c"]);
  });

  // The survivor's save is slow (a slow disk or a synced folder): the source's
  // tag removal, already in the document, must still not reach its file first.
  it("tag union with a slow survivor save", async () => {
    const survivor = page("New", "pages/New.md", [block("kept note")], "tags:: b, c");
    const source = page("Old", "pages/Old.md", [block("gone note")], "tags:: a, b");
    loadFeed([survivor, source]);
    const spies = await bindDisk([survivor, source]);
    const submit = spies.submit.getMockImplementation()!;
    let slow = true;
    spies.submit.mockImplementation(async (...args) => {
      if (slow && args[2] === "pages/New.md") {
        slow = false;
        await new Promise((resolve) => setTimeout(resolve, 1500));
      }
      return submit(...args);
    });
    offer(tagged);
    vi.spyOn(backend(), "confirm").mockResolvedValue(true);
    vi.spyOn(backend(), "renamePage").mockImplementation(async () => {
      mergeOnDisk("pages/Old.md", "pages/New.md");
      return { outcome: "merged", renamed: [{ from: "Old", to: "New", path: "pages/Old.md" }], touched: [] };
    });
    expect(await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("merged");
    expect(await flushAll()).toBe(true);
    everyBoundaryKeeps(["kept note", "gone note"], ["a", "b", "c"]);
  }, 10_000);

  // The request is refused: the put-back restores the source's tag line
  // before it takes the union off the survivor.
  it("tag union put back after a refused merge", async () => {
    const survivor = page("New", "pages/New.md", [block("kept note")], "tags:: b, c");
    const source = page("Old", "pages/Old.md", [block("gone note")], "tags:: a, b");
    loadFeed([survivor, source]);
    await bindDisk([survivor, source]);
    offer(tagged);
    vi.spyOn(backend(), "confirm").mockResolvedValue(true);
    vi.spyOn(backend(), "renamePage").mockRejectedValue(new Error("refused"));
    await expect(renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).rejects.toThrow("refused");
    expect(await flushAll()).toBe(true);
    expect([...files.values()].map((d) => d.pre_block)).toEqual(["tags:: b, c", "tags:: a, b"]);
    expect(states).toHaveLength(5); // two helper saves, two put-back saves
    everyBoundaryKeeps(["kept note", "gone note"], ["a", "b", "c"]);
  });

  it("use the name for this page, then the rename", async () => {
    const declarer = page("Machine Learning", "pages/Machine Learning.md", [block("ml body")], "alias:: AI, ML");
    const second = page("Models", "pages/Models.md", [block("models body")], "alias:: ML");
    const source = page("ML notes", "pages/ML notes.md", [block("notes body")]);
    loadFeed([declarer, second, source]);
    await bindDisk([declarer, second, source]);
    const dialog: AliasDialogOffer = {
      kind: "alias",
      reason: { kind: "aliasHeld", alias: "ML", page: "Machine Learning", path: "pages/Machine Learning.md" },
      message: "",
      opens: { name: "Machine Learning", path: "pages/Machine Learning.md", journal: false, aliases: ["AI", "ML"] },
      component: [{ name: "Machine Learning", path: "pages/Machine Learning.md", journal: false, aliases: ["AI", "ML"] }],
      source: { name: "ML notes", path: "pages/ML notes.md", journal: false, aliases: [] },
      merges: [],
      mergesRefused: [],
      own: {
        edits: [
          { page: "Machine Learning", path: "pages/Machine Learning.md", journal: false, key: "alias", before: "AI, ML", after: "AI" },
          { page: "Models", path: "pages/Models.md", journal: false, key: "alias", before: "ML", after: null },
        ],
        effect: { pages: ["Machine Learning", "Models", "ML notes"], pageCount: 3, retargeted: [] },
      },
      ownRefused: null,
    };
    offer(dialog);
    vi.spyOn(backend(), "renamePage").mockImplementation(async () => {
      const moved = files.get("pages/ML notes.md")!;
      files.delete("pages/ML notes.md");
      write("pages/ML.md", { ...moved, name: "ML", title: "ML" });
      return { outcome: "renamed", renamed: [{ from: "ML notes", to: "ML", path: "pages/ML notes.md" }], touched: [] };
    });
    const done = renameOrMergePage("ML notes", "ML");
    await vi.waitFor(() => expect(aliasDialogRequest()).not.toBeNull());
    aliasDialogRequest()!.finish("own");
    expect(await done).toBe("renamed");
    expect(await flushAll()).toBe(true);
    expect(states).toHaveLength(4); // two alias saves, then the rename
    everyBoundaryKeeps(["ml body", "models body", "notes body"]);
  });
});

describe("a crash at any write boundary of a day move loses no note on reopen", () => {
  const dayMove = (over: Partial<DayMoveOffer> = {}): DayMoveOffer => ({
    kind: "dayMove", reason: { kind: "day", name: DAY }, message: "",
    day: { name: DAY, path: DAY_FILE, journal: true, aliases: [] },
    name: "Trip", journal: false, into: null, path: "pages/Trip.md", references: [], referenceCount: 0, release: null, retargeted: null,
    ...over,
  });
  const answer = async () => {
    await vi.waitFor(() => expect(renameHelperRequest()).not.toBeNull());
    (renameHelperRequest()!.finish as (answer: true) => void)(true);
  };
  const notes = ["day note", "TODO call", "child note"];
  const dayPage = (pre: string | null) => {
    const parent = block("TODO call");
    parent.children = [block("child note")];
    return page(DAY, DAY_FILE, [block("day note"), parent], pre);
  };
  const target = { name: DAY, pageKind: "journal" as const, path: DAY_FILE };

  it.each([
    ["to a new page", null, dayMove(), false],
    ["into an existing page", null, dayMove({ into: { name: "Trip", path: "pages/Trip.md", journal: false, aliases: [] } }), true],
    ["to the day's own alias (the alias released first)", "alias:: Launch day", dayMove({ name: "Launch day", path: "pages/Launch day.md",
      day: { name: DAY, path: DAY_FILE, journal: true, aliases: ["Launch day"] },
      release: { page: DAY, path: DAY_FILE, journal: true, key: "alias", before: "Launch day", after: null } }), false],
  ] as const)("%s", async (_, pre, made, existing) => {
    const day = dayPage(pre);
    const trip = page("Trip", "pages/Trip.md", [block("trip plan")]);
    loadFeed(existing ? [day, trip] : [day]);
    await bindDisk(existing ? [day, trip] : [day]);
    absent();
    offer(made);
    const done = renameOrMergePage(DAY, made.name, target);
    await answer();
    expect(await done).toEqual({ moved: made.name, kind: "page" });
    expect(await flushAll()).toBe(true);
    // The alias save (own alias only), then the host move: receiver, then day.
    expect(states).toHaveLength(made.release ? 4 : 3);
    everyBoundaryKeeps(existing ? [...notes, "trip plan"] : notes);
    // The documented recovery state after the alias save alone: the day
    // without the alias line, its notes all still on it.
    if (made.release) expect(lines(states[1].get(DAY_FILE)!)).not.toContain("alias:: Launch day");
  });
});
