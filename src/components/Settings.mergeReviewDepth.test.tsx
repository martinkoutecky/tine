import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { Settings } from "./Settings";
import { closeSettings, openSettings } from "../ui";
import { backend } from "../backend";
import { OUTLINE_MAX_DEPTH } from "../editor/outline";
import type { DiffRow } from "../types";

// og 15b (I-22 / I-4): the Review & merge row tree renders recursively. Both
// files are admitted at the parse cap, so the diff is at most OUTLINE_MAX_DEPTH
// levels deep; a conflict copy exactly at the cap must still render in full.

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function deepRows(levels: number): DiffRow[] {
  let row: DiffRow | null = null;
  for (let level = levels; level >= 1; level--) {
    const view = (text: string) => ({ uuid: "", text, child_count: row ? 1 : 0 });
    row = { id: String(level), kind: "modified", mine: view(`mine ${level}`), theirs: view(`copy ${level}`), children: row ? [row] : [] };
  }
  return [row!];
}

afterEach(() => {
  closeSettings();
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

async function openReview(rows: DiffRow[]) {
  vi.spyOn(backend(), "listSyncConflicts").mockResolvedValue([
    { path: "pages/Deep.sync-conflict-1.md", base_name: "Deep", base_path: "pages/Deep.md", kind: "page", tag: "sync-conflict-1", preview: "mine 1" },
  ]);
  vi.spyOn(backend(), "syncConflictDiff").mockResolvedValue({
    base_rev: "a", conflict_rev: "b", rows,
    mine_pre: null, theirs_pre: null, pre_differs: false, blocks_identical: false,
  });
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <Settings />, root);
  openSettings("backups");
  for (let i = 0; i < 5; i++) await tick();
  const review = [...root.querySelectorAll("button")].find((button) => button.textContent?.includes("Review"));
  expect(review, "the conflict row is listed").toBeTruthy();
  review!.click();
  for (let i = 0; i < 5; i++) await tick();
  return dispose;
}

const shown = () => [...document.querySelectorAll<HTMLElement>(".sync-merge-row")].map((row) => `${row.style.paddingLeft}:${row.querySelector(".mine")?.textContent}`);

describe("sync-conflict merge review depth", () => {
  it("renders a conflict diff exactly at the outline cap", async () => {
    const dispose = await openReview(deepRows(OUTLINE_MAX_DEPTH));
    const rows = document.querySelectorAll(".sync-merge-row");
    expect(rows.length).toBe(OUTLINE_MAX_DEPTH);
    expect(rows[rows.length - 1].textContent).toContain(`copy ${OUTLINE_MAX_DEPTH}`);
    dispose();
  });

  it("keeps document order and hides an unchanged row with its subtree", async () => {
    const v = (text: string) => ({ uuid: "", text, child_count: 0 });
    const row = (id: string, kind: DiffRow["kind"], children: DiffRow[] = []): DiffRow => ({ id, kind, mine: v(id), theirs: v(id), children });
    const dispose = await openReview([
      row("A", "modified", [row("B", "unchanged", [row("C", "modified")]), row("E", "added")]),
      row("D", "removed"),
    ]);
    expect(shown()).toEqual(["0px:A", "16px:E", "0px:D"]);
    const toggle = document.querySelector<HTMLInputElement>(".sync-merge-showunchanged input")!;
    toggle.checked = true;
    toggle.dispatchEvent(new Event("change", { bubbles: true }));
    await tick();
    expect(shown()).toEqual(["0px:A", "16px:B", "32px:C", "16px:E", "0px:D"]);
    dispose();
  });
});
