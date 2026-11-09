import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { initParser } from "../../render/parse";
import { carryUnfinished, isDirty, loadFeed, moveBlockFeed, pageByName, resetStore } from "..";
import { doc, setDoc } from "../model";
import { loadSingle } from "../workingSet";
import { moveBlock } from "../edits/moves";
import { pushUndo, undo } from "../history";
import { capturePages, persistTransfer } from "./wiring";
import { bindTestHost } from "./wiring.test.support";
import { toasts, setToasts } from "../../toasts";
import { journalTitle } from "../../journal";
import type { BlockDto, PageDto } from "../../types";

// L2-14 (arrival audit 2026-10-22): with no page host (startup before the bind
// finishes, or a failed bind) a block move between pages used to be marked
// dirty as two independent edits and reported as done. The host-less window has
// nothing that can run the transfer protocol, so the move is refused and
// restored, and the user is told, the same way the host's own refusals are.

beforeAll(() => initParser());
afterEach(() => { vi.restoreAllMocks(); resetStore(); setToasts([]); });

function loadPair(): void {
  resetStore();
  setToasts([]);
  for (const name of ["Src", "Dst"]) loadSingle({ name, kind: "page", title: name,
    id: `pages/${name}.md`, rev: name, pre_block: "", format: "md",
    blocks: [{ id: name, raw: name === "Src" ? "the moved block" : "destination", collapsed: false, children: [] }] });
}

it("L2-14: a drag across pages with no page host leaves the block in its source and says so", async () => {
  loadPair();
  await moveBlock("Src", null, pageByName("Dst")!.roots.length, "Dst");
  expect(doc.byId.Src.page).toBe("Src");
  expect(pageByName("Src")!.roots).toEqual(["Src"]);
  expect(pageByName("Dst")!.roots).toEqual(["Dst"]);
  expect(isDirty("Src")).toBe(false);
  expect(isDirty("Dst")).toBe(false);
  expect(toasts().some(toast => toast.kind === "error" && toast.key.includes("document-transfer-rolled-back"))).toBe(true);
});

const journal = (name: string, raws: string[]): PageDto => ({
  name, kind: "journal", title: name, pre_block: null,
  blocks: raws.map((raw, i): BlockDto => ({ id: `${name}-${i}`, raw, collapsed: false, children: [] })),
});
const raws = (name: string) => pageByName(name)!.roots.map((id) => doc.byId[id].raw);
const refused = () => toasts().some(toast => toast.kind === "error" && toast.key.includes("document-transfer-rolled-back"));

it("L2-14: a cross-day move from the keyboard with no page host leaves the block in its day and says so", async () => {
  resetStore();
  setToasts([]);
  loadFeed([journal("Today", ["t1"]), journal("Older", ["o1", "o2"])]);
  expect(await moveBlockFeed("Older-0", -1)).toBe("crossed");
  expect(raws("Today")).toEqual(["t1"]);
  expect(raws("Older")).toEqual(["o1", "o2"]);
  expect(refused()).toBe(true);
});

it("L2-14: carrying tasks to today with no page host reports it was not saved and keeps them in their day", async () => {
  resetStore();
  setToasts([]);
  const today = journalTitle(new Date());
  loadFeed([journal(today, [""]), journal("Older", ["TODO carry me", "note"])]);
  const { persisted } = carryUnfinished(["Older"], true, null);
  expect(await persisted).toBe(false);
  expect(raws("Older")).toEqual(["TODO carry me", "note"]);
  expect(raws(today)).toEqual([""]);
  expect(refused()).toBe(true);
});

it("L2-14: persistTransfer with no page host refuses a transfer instead of reporting it done", async () => {
  loadPair();
  const before = capturePages(["Src", "Dst"]);
  setDoc("pages", p => p.name === "Src", "roots", []);
  setDoc("pages", p => p.name === "Dst", "roots", roots => [...roots, "Src"]);
  setDoc("byId", "Src", "page", "Dst");
  expect(await persistTransfer(before, "move-blocks", [["Src", "Dst"]])).toBe(false);
  expect(doc.byId.Src.page).toBe("Src");
  expect(pageByName("Dst")!.roots).toEqual(["Dst"]);
});

it("L2-14: undo of a cross-page move with no page host is refused and restored", () => {
  loadPair();
  pushUndo("move", ["Src", "Dst"]);
  setDoc("pages", p => p.name === "Src", "roots", []);
  setDoc("pages", p => p.name === "Dst", "roots", roots => [...roots, "Src"]);
  setDoc("byId", "Src", "page", "Dst");
  undo();
  expect(doc.byId.Src.page).toBe("Dst");
  expect(pageByName("Dst")!.roots).toEqual(["Dst", "Src"]);
  expect(toasts().some(toast => toast.kind === "error" && toast.key.includes("document-transfer-rolled-back"))).toBe(true);
});

it("L2-14: a multi-page edit with no block moving between pages stays an ordinary edit, with or without a host", async () => {
  loadPair();
  // No transfer edge: two independent page edits are noted and saved when the host binds.
  expect(await persistTransfer(capturePages(["Src", "Dst"]), "replace-page", [])).toBe(true);
  expect(isDirty("Src")).toBe(true);
  expect(isDirty("Dst")).toBe(true);
  expect(toasts()).toEqual([]);
  // With a host the same call is the host's ordinary dirty marking.
  loadPair();
  await bindTestHost();
  expect(await persistTransfer(capturePages(["Src", "Dst"]), "replace-page", [])).toBe(true);
  expect(isDirty("Src")).toBe(true);
  expect(toasts()).toEqual([]);
});
