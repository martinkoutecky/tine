// I-9: a parser (wasm) fault in the logbook / marker helpers used to be a
// console.error, an empty fallback or nothing at all. The fallback is kept (the
// edit and the render still go through) but the user is told, with a sticky
// error toast. Each test makes the wasm entry point throw and asserts the toast.
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { initParser } from "./render/parse";
import { setToasts, toasts } from "./toasts";

const wasm = vi.hoisted(() => ({ fail: false }));
vi.mock("./render/wasm/lsdoc_wasm.js", async (importOriginal) => {
  const real = await importOriginal<typeof import("./render/wasm/lsdoc_wasm.js")>();
  const guard = <A extends unknown[], R>(fn: (...args: A) => R) => (...args: A): R => {
    if (wasm.fail) throw new Error("wasm trap");
    return fn(...args);
  };
  return {
    ...real,
    logbook_apply_marker_transition: guard(real.logbook_apply_marker_transition),
    logbook_info_json: guard(real.logbook_info_json),
    header_tokens_json: guard(real.header_tokens_json),
    page_header_json: guard(real.page_header_json),
  };
});

import { applyMarkerTransition, logbookInfo } from "./logbook";
import { headerTokens } from "./markers";
import { splitPagePreamble } from "./editor/properties";

beforeAll(() => initParser());
beforeEach(() => { setToasts([]); wasm.fail = true; });
afterEach(() => { wasm.fail = false; });

const sticky = (needle: string) => toasts().filter((t) => t.kind === "error" && t.sticky && t.message.includes(needle));

describe("parser faults are shown, with the fallback kept", () => {
  it("a failed logbook marker transition still applies the edit and says the entry was not recorded", () => {
    expect(applyMarkerTransition("TODO a", "DOING a", "md", true, false)).toBe("DOING a");
    expect(sticky("time-tracking")).toHaveLength(1);
  });

  it("an unreadable logbook renders empty and says so", () => {
    expect(logbookInfo("DONE x\n:LOGBOOK:\n:END:", "md")).toMatchObject({ seconds: 0, rows: [] });
    expect(sticky("time-tracking")).toHaveLength(1);
  });

  it("a block whose marker cannot be read shows plain text and says so", () => {
    expect(headerTokens("TODO fault-case-unique", "md")).toEqual({ marker: null, priority: null });
    expect(sticky("task marker")).toHaveLength(1);
  });

  it("a page whose property header cannot be read shows ordinary text and says so", () => {
    expect(splitPagePreamble("title:: fault-case-header\n\nbody").properties).toBeNull();
    expect(sticky("property header")).toHaveLength(1);
  });

  it("a repeated fault shares one toast", () => {
    logbookInfo("DONE x\n:LOGBOOK:\n:END:", "md");
    logbookInfo("DONE x\n:LOGBOOK:\n:END:", "md");
    expect(sticky("time-tracking")).toHaveLength(1);
  });
});
