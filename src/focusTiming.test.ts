// GH #623: which calls are timed, and which are also "after focus".
import { afterEach, expect, it } from "vitest";
import { AFTER_FOCUS_WINDOW_MS, FOCUS_PHASES, noteFocusReturn, resetFocusClock, timingNamesForCommand } from "./focusTiming";

afterEach(() => resetFocusClock());

it("times a page-load command always, and apart for ten seconds after a focus return", () => {
  expect(timingNamesForCommand("get_page", 1_000)).toEqual(["get_page"]);
  noteFocusReturn(2_000);
  expect(timingNamesForCommand("get_page", 3_000)).toEqual(["get_page", "get_page.afterFocus"]);
  expect(timingNamesForCommand("get_page", 2_000 + AFTER_FOCUS_WINDOW_MS)).toEqual(["get_page", "get_page.afterFocus"]);
  expect(timingNamesForCommand("get_page", 2_000 + AFTER_FOCUS_WINDOW_MS + 1)).toEqual(["get_page"]);
});

it("never times a command outside the closed list, so no argument or free text can become a name", () => {
  noteFocusReturn(0);
  expect(timingNamesForCommand("save_pages", 1)).toEqual([]);
  expect(timingNamesForCommand("My secret page", 1)).toEqual([]);
  expect(FOCUS_PHASES.every((phase) => phase.startsWith("focus."))).toBe(true);
});
