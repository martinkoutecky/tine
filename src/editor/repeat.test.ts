import { describe, it, expect } from "vitest";
import { hasRepeater, rollRepeat, cycleMarkerSmart } from "./repeat";

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
});
