import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { QueryDisplay } from "./QueryDisplay";
import { AnchorLine, AnchorPromptPanel } from "./querySheetParts";
import { QueryOgHint } from "./QueryResultParts";
import { DatePicker } from "./DatePicker";
import { closeDatePicker, openDatePicker } from "../ui";
import { resetStore } from "../document";
import { msg, renderEnglish } from "../i18n";
import type { ViewSettings } from "../editor/queryIr";

const locale = vi.hoisted(() => ({ set: (_value: boolean) => {} }));
vi.mock("../i18n", async (original) => {
  const actual = await original<typeof import("../i18n")>();
  const { createSignal } = await import("solid-js");
  const [decorated, setDecorated] = createSignal(false);
  locale.set = setDecorated;
  const display = (text: string) => decorated() ? `«${text}»` : text;
  return { ...actual,
    t: (...args: Parameters<typeof actual.t>) => display(actual.t(...args)),
    render: (text: Parameters<typeof actual.render>[0]) => display(actual.render(text)),
  };
});
afterEach(() => { locale.set(false); closeDatePicker(); resetStore(); document.body.replaceChildren(); });

it("open Display controls follow the locale while writes preserve query syntax", () => {
  const apply = vi.fn();
  const view: ViewSettings = { view: "list" };
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <QueryDisplay inline view={() => view} apply={apply} rowKind={() => "block"}
    registry={{ rows: () => [], pending: () => false, failure: () => null, unavailable: () => false, request: () => {}, retry: () => {} }} />, root);
  try {
    expect(root.textContent).toContain("Group by");
    expect(root.querySelector("input[inputmode=numeric]")?.getAttribute("placeholder")).toBe("No limit");
    locale.set(true);
    expect(root.textContent).toContain("«Group by»");
    expect(root.querySelector("input[inputmode=numeric]")?.getAttribute("placeholder")).toBe("«No limit»");
    const table = [...root.querySelectorAll("button")].find((button) => button.textContent?.replace(/[«»]/g, "") === "table");
    expect(table).toBeDefined();
    table!.click();
    expect(apply).toHaveBeenLastCalledWith({ view: "table" });
  } finally { dispose(); }
});

it("stored anchor options and a complete prompt follow the locale and preserve authored condition names", () => {
  const root = document.createElement("div");
  const authored = "客户 <em>literal</em>";
  const dispose = render(() => <>
    <AnchorLine anchor={() => "block"} onAnchor={() => {}} empty={false} openMenu={() => "anchor"} setOpenMenu={() => {}} />
    <AnchorPromptPanel prompt={{ anchor: "page", count: 1, total: 2, names: [authored], onRemove: () => {}, onKeep: () => {}, onCancel: () => {} }} />
  </>, root);
  try {
    expect(root.textContent).toContain("individual bullets, anywhere in the graph");
    expect(root.querySelector(".qs-anchor-prompt-text")?.textContent).toBe(`Switching to pages — 1 of your 2 conditions don't apply to pages (${authored}).`);
    locale.set(true);
    expect(root.textContent).toContain("«individual bullets, anywhere in the graph»");
    expect(root.querySelector(".qs-anchor-prompt-text")?.textContent).toContain("«Switching to");
    expect(root.querySelector(".qs-anchor-prompt-text code")?.textContent).toBe(authored);
    expect(root.querySelector(".qs-anchor-prompt-text em")).toBeNull();
  } finally { dispose(); }
});

it("date controls update while their persisted repeat units and clock format stay intact", () => {
  const root = document.createElement("div");
  const dispose = render(() => <DatePicker />, root);
  openDatePicker("capture-block", "scheduled", 12, 24);
  try {
    expect(root.querySelector(".dp-time-add")?.textContent ?? root.textContent).toContain("+ Add time");
    const units = () => [...root.querySelectorAll("option")].map((option) => option.value);
    expect(units()).toEqual(["", "d", "w", "m", "y"]);
    locale.set(true);
    expect(root.querySelector<HTMLButtonElement>("button[title]")?.title).toBe("«Previous month»");
    expect(root.textContent).toContain("«Daily»");
    expect(units()).toEqual(["", "d", "w", "m", "y"]);
  } finally { dispose(); }
});

it("Logseq's bare forms remain code inside the translated explanation", () => {
  const root = document.createElement("div");
  const dispose = render(() => <QueryOgHint hint={{ bare: ["task", "priority"], no_conditions: true, rewrites: [] }} />, root);
  try {
    expect(root.textContent).toBe("(task) without markers adds no condition, as in Logseq. (priority) without levels adds no condition, as in Logseq. On its own it matches nothing.");
    locale.set(true);
    expect(root.textContent).toContain("«(task) without markers adds no condition, as in Logseq. »");
    expect([...root.querySelectorAll("code")].map((code) => code.textContent)).toEqual(["(task)", "(priority)"]);
  } finally { dispose(); }
});

it("whole plural notices keep English reports and ungrouped counts", () => {
  expect(renderEnglish(msg("query-preview-count", { kind: "block", count: 1 }))).toBe("1 result");
  expect(renderEnglish(msg("query-preview-count", { kind: "page", count: 2400 }))).toBe("2400 pages");
  expect(renderEnglish(msg("query-preview-families", { pages: 1, blocks: 2 }))).toBe("1 page · 2 blocks");
  expect(renderEnglish(msg("document-external-updates", { pages: 2, conflicts: 0 }))).toBe("2 pages updated externally");
  expect(renderEnglish(msg("document-external-updates", { pages: 2, conflicts: 1 }))).toBe("2 pages updated externally · 1 conflict to review");
  expect(renderEnglish(msg("document-external-updates", { pages: 2, conflicts: 3 }))).toBe("2 pages updated externally · 3 conflicts to review");
  expect(renderEnglish(msg("document-move-incomplete", { completed: 0, pages: "客户”, “B" }))).toBe("The move did not happen: “客户”, “B” could not be saved first.");
  expect(renderEnglish(msg("document-move-incomplete", { completed: 1, pages: "客户" }))).toBe("Only part of the move happened: the blocks from “客户” did not move.");
  expect(renderEnglish(msg("sheet-cell-refused", { value: "", hasValue: 0 }))).toBe("Couldn't change that cell: the page is read-only, the row is gone, or the value doesn't fit the column.");
  expect(renderEnglish(msg("sheet-cell-refused", { value: "客户", hasValue: 1 }))).toContain("that cell to “客户”:");
  expect(renderEnglish(msg("ui-sync-conflicts", { count: 1 }))).toBe("1 new sync conflict needs review");
  expect(renderEnglish(msg("ui-sync-conflicts", { count: 2 }))).toBe("2 new sync conflicts need review");
});
