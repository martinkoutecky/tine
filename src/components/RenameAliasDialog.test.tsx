import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { renameOrMergePage } from "../graph";
import { resetStore } from "../document";
import { RenameAliasDialog } from "./RenameAliasDialog";
import type { AliasDialogOffer } from "../types";

// D-20261115-HELP: the alias dialog shows what Tine knows and offers only the
// requests the model allows, each with what it changes; the user's click is
// the request. Driven through the rename path the title and menu use.

let dispose: () => void;
beforeEach(() => { dispose = render(() => <RenameAliasDialog />, document.body); });
afterEach(() => { dispose(); document.body.innerHTML = ""; resetStore(); vi.restoreAllMocks(); });
const dialog = () => document.querySelector(".tine-rename-alias-dialog");

const offer = (over: Partial<AliasDialogOffer> = {}): AliasDialogOffer => ({
  kind: "alias",
  reason: { kind: "aliasHeld", alias: "ML", page: "Machine Learning", path: "pages/Machine Learning.md" },
  message: "",
  opens: { name: "Machine Learning", path: "pages/Machine Learning.md", journal: false, aliases: ["AI", "ML"] },
  declarers: [{ name: "Machine Learning", path: "pages/Machine Learning.md", journal: false, aliases: ["AI", "ML"] }],
  source: { name: "ML notes", path: "pages/ML notes.md", journal: false, aliases: ["Notes"] },
  merge: { into: "pages/Machine Learning.md", name: "Machine Learning", journal: false, tags: { survivor: "x", source: "y", united: "x, y" },
    effect: { pages: ["Machine Learning", "ML notes", "q"], pageCount: 3, retargeted: [{ name: "Notes", before: "ML notes", after: "Machine Learning", pages: ["r"], pageCount: 1 }] } },
  mergeRefused: null,
  own: {
    edits: [{ page: "Machine Learning", path: "pages/Machine Learning.md", journal: false, key: "alias", before: "AI, ML", after: "AI" }],
    effect: { pages: ["Machine Learning", "ML notes"], pageCount: 2, retargeted: [{ name: "ML", before: "Machine Learning", after: "ML", pages: ["q"], pageCount: 1 }] },
  },
  ownRefused: null,
  ...over,
});

it("shows the component, what the name opens now, and each request's effect", async () => {
  vi.spyOn(backend(), "renameOffer").mockResolvedValue(offer());
  const rename = vi.spyOn(backend(), "renamePage");
  const done = renameOrMergePage("ML notes", "ML");
  await vi.waitFor(() => expect(dialog()).not.toBeNull());
  const text = dialog()!.textContent!;
  expect(text).toContain("“ML” already opens “Machine Learning”");
  const owner = dialog()!.querySelector('[data-component-page="Machine Learning"]')!.textContent!;
  expect(owner).toContain("alias:: AI, ML");
  expect(owner).toContain("opens “ML” now");
  expect(dialog()!.querySelector('[data-component-page="ML notes"]')!.textContent).toContain("the page you are renaming");
  expect(text).toContain("Merge “ML notes” into “Machine Learning”");
  expect(text).toContain("The tags of both pages are united first: tags:: x, y");
  expect(text).toContain("Changes 3 pages: Machine Learning, ML notes, q");
  expect(text).toContain("Links to “Notes” open “Machine Learning” instead of “ML notes”, their text unchanged, in 1 page: r");
  expect(text).toContain("Use the name “ML” for this page");
  expect(text).toContain("alias:: AI, ML becomes alias:: AI");
  expect(text).toContain("Links to “ML” open “ML” instead of “Machine Learning”, their text unchanged, in 1 page: q");
  (dialog()!.querySelector("[data-choice-cancel]") as HTMLButtonElement).click();
  expect(await done).toBe("cancelled");
  expect(dialog()).toBeNull();
  expect(rename).not.toHaveBeenCalled();
});

it("a refused request is not offered: the dialog says why and shows the request that is allowed", async () => {
  vi.spyOn(backend(), "renameOffer").mockResolvedValue(offer({
    own: null,
    ownRefused: { kind: "plan", reason: { kind: "capture", name: "ML notes/x", image: "ML/x", page: "ML/x" } },
  }));
  const done = renameOrMergePage("ML notes", "ML");
  await vi.waitFor(() => expect(dialog()).not.toBeNull());
  expect(dialog()!.querySelector('[data-choice="own"]')).toBeNull();
  expect(dialog()!.textContent).toContain("Using “ML” for this page is not possible: Couldn't rename: “ML notes/x” would become “ML/x”");
  expect(dialog()!.querySelector('[data-choice="merge"]')).not.toBeNull();
  document.querySelector(".tine-rename-alias-overlay")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  expect(await done).toBe("cancelled");
});

it("with neither request allowed, the dialog says why for both and asks for another name", async () => {
  vi.spyOn(backend(), "renameOffer").mockResolvedValue(offer({
    merge: null, mergeRefused: { kind: "differentFormats", source: "pages/ML notes.md", target: "pages/Machine Learning.org" },
    own: null, ownRefused: { kind: "aliasLine", page: "Machine Learning", path: "pages/Machine Learning.md" },
  }));
  const done = renameOrMergePage("ML notes", "ML");
  await vi.waitFor(() => expect(dialog()).not.toBeNull());
  expect(dialog()!.querySelectorAll("[data-choice]")).toHaveLength(0);
  const text = dialog()!.textContent!;
  expect(text).toContain("Merging into “Machine Learning” is not possible: Cannot merge:");
  expect(text).toContain("the alias line of “Machine Learning” holds more than the aliases, or repeats");
  expect(text).toContain("Pick another name instead.");
  (dialog()!.querySelector("[data-choice-cancel]") as HTMLButtonElement).click();
  expect(await done).toBe("cancelled");
});

it("clicking a request is the decision: the merge runs with no second question", async () => {
  vi.spyOn(backend(), "renameOffer").mockResolvedValue(offer({ merge: { ...offer().merge!, tags: null } }));
  const confirm = vi.spyOn(backend(), "confirm");
  const rename = vi.spyOn(backend(), "renamePage").mockResolvedValue({ outcome: "merged", renamed: [{ from: "ML notes", to: "Machine Learning", path: "pages/ML notes.md" }], touched: [] });
  const done = renameOrMergePage("ML notes", "ML", { name: "ML notes", pageKind: "page", path: "pages/ML notes.md" });
  await vi.waitFor(() => expect(dialog()).not.toBeNull());
  (dialog()!.querySelector('[data-choice="merge"]') as HTMLButtonElement).click();
  expect(await done).toBe("merged");
  expect(confirm).not.toHaveBeenCalled();
  expect(rename).toHaveBeenCalledWith("ML notes", "Machine Learning", "rename-page", "pages/ML notes.md", "pages/Machine Learning.md", []);
});
