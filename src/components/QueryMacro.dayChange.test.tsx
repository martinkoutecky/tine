// Card hB3Lv8 (Martin 2026-10-19, YX2.1): a query whose answer depends on
// today's date is asked again once the local date changes while the window
// stays open, and only such a query. The backend says which answers those are
// (`as_of_day`, the day the relative dates were resolved for); the frontend's
// one date-change signal is `currentDayKey()` (journal.ts), driven here by a
// controllable clock: the test day setter, and a resume from sleep simulated
// as "the wall clock jumped, no timer fired, then the window got focus".
//
// Intentional OG difference: OG keeps an open query's relative dates as they
// were resolved when it rendered (react cache keyed by query text and
// resolved inputs, refreshed only by DB transactions); only its journals list
// follows the date (`watch-for-date!`, 5 s).
import { afterEach, beforeAll, beforeEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import type { JSX } from "solid-js";
import { QueryMacro } from "./Macro";
import { initParser } from "../render/parse";
import { backend } from "../backend";
import { resetSharedQueryResultsForTests } from "../queryResultCache";
import { resetStore } from "../document";
import { setDoc } from "../document/model";
import { backendReadsQueries, blockRunResult } from "../tests/queryReadingsTestkit";
import { currentDayKey, journalTitle, localDateFromDayKey, localDayKey, setBackendClock, setCurrentDayKeyForTest } from "../journal";
import type { QueryResult } from "../editor/queryIr";

beforeAll(async () => {
  await initParser();
});

beforeEach(() => {
  setDoc({ byId: {}, pages: [], feed: [], loaded: true });
  setCurrentDayKeyForTest(localDayKey());
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  resetSharedQueryResultsForTests();
  resetStore();
  setCurrentDayKeyForTest(localDayKey());
  document.body.innerHTML = "";
});

function mount(node: () => JSX.Element): { root: HTMLDivElement; dispose: () => void } {
  const root = document.createElement("div");
  document.body.appendChild(root);
  const dispose = render(node, root);
  return { root, dispose };
}

function nextDay(key: number): number {
  const date = new Date(Math.floor(key / 10_000), Math.floor((key % 10_000) / 100) - 1, key % 100 + 1);
  return localDayKey(date);
}

/** A backend whose answer names the day it resolved `today` against, as the
 *  store does for a day-relative query; `dayRelative: false` answers as a
 *  date-free query does, with no day. */
function backendAnswering(dayRelative: boolean, day: () => number) {
  return vi.spyOn(backend(), "queryRun").mockImplementation(async (): Promise<QueryResult> => {
    const result = blockRunResult([{ page: "Plans", kind: "page", blocks: [
      { id: `row-${day()}`, raw: `upcoming as of ${day()}`, collapsed: false, children: [] },
    ] }]);
    return dayRelative ? { ...result, as_of_day: day() } : result;
  });
}

it("re-asks a day-relative query once the local date changes, and shows the new day's answer", async () => {
  let backendDay = currentDayKey();
  const run = backendAnswering(true, () => backendDay);
  const { root, dispose } = mount(() => <QueryMacro body="query (between today +7d)" />);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain(`upcoming as of ${backendDay}`));
    expect(run).toHaveBeenCalledTimes(1);
    const yesterday = backendDay;
    backendDay = nextDay(backendDay);
    setCurrentDayKeyForTest(backendDay);
    await vi.waitFor(() => expect(root.textContent).toContain(`upcoming as of ${backendDay}`));
    expect(root.textContent).not.toContain(`upcoming as of ${yesterday}`);
    expect(run).toHaveBeenCalledTimes(2);
    // The new answer is for the new day: nothing further to re-ask.
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(run).toHaveBeenCalledTimes(2);
  } finally {
    dispose();
  }
});

it("does not re-run a date-independent query when the date changes", async () => {
  const run = backendAnswering(false, () => 0);
  const { root, dispose } = mount(() => <QueryMacro body="query (task TODO)" />);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("upcoming as of 0"));
    expect(run).toHaveBeenCalledTimes(1);
    setCurrentDayKeyForTest(nextDay(currentDayKey()));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(run).toHaveBeenCalledTimes(1);
  } finally {
    dispose();
  }
});

it("a query mounted after the date changed does not adopt yesterday's cached answer", async () => {
  let backendDay = currentDayKey();
  const run = backendAnswering(true, () => backendDay);
  const first = mount(() => <QueryMacro body="query (between -30d +7d)" />);
  await vi.waitFor(() => expect(first.root.textContent).toContain(`upcoming as of ${backendDay}`));
  first.dispose();
  backendDay = nextDay(backendDay);
  setCurrentDayKeyForTest(backendDay);
  // Today's journal (agenda, default queries) mounts its queries afresh.
  const second = mount(() => <QueryMacro body="query (between -30d +7d)" />);
  try {
    await vi.waitFor(() => expect(second.root.textContent).toContain(`upcoming as of ${backendDay}`));
    expect(run).toHaveBeenCalledTimes(2);
  } finally {
    second.dispose();
  }
});

it("after a resume from sleep, the first focus re-asks a day-relative query", async () => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(2030, 6, 15, 23, 0));
  setCurrentDayKeyForTest(localDayKey());
  const run = backendAnswering(true, () => localDayKey());
  const { root, dispose } = mount(() => <QueryMacro body="query (between today +7d)" />);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("upcoming as of 20300715"));
    // The machine slept through midnight: the wall clock moved, the midnight
    // timer did not fire. Returning to the window resynchronizes the day.
    vi.setSystemTime(new Date(2030, 6, 16, 8, 0));
    expect(run).toHaveBeenCalledTimes(1);
    window.dispatchEvent(new Event("focus"));
    await vi.waitFor(() => expect(root.textContent).toContain("upcoming as of 20300716"));
    expect(run).toHaveBeenCalledTimes(2);
  } finally {
    dispose();
  }
});

it("a resume no focus or visibility event announces is caught by the next backend clock sample", async () => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(2030, 6, 15, 23, 0));
  setCurrentDayKeyForTest(localDayKey());
  const run = backendAnswering(true, () => localDayKey());
  const { root, dispose } = mount(() => <QueryMacro body="query (between today +7d)" />);
  try {
    await vi.waitFor(() => expect(root.textContent).toContain("upcoming as of 20300715"));
    vi.setSystemTime(new Date(2030, 6, 16, 8, 0));
    // The ten-minute sample of an agreeing backend clock (no zone change).
    const now = Date.now();
    setBackendClock({ offset_minutes: -new Date(now).getTimezoneOffset(), unix_ms: now });
    await vi.waitFor(() => expect(root.textContent).toContain("upcoming as of 20300716"));
    expect(run).toHaveBeenCalledTimes(2);
  } finally {
    dispose();
  }
});

it("an advanced query whose :current-page falls back to today's journal follows the new day", async () => {
  // OG `current-page-fn`: no routed page and no home page binds today's journal.
  const argument = "{:query [:find (pull ?b [*]) :in $ ?p :where [?b :block/page ?p]] :inputs [:current-page]}";
  backendReadsQueries({ [argument]: { form: argument, kind: "advanced" } });
  const run = vi.spyOn(backend(), "queryRun").mockResolvedValue(blockRunResult([]));
  const today = () => journalTitle(localDateFromDayKey(currentDayKey()));
  const { dispose } = mount(() => <QueryMacro body={`query ${argument}`} />);
  try {
    await vi.waitFor(() => expect(run).toHaveBeenCalledTimes(1));
    expect(run.mock.calls[0][2]).toEqual({ current_page: today() });
    const before = today();
    setCurrentDayKeyForTest(nextDay(currentDayKey()));
    await vi.waitFor(() => expect(run).toHaveBeenCalledTimes(2));
    expect(run.mock.calls[1][2]).toEqual({ current_page: today() });
    expect(today()).not.toBe(before);
  } finally {
    dispose();
  }
});
