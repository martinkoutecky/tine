import { describe, it, expect } from "vitest";
import { hasRepeater, rollRepeat, cycleMarkerSmart, toggleTaskDone, markerLabelClickable, toggleMarkerLabel } from "./repeat";

describe("repeaters", () => {
  it("threads Org through planning and protects Org literal repeaters", () => {
    const literal = "#+BEGIN_SRC text\nSCHEDULED: <2026-06-16 Tue +1w>\n#+END_SRC";
    const raw = `DOING task\nSCHEDULED: <2026-06-16 Tue +1w>\n${literal}`;
    expect(hasRepeater(raw, "org")).toBe(true);
    expect(rollRepeat(raw, "todo", "org")).toBe(`TODO task\nSCHEDULED: <2026-06-23 Tue +1w>\n${literal}`);
    expect(hasRepeater(`TODO task\n${literal}`, "org")).toBe(false);
  });
  it("detects a repeater on scheduled/deadline", () => {
    expect(hasRepeater("TODO x\nSCHEDULED: <2026-06-16 Tue +1w>", "md")).toBe(true);
    expect(hasRepeater("TODO x\nSCHEDULED: <2026-06-16 Tue>", "md")).toBe(false);
    expect(hasRepeater("plain", "md")).toBe(false);
  });

  it("rolls a weekly repeater forward and resets the marker", () => {
    const out = rollRepeat("DOING water plants\nSCHEDULED: <2026-06-16 Tue +1w>", "todo", "md");
    expect(out).toBe("TODO water plants\nSCHEDULED: <2026-06-23 Tue +1w>");
  });

  it("rolls monthly + uses :now workflow open state (LATER)", () => {
    const out = rollRepeat("NOW pay rent\nDEADLINE: <2026-06-16 Tue +1m>", "now", "md");
    expect(out).toBe("LATER pay rent\nDEADLINE: <2026-07-16 Thu +1m>");
  });

  it("cycleMarkerSmart rolls a repeater instead of marking DONE", () => {
    const { raw } = cycleMarkerSmart("DOING jog\nSCHEDULED: <2026-06-16 Tue +1d>", "todo", "md");
    expect(raw).toBe("TODO jog\nSCHEDULED: <2026-06-17 Wed +1d>");
  });

  it("cycleMarkerSmart behaves normally for non-repeating tasks", () => {
    const { raw } = cycleMarkerSmart("DOING jog", "todo", "md");
    expect(raw).toBe("DONE jog");
  });

  it("toggleTaskDone checks an open task to DONE and unchecks back to the open marker", () => {
    expect(toggleTaskDone("TODO buy milk", "todo", "md")).toBe("DONE buy milk");
    expect(toggleTaskDone("DOING buy milk", "todo", "md")).toBe("DONE buy milk");
    expect(toggleTaskDone("DONE buy milk", "todo", "md")).toBe("TODO buy milk");
    // `now` workflow unchecks to LATER, not TODO.
    expect(toggleTaskDone("DONE buy milk", "now", "md")).toBe("LATER buy milk");
  });

  it("toggleTaskDone preserves following lines (properties/scheduled) on line 0 rewrite", () => {
    expect(toggleTaskDone("TODO ship\nSCHEDULED: <2026-07-10 Fri>", "todo", "md")).toBe(
      "DONE ship\nSCHEDULED: <2026-07-10 Fri>"
    );
  });

  it("toggleTaskDone rolls a repeater forward instead of closing it", () => {
    expect(toggleTaskDone("TODO water\nSCHEDULED: <2026-06-16 Tue +1w>", "todo", "md")).toBe(
      "TODO water\nSCHEDULED: <2026-06-23 Tue +1w>"
    );
  });

  it("toggleTaskDone returns null for blocks with no checkbox (no marker / CANCELED)", () => {
    expect(toggleTaskDone("just a note", "todo", "md")).toBeNull();
    expect(toggleTaskDone("CANCELED nope", "todo", "md")).toBeNull();
  });

  it("a ++ catch-up repeater advances past today and preserves the ++ kind", () => {
    // Stored date far in the past so catch-up must skip many occurrences.
    const out = rollRepeat("TODO standup\nSCHEDULED: <2020-01-06 Mon ++1w>", "todo", "md")!;
    expect(out).toContain("++1w"); // NOT downgraded to +1w
    const m = /<(\d{4})-(\d{2})-(\d{2})/.exec(out)!;
    const d = new Date(+m[1], +m[2] - 1, +m[3]);
    const todayStart = new Date();
    todayStart.setHours(0, 0, 0, 0);
    expect(d.getTime()).toBeGreaterThan(todayStart.getTime());
  });

  it("marker-label clicks use Logseq's two-state toggle instead of the keyboard cycle", () => {
    expect(toggleMarkerLabel("TODO buy milk")).toBe("DOING buy milk");
    expect(toggleMarkerLabel("DOING buy milk")).toBe("TODO buy milk");
    expect(toggleMarkerLabel("LATER buy milk")).toBe("NOW buy milk");
    expect(toggleMarkerLabel("NOW buy milk")).toBe("LATER buy milk");
    expect(toggleMarkerLabel("DONE buy milk")).toBeNull();
    expect(toggleMarkerLabel("CANCELED buy milk")).toBeNull();
    expect(["TODO", "DOING", "LATER", "NOW"].every(markerLabelClickable)).toBe(true);
    expect(markerLabelClickable("DONE")).toBe(false);
  });
});

it("D19 completing a timed task advances its date and preserves the time", () => {
  for (const format of ["md", "org"] as const) {
    expect(toggleTaskDone("TODO task\nSCHEDULED: <2026-09-30 Wed 09:00 +1w>", "todo", format))
      .toBe("TODO task\nSCHEDULED: <2026-10-07 Wed 09:00 +1w>");
  }
});
