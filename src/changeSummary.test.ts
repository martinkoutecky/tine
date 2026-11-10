import { beforeEach, expect, it } from "vitest";
import { applyChangeSummary, changeSummary, setChangeSummaryBaseline } from "./changeSummary";
import { applyGraphAnswers } from "./graphAnswers";
import { bumpGraphEpoch } from "./graphSession";
import { dataRev } from "./dataRevision";
import type { ChangeSummary } from "./backendTypes";
import { invalidateBinding } from "./binding";
import { applyGraphChangesBulk } from "./document";

const summary = (rev: string, full = false): ChangeSummary => ({
  rev, full, pages: ["a"], blocks: ["block"], linksTo: ["b"],
  props: { a: ["icon"] }, inventoryChanged: false,
});
beforeEach(() => bumpGraphEpoch());

it("late binding and a missed revision are full; consecutive summaries stay precise", () => {
  expect(changeSummary()).toBeUndefined();
  applyChangeSummary(summary("14"));
  expect(changeSummary()).toEqual(summary("14", true));
  applyChangeSummary(summary("15"));
  expect(changeSummary()).toEqual(summary("15"));
  applyChangeSummary(summary("17"));
  expect(changeSummary()).toEqual(summary("17", true));
  applyChangeSummary(summary("18", true));
  expect(changeSummary()).toEqual(summary("18", true));
});

// UI-FEED-FIRST-CHANGE-RESTART: graph open seeds the baseline from the open
// answer, so the session's first summary is the delta it is (a full one bumps
// `days` and restarts a scrolled Journals feed). A real gap still flushes.
it("a baseline from graph open makes the first summary a delta; a skipped revision is still full", () => {
  setChangeSummaryBaseline("14");
  expect(changeSummary()).toBeUndefined();
  applyChangeSummary(summary("15"));
  expect(changeSummary()).toEqual(summary("15"));
  applyChangeSummary(summary("17"));
  expect(changeSummary()).toEqual(summary("17", true));
  applyChangeSummary(summary("18"));
  expect(changeSummary()).toEqual(summary("18"));
});

it("a first summary past the open baseline is a gap and flushes in full", () => {
  setChangeSummaryBaseline("14");
  applyChangeSummary(summary("16"));
  expect(changeSummary()).toEqual(summary("16", true));
});

it("a summary the open answer already covered is a duplicate", () => {
  setChangeSummaryBaseline("14");
  applyChangeSummary(summary("14"));
  applyChangeSummary(summary("13"));
  expect(changeSummary()).toBeUndefined();
  applyChangeSummary(summary("15"));
  expect(changeSummary()).toEqual(summary("15"));
});

it("an absent baseline, a later epoch or a store reset leave the first summary full", () => {
  setChangeSummaryBaseline(null);
  applyChangeSummary(summary("15"));
  expect(changeSummary()).toEqual(summary("15", true));
  bumpGraphEpoch();
  setChangeSummaryBaseline("20");
  bumpGraphEpoch();
  applyChangeSummary(summary("21"));
  expect(changeSummary()).toEqual(summary("21", true));
  setChangeSummaryBaseline("30");
  invalidateBinding();
  applyChangeSummary(summary("31"));
  expect(changeSummary()).toEqual(summary("31", true));
});

it("a summary already received in this epoch outranks a later baseline", () => {
  applyChangeSummary(summary("15"));
  setChangeSummaryBaseline("10");
  applyChangeSummary(summary("16"));
  expect(changeSummary()).toEqual(summary("16"));
  setChangeSummaryBaseline("20");
  applyChangeSummary(summary("17"));
  expect(changeSummary()).toEqual(summary("17"));
});

it("duplicates and reordered messages do not replace the latest publication", () => {
  applyChangeSummary(summary("9007199254740992"));
  applyChangeSummary(summary("9007199254740993"));
  applyChangeSummary(summary("9007199254740993", true));
  applyChangeSummary(summary("9007199254740992"));
  expect(changeSummary()).toEqual(summary("9007199254740993"));
});

it("a graph switch drops the baseline even when revision numbers overlap", () => {
  applyChangeSummary(summary("100"));
  bumpGraphEpoch();
  expect(changeSummary()).toBeUndefined();
  applyChangeSummary(summary("1"));
  expect(changeSummary()).toEqual(summary("1", true));
});

it("a store reset drops the baseline before a new graph epoch arrives", () => {
  applyChangeSummary(summary("100"));
  invalidateBinding();
  expect(changeSummary()).toBeUndefined();
  applyChangeSummary(summary("1"));
  expect(changeSummary()).toEqual(summary("1", true));
});

it("the existing answer ingress receives summaries without changing dataRev", () => {
  const before = dataRev();
  applyGraphAnswers({ rev: "4", inventoryChanged: false, blockRefCounts: {}, summary: summary("4") });
  expect(changeSummary()).toEqual(summary("4", true));
  expect(dataRev()).toBe(before);
  applyGraphAnswers({ rev: "5", inventoryChanged: false, blockRefCounts: {} });
  applyChangeSummary(undefined);
  applyGraphAnswers({ rev: "6", inventoryChanged: false, blockRefCounts: {}, summary: summary("6") });
  expect(changeSummary()).toEqual(summary("6", true));
});

it("an own summary-only notification follows the existing binding gate and leaves refresh behavior unchanged", async () => {
  const before = dataRev();
  const answers = { rev: "4", inventoryChanged: false, blockRefCounts: {}, summary: summary("4") };
  await applyGraphChangesBulk({ changes: [], answers, binding_generation: -1 });
  expect(changeSummary()).toBeUndefined();
  await applyGraphChangesBulk({ changes: [], answers });
  expect(changeSummary()).toEqual(summary("4", true));
  expect(dataRev()).toBe(before);
});
