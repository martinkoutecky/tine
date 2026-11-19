import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { renameOrMergePage } from "../graph";
import { resetStore } from "../document";
import { initParser } from "../render/parse";
import { RenameHelperDialog } from "./RenameHelperDialog";
import type { ContestedOffer, DayMoveOffer, PageRead } from "../types";

// D-20261111-CMDF, -MOA, D-20261019-M2(6), D-20261115-HELP: the day-move and
// contested-name dialogs show what Tine knows (what moves, what stays on the
// day, which files claim the name) and the click is the request. Driven
// through the rename path the title and the page menu use.

let dispose: () => void;
beforeAll(() => initParser());
beforeEach(() => { dispose = render(() => <RenameHelperDialog />, document.body); });
afterEach(() => { dispose(); document.body.innerHTML = ""; resetStore(); vi.restoreAllMocks(); });
const dialog = () => document.querySelector(".tine-rename-alias-dialog");

const DAY = "Nov 18th, 2026";
const day: PageRead = { name: DAY, title: DAY, kind: "journal", id: "journals/2026_11_18.md", rev: "r1", pre_block: "alias:: Launch day",
  blocks: [{ id: "d1", raw: "day note", collapsed: false, children: [] }] };
const dayMove = (over: Partial<DayMoveOffer> = {}): DayMoveOffer => ({
  kind: "dayMove", reason: { kind: "day", name: DAY }, message: "",
  day: { name: DAY, path: day.id, journal: true, aliases: ["Launch day"] },
  name: "Trip", journal: false, into: null, path: "pages/Trip.md", references: ["diary", "plans"], referenceCount: 2, release: null, retargeted: null,
  ...over,
});

it("the day move says where the notes go, what stays on the day and which links keep opening it", async () => {
  vi.spyOn(backend(), "getPage").mockImplementation(async (name) => name === DAY ? structuredClone(day) : null);
  vi.spyOn(backend(), "renameOffer").mockResolvedValue(dayMove());
  const move = vi.spyOn(backend(), "pageMove");
  const done = renameOrMergePage(DAY, "Trip", { name: DAY, pageKind: "journal", path: day.id });
  await vi.waitFor(() => expect(dialog()).not.toBeNull());
  const text = dialog()!.textContent!;
  expect(text).toContain("“Nov 18th, 2026” keeps its date. Move its notes to “Trip”?");
  expect(text).toContain("Its blocks move to a new page, “Trip”.");
  expect(text).toContain("gets one block: Moved to [[Trip]]");
  expect(text).toContain("Its alias line stays on the day: alias:: Launch day");
  expect(text).toContain("Links to “Nov 18th, 2026” keep opening the day, in 2 pages: diary, plans");
  expect(dialog()!.querySelector('[data-choice="move"]')!.textContent).toBe("Move the notes to “Trip”");
  (dialog()!.querySelector("[data-choice-cancel]") as HTMLButtonElement).click();
  expect(await done).toBe("cancelled");
  expect(dialog()).toBeNull();
  expect(move).not.toHaveBeenCalled();
});

it("into an existing page, the button names the merge, and the own-alias case names the links that follow", async () => {
  vi.spyOn(backend(), "getPage").mockImplementation(async (name) => name === DAY ? structuredClone(day) : null);
  vi.spyOn(backend(), "renameOffer").mockResolvedValueOnce(dayMove({ into: { name: "Trip", path: "pages/Trip.md", journal: false, aliases: [] } }))
    .mockResolvedValueOnce(dayMove({ name: "Launch day",
      release: { page: DAY, path: day.id, journal: true, key: "alias", before: "Launch day", after: null },
      retargeted: { name: "Launch day", before: DAY, after: "Launch day", pages: ["diary"], pageCount: 1 } }));
  let done = renameOrMergePage(DAY, "trip");
  await vi.waitFor(() => expect(dialog()).not.toBeNull());
  expect(dialog()!.textContent).toContain("into the existing page “Trip”, after that page's own blocks");
  expect(dialog()!.querySelector('[data-choice="move"]')!.textContent).toBe("Move the notes into “Trip”");
  document.querySelector(".tine-rename-alias-overlay")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
  expect(await done).toBe("cancelled");
  done = renameOrMergePage(DAY, "Launch day");
  await vi.waitFor(() => expect(dialog()).not.toBeNull());
  expect(dialog()!.textContent).toContain("Links to “Launch day” open “Launch day” instead of “Nov 18th, 2026”, their text unchanged, in 1 page: diary");
  (dialog()!.querySelector("[data-choice-cancel]") as HTMLButtonElement).click();
  expect(await done).toBe("cancelled");
});

it("the contested dialog lists the claiming files, marks the one the name opens, and keeps the clicked one", async () => {
  const contested: ContestedOffer = {
    kind: "contested", reason: { kind: "contested", name: "Twin", files: ["pages/Twin.md", "pages/twin copy.md"] },
    message: "", name: "Twin", files: ["pages/Twin.md", "pages/twin copy.md"], opens: "pages/Twin.md",
  };
  vi.spyOn(backend(), "renameOffer").mockResolvedValueOnce(contested).mockResolvedValueOnce({ kind: "refused", reason: { kind: "taken", into: "Twin" }, message: "" });
  const merge = vi.spyOn(backend(), "mergePages").mockResolvedValue(undefined as never);
  const done = renameOrMergePage("Src", "Twin");
  await vi.waitFor(() => expect(dialog()).not.toBeNull());
  expect(dialog()!.textContent).toContain("Several files claim “Twin”");
  expect(dialog()!.querySelector('[data-contested-file="pages/Twin.md"]')!.textContent).toContain("opens “Twin” now");
  expect(dialog()!.querySelector('[data-contested-file="pages/twin copy.md"]')!.textContent).not.toContain("opens “Twin” now");
  (dialog()!.querySelector('[data-keep="pages/Twin.md"]') as HTMLButtonElement).click();
  await done;
  expect(merge).toHaveBeenCalledExactlyOnceWith("pages/twin copy.md", "pages/Twin.md", ["insert-blocks", "delete-page"], undefined);
});
