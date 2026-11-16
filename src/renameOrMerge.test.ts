import { afterEach, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { renameOrMergePage, renameOutcomeMessage } from "./graph";
import { tryFreezeGraphRewrite } from "./document/graphRewriteState";
import { bumpGraphEpoch } from "./graphSession";
import { invalidateBinding } from "./binding";
import { resetStore } from "./document";
import { recentPages, setRecentPages, rightSidebar, setRightSidebar } from "./ui";
import { aliasRenameRequest } from "./renameAliasChoice";
import { renderEnglish, type Msg } from "./i18n";
import type { RenameOffer } from "./types";

const offer = (value: RenameOffer) => vi.spyOn(backend(), "renameOffer").mockResolvedValue(value);

afterEach(() => {
  vi.restoreAllMocks();
  resetStore();
  setRecentPages([]);
  setRightSidebar([]);
});

// GH #327 / OG `merge-pages!`: renaming onto another page's name offers a merge.
it("asks before merging onto an existing page and names the confirmed survivor", async () => {
  const asked = offer({ kind: "merge", into: "pages/New.md", name: "New", journal: false });
  const confirm = vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  const rename = vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "merged", renamed: [{ from: "Old", to: "New", path: "pages/Old.md" }], touched: [] });
  expect(await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("merged");
  expect(confirm).toHaveBeenCalledWith("Page “New” already exists. Merge “Old” into it?", { okLabel: "Merge pages", cancelLabel: "Cancel" });
  expect(rename).toHaveBeenCalledWith("Old", "New", "rename-page", "pages/Old.md", "pages/New.md", []);
  // The question is the backend's (M-Q6: the window never re-derives it).
  expect(asked).toHaveBeenCalledWith("Old", "New", "pages/Old.md");
});

it("declining the merge writes nothing", async () => {
  offer({ kind: "merge", into: "pages/New.md", name: "New", journal: false });
  vi.spyOn(backend(), "confirm").mockResolvedValue(false);
  const rename = vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "renamed", renamed: [{ from: "Old", to: "New", path: "pages/Old.md" }], touched: [] });
  expect(await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("cancelled");
  expect(rename).not.toHaveBeenCalled();
});

it("renames without asking when the backend's plan is a plain rename", async () => {
  offer({ kind: "rename" });
  const confirm = vi.spyOn(backend(), "confirm");
  const rename = vi.spyOn(backend(), "renamePage").mockResolvedValueOnce({ outcome: "renamed", renamed: [{ from: "Old", to: "New", path: "pages/Old.md" }], touched: [] }).mockResolvedValueOnce({ outcome: "renamed", renamed: [{ from: "Old", to: "old", path: null }], touched: [] });
  expect(await renameOrMergePage("Old", "New", { name: "Old", pageKind: "page", path: "pages/Old.md" })).toBe("renamed");
  // GH #609: the same identity changes spelling without a merge prompt.
  expect(await renameOrMergePage("Old", "old")).toBe("renamed");
  expect(renameOutcomeMessage("renamed", "Old", "old")).toBeNull();
  expect(confirm).not.toHaveBeenCalled();
  expect(rename).toHaveBeenNthCalledWith(1, "Old", "New", "rename-page", "pages/Old.md", undefined, []);
  expect(rename).toHaveBeenNthCalledWith(2, "Old", "old", "rename-page", undefined, undefined, []);
});

// Rule 2 B1: an alias reaches its owner's page, and a reference-only `from`
// repoints its references there, so both ask before merging.
it("retries an entry-first interrupted rename from its still-old displayed title", async () => {
  offer({ kind: "rename" });
  const confirm = vi.spyOn(backend(), "confirm");
  const rename = vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "renamed", renamed: [{ from: "old", to: "Old", path: "pages/Old.md" }], touched: [{ path: "pages/Old.md", moved: false }] });
  expect(await renameOrMergePage("old", "Old", { name: "old", pageKind: "page", path: "pages/Old.md" })).toBe("renamed");
  expect(rename).toHaveBeenCalledWith("old", "Old", "rename-page", "pages/Old.md", undefined, []);
  expect(confirm).not.toHaveBeenCalled();
});

it("asks before merging from a file-less name into the page the backend names", async () => {
  offer({ kind: "merge", into: "pages/New.md", name: "New", journal: false });
  const confirm = vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  const rename = vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "merged", renamed: [{ from: "Ghost", to: "New", path: null }], touched: [] });
  expect(await renameOrMergePage("Ghost", "New")).toBe("merged");
  expect(rename).toHaveBeenLastCalledWith("Ghost", "New", "rename-page", undefined, "pages/New.md", []);
  expect(confirm).toHaveBeenCalledTimes(1);
});

// M-Q7 (R10): a date spelling of an existing journal day offers a merge into
// the journal; the confirm names the day as the backend spells it.
it("offers a merge into a journal day reached by another date spelling", async () => {
  offer({ kind: "merge", into: "journals/2026_10_26.md", name: "Oct 26th, 2026", journal: true });
  const confirm = vi.spyOn(backend(), "confirm").mockResolvedValue(true);
  const rename = vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "merged", renamed: [{ from: "meeting", to: "2026-10-26", path: null }], touched: [] });
  expect(await renameOrMergePage("meeting", "2026-10-26")).toBe("merged");
  expect(confirm.mock.calls[0][0]).toContain("“Oct 26th, 2026”");
  expect(rename).toHaveBeenLastCalledWith("meeting", "2026-10-26", "rename-page", undefined, "journals/2026_10_26.md", []);
});

// Model r6 O3: a name that is only another page's alias is refused, naming
// the page it opens; no dialog opens and nothing is sent to rename.
it("an alias-only name is refused naming the page it opens", async () => {
  const confirm = vi.spyOn(backend(), "confirm");
  const rename = vi.spyOn(backend(), "renamePage");
  const reason = { kind: "aliasHeld" as const, alias: "ML", page: "Machine Learning", path: "pages/Machine Learning.md" };
  offer({ kind: "refused", reason, message: "" });
  const result = await renameOrMergePage("ML notes", "ML", { name: "ML notes", pageKind: "page", path: "pages/ML notes.md" });
  expect(result).toEqual({ refused: reason });
  expect(aliasRenameRequest()).toBeNull();
  expect(renderEnglish(renameOutcomeMessage(result, "ML notes", "ML") as Msg)).toBe("Couldn't rename: “ML” is an alias of the page “Machine Learning”. Pick another name, or remove that alias from “Machine Learning” first.");
  expect(confirm).not.toHaveBeenCalled();
  expect(rename).not.toHaveBeenCalled();
});

// CM-OWN-CHILD / M-Q6 and R14: the backend's refusal reaches the user with
// its reason, and nothing is sent to rename.
it("a refused rename writes nothing and words the backend's reason", async () => {
  const rename = vi.spyOn(backend(), "renamePage");
  offer({ kind: "refused", reason: { kind: "ownChild", old: "Foo", new: "Foo/bar" }, message: "" });
  const own = await renameOrMergePage("Foo", "Foo/bar");
  expect(own).toEqual({ refused: { kind: "ownChild", old: "Foo", new: "Foo/bar" } });
  expect(renderEnglish(renameOutcomeMessage(own, "Foo", "Foo/bar") as Msg)).toContain("already a page or an alias inside “Foo”");
  offer({ kind: "refused", reason: { kind: "fileTaken", file: "pages/n%2Fy.md", page: "B" }, message: "" });
  const taken = await renameOrMergePage("N/y", "n/y");
  expect(renderEnglish(renameOutcomeMessage(taken, "N/y", "n/y") as Msg)).toContain("belongs to the page “B”");
  expect(rename).not.toHaveBeenCalled();
});

// Model r6 O0, O2 and O4: no such page, a day source, and a moved file with
// conflict markers are each worded by their reason, and nothing is renamed.
it("words the O0, O2 and O4 refusals and submits no rename", async () => {
  const rename = vi.spyOn(backend(), "renamePage");
  for (const [reason, text] of [
    [{ kind: "notAPage", name: "Ghost" }, "Couldn't rename “Ghost”: no page or alias has that name."],
    [{ kind: "day", name: "Oct 26th, 2026" }, "Couldn't rename “Oct 26th, 2026”: it is a journal day, and days keep their names."],
    [{ kind: "conflictMarkers", file: "pages/Old.md" }, "Couldn't rename: pages/Old.md has unresolved conflict markers and would need changes to move. Resolve the conflict first."],
  ] as const) {
    offer({ kind: "refused", reason, message: "" });
    const result = await renameOrMergePage("old", "new");
    expect(renderEnglish(renameOutcomeMessage(result, "old", "new") as Msg)).toBe(text);
  }
  expect(rename).not.toHaveBeenCalled();
});

// Rule 2 B7: a refused start, a failed flush and a possibly committed rename
// are different outcomes with different messages.
it("distinguishes a busy rewrite from an unsaved edit and an uncertain commit", async () => {
  offer({ kind: "rename" });
  const release = tryFreezeGraphRewrite()!;
  expect(await renameOrMergePage("Old", "New")).toBe("busy");
  release();
  let finish!: () => void;
  vi.spyOn(backend(), "renamePage").mockImplementationOnce(() => new Promise((done) => { finish = () => done({ outcome: "renamed", renamed: [{ from: "Old", to: "New", path: null }], touched: [] }); }));
  const pending = renameOrMergePage("Old", "New");
  await vi.waitFor(() => expect(backend().renamePage).toHaveBeenCalledOnce());
  // The graph owner is the binding (R4): a graph switch/restore retires it.
  invalidateBinding();
  finish();
  expect(await pending).toBe("uncertain");
  expect(renameOutcomeMessage("uncertain", "Old", "New")).toContain("Check whether");
  expect(renameOutcomeMessage("busy", "Old", "New")).not.toContain("pending edits");
});

// R4 / I-20 (og-flow3): a display repaint (here: another page's rename bumping
// the epoch) on the same graph does not make a landed rename "uncertain".
it("a rename that lands after a display repaint reports what it did", async () => {
  offer({ kind: "rename" });
  let finish!: () => void;
  vi.spyOn(backend(), "renamePage").mockImplementationOnce(() => new Promise((done) => { finish = () => done({ outcome: "renamed", renamed: [{ from: "Old", to: "New", path: null }], touched: [] }); }));
  const pending = renameOrMergePage("Old", "New");
  await vi.waitFor(() => expect(backend().renamePage).toHaveBeenCalledOnce());
  bumpGraphEpoch();
  finish();
  expect(await pending).toBe("renamed");
});

// og 12b Rule 2 B2: the backend also writes nothing for a name no file and no
// reference uses (a never-saved page nobody links to), so the message may not
// claim a case-only rename there.
it("words unchanged spelling and absent source truthfully", async () => {
  offer({ kind: "rename" });
  vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "unchanged", renamed: [], touched: [] });
  const outcome = await renameOrMergePage("Draft", "Other");
  expect(outcome).toBe("unchanged");
  const message = renameOutcomeMessage(outcome, "Draft", "Other")!;
  expect(message).not.toContain("same page name");
  expect(message).toContain("“Draft”");
  expect(renameOutcomeMessage("unchanged", "Old", "Old")).toContain("already has that spelling");
  expect(renameOutcomeMessage("unchanged", "Old", "old")).toContain("no page file or reference");
});

it("a case-only rename refreshes recent and right-sidebar names through the ordinary intent", async () => {
  setRecentPages([{ name: "my note", kind: "page", path: "pages/my note.md" }]);
  setRightSidebar([{ kind: "page", name: "my note", pageKind: "page", path: "pages/my note.md" }]);
  offer({ kind: "rename" });
  vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "renamed", renamed: [{ from: "my note", to: "My Note", path: "pages/my note.md" }], touched: [{ path: "pages/my note.md", moved: true }] });
  const confirmed = vi.spyOn(backend(), "confirm");
  expect(await renameOrMergePage("my note", "My Note", { name: "my note", pageKind: "page", path: "pages/my note.md" })).toBe("renamed");
  expect(recentPages()).toEqual([{ name: "My Note", kind: "page" }]);
  expect(rightSidebar()).toEqual([{ kind: "page", name: "My Note", pageKind: "page", path: undefined }]);
  expect(confirmed).not.toHaveBeenCalled();
});
