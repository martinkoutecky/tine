import { describe, it, expect } from "vitest";
import { hasRepeater, rollRepeat, cycleMarkerSmart, toggleTaskDone } from "./repeat";

describe("repeaters", () => {
  it("detects a repeater on scheduled/deadline", () => {
    expect(hasRepeater("TODO x\nSCHEDULED: <2026-06-16 Tue +1w>")).toBe(true);
    expect(hasRepeater("TODO x\nSCHEDULED: <2026-06-16 Tue>")).toBe(false);
    expect(hasRepeater("plain")).toBe(false);
  });

  it("rolls a weekly repeater forward and resets the marker", () => {
    const out = rollRepeat("DOING water plants\nSCHEDULED: <2026-06-16 Tue +1w>", "todo");
    expect(out).toBe("TODO water plants\nSCHEDULED: <2026-06-23 Tue +1w>");
  });

  it("rolls monthly + uses :now workflow open state (LATER)", () => {
    const out = rollRepeat("NOW pay rent\nDEADLINE: <2026-06-16 Tue +1m>", "now");
    expect(out).toBe("LATER pay rent\nDEADLINE: <2026-07-16 Thu +1m>");
  });

  it("cycleMarkerSmart rolls a repeater instead of marking DONE", () => {
    const { raw } = cycleMarkerSmart("DOING jog\nSCHEDULED: <2026-06-16 Tue +1d>", "todo");
    expect(raw).toBe("TODO jog\nSCHEDULED: <2026-06-17 Wed +1d>");
  });

  it("cycleMarkerSmart behaves normally for non-repeating tasks", () => {
    const { raw } = cycleMarkerSmart("DOING jog", "todo");
    expect(raw).toBe("DONE jog");
  });

  it("toggleTaskDone checks an open task to DONE and unchecks back to the open marker", () => {
    expect(toggleTaskDone("TODO buy milk", "todo")).toBe("DONE buy milk");
    expect(toggleTaskDone("DOING buy milk", "todo")).toBe("DONE buy milk");
    expect(toggleTaskDone("DONE buy milk", "todo")).toBe("TODO buy milk");
    // `now` workflow unchecks to LATER, not TODO.
    expect(toggleTaskDone("DONE buy milk", "now")).toBe("LATER buy milk");
  });

  it("toggleTaskDone preserves following lines (properties/scheduled) on line 0 rewrite", () => {
    expect(toggleTaskDone("TODO ship\nSCHEDULED: <2026-07-10 Fri>", "todo")).toBe(
      "DONE ship\nSCHEDULED: <2026-07-10 Fri>"
    );
  });

  it("toggleTaskDone rolls a repeater forward instead of closing it", () => {
    expect(toggleTaskDone("TODO water\nSCHEDULED: <2026-06-16 Tue +1w>", "todo")).toBe(
      "TODO water\nSCHEDULED: <2026-06-23 Tue +1w>"
    );
  });

  it("toggleTaskDone returns null for blocks with no checkbox (no marker / CANCELED)", () => {
    expect(toggleTaskDone("just a note", "todo")).toBeNull();
    expect(toggleTaskDone("CANCELED nope", "todo")).toBeNull();
  });

  it("a ++ catch-up repeater advances past today and preserves the ++ kind", () => {
    // Stored date far in the past so catch-up must skip many occurrences.
    const out = rollRepeat("TODO standup\nSCHEDULED: <2020-01-06 Mon ++1w>", "todo")!;
    expect(out).toContain("++1w"); // NOT downgraded to +1w
    const m = /<(\d{4})-(\d{2})-(\d{2})/.exec(out)!;
    const d = new Date(+m[1], +m[2] - 1, +m[3]);
    const todayStart = new Date();
    todayStart.setHours(0, 0, 0, 0);
    expect(d.getTime()).toBeGreaterThan(todayStart.getTime());
  });
});
