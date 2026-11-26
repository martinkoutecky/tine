// GH #1012: a date picked on a block without title text goes after an empty
// first line, because Logseq reads a block's first line as its title and takes
// no date from it. The commands that run on that shape afterwards keep it.
import { beforeAll, beforeEach, describe, expect, it } from "vitest";
import { blockRegions, initParser } from "../render/parse";
import { resetStore } from "./index";
import { loadSingle } from "./workingSet";
import { doc } from "./model";
import { rawWithHeading, setSchedule } from "./edits/properties";
import { cycleMarker, setMarker } from "../editor/marker";
import { toggleTaskDone } from "../editor/repeat";
import type { Format } from "../types";

beforeAll(() => initParser());
beforeEach(() => resetStore());

const LB = ":LOGBOOK:\nCLOCK: [2026-10-01 Wed 10:00]--[2026-10-01 Wed 10:05] =>  00:05:00\n:END:";

function picked(raw: string, format: Format, repeater: string | null = null): string {
  resetStore();
  loadSingle({ name: "P", kind: "page", title: "P", pre_block: null, blocks: [{ id: "b", raw, collapsed: false, children: [] }], format });
  setSchedule("b", "scheduled", { y: 2026, m: 9, d: 3 }, repeater); // m is 0-based
  return doc.byId.b.raw;
}
const datedOffLine0 = (raw: string, format: Format) => {
  const planning = blockRegions(raw, format).planning;
  return planning.length > 0 && planning.every(p => p.line[0] > 0);
};

describe("a date picked on a block without title text (GH #1012)", () => {
  it.each(["md", "org"] as const)("is written after an empty first line (%s)", (format) => {
    expect(picked("", format)).toBe("\nSCHEDULED: <2026-10-03 Sat>");
    expect(picked(LB, format)).toBe(`\nSCHEDULED: <2026-10-03 Sat>\n${LB}`);
    expect(picked("x", format)).toBe("x\nSCHEDULED: <2026-10-03 Sat>");
  });

  it.each(["md", "org"] as const)("a marker goes on the empty title line and comes off it again (%s)", (format) => {
    const raw = picked("", format);
    const todo = cycleMarker(raw, "todo").raw;
    expect(todo).toBe("TODO \nSCHEDULED: <2026-10-03 Sat>");
    expect(blockRegions(todo, format).header.marker).toBe("TODO");
    expect(datedOffLine0(todo, format)).toBe(true);
    const done = setMarker(todo, "DONE");
    expect(cycleMarker(done, "todo").raw).toBe("\nSCHEDULED: <2026-10-03 Sat>");
    expect(setMarker(todo, null)).toBe("\nSCHEDULED: <2026-10-03 Sat>");
  });

  it.each(["md", "org"] as const)("completing a repeating task rolls the date on its own line (%s)", (format) => {
    const todo = setMarker(picked("", format, "+1w"), "TODO");
    expect(toggleTaskDone(todo, "todo", format)).toBe("TODO \nSCHEDULED: <2026-10-10 Sat +1w>");
  });

  it("a heading on a titled block after an empty first line is unchanged", () => {
    expect(rawWithHeading("\nTitle", "md", 1)).toBe("# Title");
  });
});
