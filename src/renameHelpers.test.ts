import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { renameOrMergePage } from "./graph";
import { pageByName, resetStore, setPageProperty, setRaw } from "./document";
import { loadSingle } from "./document/workingSet";
import { doc } from "./document/model";
import { bindTestHost } from "./document/host/wiring.test.support";
import { installDiskHost, type FakeDisk } from "./diskHost.test.support";
import { initParser } from "./render/parse";
import { setGraphMeta } from "./graphSession";
import { setToasts, toasts } from "./toasts";
import { aliasDialogRequest, type AliasDialogAnswer } from "./renameAliasDialog";
import { renderEnglish } from "./i18n";
import type { AliasDialogOffer, GraphMeta, PageDto, RenameOffer } from "./types";

// The rename helpers (D-20261115-HELP, D-20261116-TAGH): the window performs
// the backend's offered edits as ordinary page edits through the page host's
// guarded save, then the one request; when the request does not happen, the
// edits are put back. Assertions read the fake disk: what reaches the files.

/** Page files on "disk": header line and body, with a revision. */
let files: Map<string, { pre: string | null; body: string; rev: number }>;
let saveFails = false;
/** Runs inside the next file write (then cleared): a graph switch or typing
 *  while a helper's save is in flight. */
let onWrite: (() => void) | null = null;
const dto = (name: string): PageDto | null => {
  const file = files.get(name);
  return file ? { name, kind: "page", title: name, id: `pages/${name}.md`, rev: `r${file.rev}`, pre_block: file.pre,
    blocks: [{ id: `${name}-b`, raw: file.body, collapsed: false, children: [] }], format: "md" } as PageDto : null;
};
const disk: FakeDisk = {
  read: (_key, name) => dto(name),
  write: (_key, page) => {
    const during = onWrite;
    onWrite = null;
    during?.();
    if (saveFails) throw new Error("disk full");
    const file = files.get(page.name)!;
    files.set(page.name, { pre: page.pre_block ?? null, body: page.blocks[0]?.raw ?? "", rev: file.rev + 1 });
    return `r${file.rev + 1}`;
  },
};
const page = (name: string, pre: string | null, body: string) => {
  files.set(name, { pre, body, rev: 1 });
  loadSingle(dto(name)!);
};
/** A page's header as its file has it. */
const header = (name: string) => files.get(name)?.pre ?? null;
const offer = (value: RenameOffer) => vi.spyOn(backend(), "renameOffer").mockResolvedValue(value);
const errors = () => toasts().filter((toast) => toast.kind === "error").map((toast) => renderEnglish(toast.text));

beforeAll(() => initParser());
beforeEach(async () => {
  resetStore();
  files = new Map();
  saveFails = false;
  onWrite = null;
  setGraphMeta({ root: "/g", pages_dir: "pages", journals_dir: "journals", assets_dir: "assets", preferred_format: "md" } as unknown as GraphMeta);
  installDiskHost(await bindTestHost(), disk);
});
afterEach(() => { vi.restoreAllMocks(); resetStore(); setGraphMeta(null); setToasts([]); });

const tagged = { kind: "merge", into: "pages/New.md", name: "New", journal: false, tags: { survivor: "b, c", source: "a, b", united: "b, c, a" } } as const;

it("a merge of two tagged pages names the united tags and writes them before the merge", async () => {
  page("New", "tags:: b, c", "- kept");
  page("Old", "tags:: a, b", "- gone");
  offer(tagged);
  const confirm = vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  const rename = vi.spyOn(backend(), "renamePage").mockImplementation(async () => {
    // The request sees the helper's edits already made.
    expect(header("New")).toBe("tags:: b, c, a");
    expect(header("Old")).toBeNull();
    return { outcome: "merged", renamed: [{ from: "Old", to: "New", path: "pages/Old.md" }], touched: [] };
  });
  expect(await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("merged");
  expect(confirm.mock.calls[0][0]).toContain("the remaining page's tags become: b, c, a");
  expect(rename).toHaveBeenCalledOnce();
});

it("a refused merge puts the tag edits back", async () => {
  page("New", "tags:: b, c", "- kept");
  page("Old", "tags:: a, b", "- gone");
  offer(tagged);
  vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  vi.spyOn(backend(), "renamePage").mockRejectedValue(new Error("disk full"));
  await expect(renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).rejects.toThrow("disk full");
  expect(header("New")).toBe("tags:: b, c");
  expect(header("Old")).toBe("tags:: a, b");
});

it("a busy rewrite leaves no tag edit behind", async () => {
  page("New", "tags:: b, c", "- kept");
  page("Old", "tags:: a, b", "- gone");
  offer(tagged);
  vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  const rename = vi.spyOn(backend(), "renamePage");
  const { tryFreezeGraphRewrite } = await import("./document/graphRewriteState");
  const release = tryFreezeGraphRewrite()!;
  const outcome = await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" });
  release();
  expect(["busy", "cancelled"]).toContain(outcome);
  expect(rename).not.toHaveBeenCalled();
  expect(header("New")).toBe("tags:: b, c");
  expect(header("Old")).toBe("tags:: a, b");
});

it("a page changed since the plan is not edited and nothing is renamed", async () => {
  page("New", "tags:: b, z", "- kept");
  page("Old", "tags:: a, b", "- gone");
  offer(tagged);
  vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  const rename = vi.spyOn(backend(), "renamePage");
  expect(await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("cancelled");
  expect(rename).not.toHaveBeenCalled();
  expect(header("New")).toBe("tags:: b, z");
  expect(header("Old")).toBe("tags:: a, b");
  expect(errors().some((text) => text.includes("“New” changed since the rename was planned"))).toBe(true);
});

const dialog = (over: Partial<AliasDialogOffer> = {}): AliasDialogOffer => ({
  kind: "alias",
  reason: { kind: "aliasHeld", alias: "ML", page: "Machine Learning", path: "pages/Machine Learning.md" },
  message: "",
  opens: { name: "Machine Learning", path: "pages/Machine Learning.md", journal: false, aliases: ["AI", "ML"] },
  component: [{ name: "Machine Learning", path: "pages/Machine Learning.md", journal: false, aliases: ["AI", "ML"] }],
  source: { name: "ML notes", path: "pages/ML notes.md", journal: false, aliases: [] },
  merges: [{ into: "pages/Machine Learning.md", name: "Machine Learning", journal: false, tags: null, effect: { pages: ["ML notes", "Machine Learning", "q"], pageCount: 3, retargeted: [] } }],
  mergesRefused: [],
  own: {
    edits: [{ page: "Machine Learning", path: "pages/Machine Learning.md", journal: false, key: "alias", before: "AI, ML", after: "AI" }],
    effect: { pages: ["Machine Learning", "ML notes"], pageCount: 2, retargeted: [{ name: "ML", before: "Machine Learning", after: "ML", pages: ["q"], pageCount: 1 }] },
  },
  ownRefused: null,
  ...over,
});
const answer = async (choice: AliasDialogAnswer) => {
  await vi.waitFor(() => expect(aliasDialogRequest()).not.toBeNull());
  aliasDialogRequest()!.finish(choice);
};

it("use the name for this page removes the alias, then renames to the typed name", async () => {
  page("Machine Learning", "alias:: AI, ML", "- ml body");
  page("ML notes", null, "- notes");
  offer(dialog());
  const confirm = vi.spyOn(backend(), "confirm");
  const rename = vi.spyOn(backend(), "renamePage").mockImplementation(async () => {
    expect(header("Machine Learning")).toBe("alias:: AI");
    return { outcome: "renamed", renamed: [{ from: "ML notes", to: "ML", path: "pages/ML notes.md" }], touched: [] };
  });
  const done = renameOrMergePage("ML notes", "ML", { name: "ML notes", pageKind: "page", path: "pages/ML notes.md" });
  await answer("own");
  expect(await done).toBe("renamed");
  expect(rename).toHaveBeenCalledWith("ML notes", "ML", "rename-page", "pages/ML notes.md", undefined, []);
  // The dialog's button is the decision: no second native question.
  expect(confirm).not.toHaveBeenCalled();
});

it("the dialog's merge is the ordinary merge into the page the name opens, with no second question", async () => {
  offer(dialog());
  const confirm = vi.spyOn(backend(), "confirm");
  const rename = vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "merged", renamed: [{ from: "ML notes", to: "Machine Learning", path: "pages/ML notes.md" }], touched: [] });
  const done = renameOrMergePage("ML notes", "ML", { name: "ML notes", pageKind: "page", path: "pages/ML notes.md" });
  await answer({ merge: "pages/Machine Learning.md" });
  expect(await done).toBe("merged");
  expect(rename).toHaveBeenCalledWith("ML notes", "Machine Learning", "rename-page", "pages/ML notes.md", "pages/Machine Learning.md", []);
  expect(confirm).not.toHaveBeenCalled();
});

it("cancelling the dialog writes nothing", async () => {
  page("Machine Learning", "alias:: AI, ML", "- ml body");
  offer(dialog());
  const rename = vi.spyOn(backend(), "renamePage");
  const done = renameOrMergePage("ML notes", "ML");
  await answer(null);
  expect(await done).toBe("cancelled");
  expect(rename).not.toHaveBeenCalled();
  expect(header("Machine Learning")).toBe("alias:: AI, ML");
});

it("a choice the backend did not offer cannot be answered", async () => {
  offer(dialog({ own: null, ownRefused: { kind: "aliasLine", page: "Machine Learning", path: "pages/Machine Learning.md" } }));
  const rename = vi.spyOn(backend(), "renamePage");
  const done = renameOrMergePage("ML notes", "ML");
  await answer("own");
  expect(aliasDialogRequest()).not.toBeNull();
  aliasDialogRequest()!.finish(null);
  expect(await done).toBe("cancelled");
  expect(rename).not.toHaveBeenCalled();
});

it("a refused rename after the alias edit puts the alias line back", async () => {
  page("Machine Learning", "alias:: AI, ML", "- ml body");
  offer(dialog());
  vi.spyOn(backend(), "renamePage").mockRejectedValue(new Error("refused"));
  const done = renameOrMergePage("ML notes", "ML");
  await answer("own");
  await expect(done).rejects.toThrow("refused");
  expect(header("Machine Learning")).toBe("alias:: AI, ML");
});

it("a helper edit whose save fails is put back and nothing is renamed", async () => {
  page("New", "tags:: b, c", "- kept");
  page("Old", "tags:: a, b", "- gone");
  offer(tagged);
  vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  const rename = vi.spyOn(backend(), "renamePage");
  saveFails = true;
  expect(await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("cancelled");
  expect(rename).not.toHaveBeenCalled();
  saveFails = false;
  expect(header("New")).toBe("tags:: b, c");
  expect(header("Old")).toBe("tags:: a, b");
  expect(errors().some((text) => text.includes("could not be saved, so it was put back"))).toBe(true);
});

/** The page's header and first block as the window holds them. */
const held = (name: string) => {
  const found = pageByName(name);
  return found ? { pre: found.preBlock ?? null, body: doc.byId[found.roots[0]]?.raw ?? null } : null;
};

it("a graph switch during a helper's save puts nothing into the new graph", async () => {
  page("New", "tags:: b, c", "- OLD GRAPH kept");
  page("Old", "tags:: a, b", "- gone");
  offer(tagged);
  vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  const rename = vi.spyOn(backend(), "renamePage");
  onWrite = () => {
    // The user switches graphs; the new graph also has a page "New".
    resetStore();
    files = new Map();
    page("New", null, "- NEW GRAPH kept");
    saveFails = true;
  };
  expect(await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("cancelled");
  saveFails = false;
  expect(rename).not.toHaveBeenCalled();
  expect(held("New")).toEqual({ pre: null, body: "- NEW GRAPH kept" });
  expect(files.get("New")).toMatchObject({ pre: null, body: "- NEW GRAPH kept" });
});

it("typing admitted during the helper's saves survives the put-back", async () => {
  page("New", "tags:: b, c", "- kept");
  page("Old", "tags:: a, b", "- gone");
  offer(tagged);
  vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  vi.spyOn(backend(), "renamePage").mockRejectedValue(new Error("disk full"));
  onWrite = () => setRaw("New-b", "- kept, typed meanwhile");
  await expect(renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).rejects.toThrow("disk full");
  expect(held("New")).toEqual({ pre: "tags:: b, c", body: "- kept, typed meanwhile" });
  expect(header("Old")).toBe("tags:: a, b");
});

it("a property changed while a later page loads is not written over", async () => {
  page("New", "tags:: b, c", "- kept");
  files.set("Old", { pre: "tags:: a, b", body: "- gone", rev: 1 });
  offer(tagged);
  vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  vi.spyOn(backend(), "getPage").mockImplementation(async () => {
    setPageProperty("New", "tags", "b, z");
    return { ...dto("Old")!, id: "pages/Old.md" };
  });
  const rename = vi.spyOn(backend(), "renamePage");
  expect(await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("cancelled");
  expect(rename).not.toHaveBeenCalled();
  expect(held("New")?.pre).toBe("tags:: b, z");
  expect(header("Old")).toBe("tags:: a, b");
});
