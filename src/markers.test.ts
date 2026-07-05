import { describe, it, expect } from "vitest";
import { MARKERS, OPEN_MARKERS, DONE_MARKERS, MARKER_RE, taskCheckboxState } from "./markers";
import { leadingMarker } from "./editor/marker";

describe("task markers (single source of truth)", () => {
  it("matches the backend set (crates/tine-core/src/doc.rs MARKERS) — keep in sync", () => {
    // If doc.rs::MARKERS changes, update this list (and vice-versa). The two can't
    // share a literal across the language boundary, so this is the drift guard.
    // This set must equal lsdoc's recognizer (lsdoc/src/parse.rs MARKERS, the
    // mldoc/OG-faithful authority) — Tine treats exactly what OG treats as a task.
    expect([...MARKERS].sort()).toEqual(
      [
        "CANCELED", "CANCELLED", "DOING", "DONE", "IN-PROGRESS",
        "LATER", "NOW", "STARTED", "TODO", "WAIT", "WAITING",
      ].sort()
    );
  });

  it("OPEN ∪ DONE partitions MARKERS with no overlap", () => {
    expect(OPEN_MARKERS.size + DONE_MARKERS.size).toBe(MARKERS.length);
    for (const m of MARKERS) {
      expect(OPEN_MARKERS.has(m) !== DONE_MARKERS.has(m)).toBe(true); // exactly one
    }
    // The drift that prompted this: IN-PROGRESS and WAIT are OPEN (carried forward).
    expect(OPEN_MARKERS.has("IN-PROGRESS")).toBe(true);
    expect(OPEN_MARKERS.has("WAIT")).toBe(true);
    expect(OPEN_MARKERS.has("STARTED")).toBe(true); // in-progress-like → carried forward
    expect(DONE_MARKERS.has("CANCELLED")).toBe(true);
  });

  it("taskCheckboxState: DONE checked, open markers unchecked, canceled/none none (OG block-checkbox)", () => {
    expect(taskCheckboxState("DONE")).toBe(true);
    for (const m of ["TODO", "DOING", "NOW", "LATER", "WAITING", "WAIT", "STARTED", "IN-PROGRESS"]) {
      expect(taskCheckboxState(m)).toBe(false); // open task → unchecked box
    }
    expect(taskCheckboxState("CANCELED")).toBeNull(); // closed-but-not-done → no box (OG)
    expect(taskCheckboxState("CANCELLED")).toBeNull();
    expect(taskCheckboxState(null)).toBeNull();
    expect(taskCheckboxState(undefined)).toBeNull();
  });

  it("MARKER_RE anchors every marker as a whole word, prefix-safe (WAITING vs WAIT)", () => {
    for (const m of MARKERS) {
      expect(MARKER_RE.exec(`${m} do the thing`)?.[1]).toBe(m);
      expect(leadingMarker(`${m} do the thing`)).toBe(m);
    }
    // "WAITING" must not be read as "WAIT".
    expect(MARKER_RE.exec("WAITING x")?.[1]).toBe("WAITING");
    // A non-marker word isn't matched.
    expect(MARKER_RE.exec("TODOLIST x")).toBeNull();
  });

  it("does NOT match a marker word followed by punctuation — audit C2 (carry overmatch)", () => {
    // `\b` matched these (lsdoc marks none); the carry `isOpenTask` would move non-task
    // prose. The marker must be followed by whitespace or end-of-line.
    expect(MARKER_RE.exec("TODO: not a task")).toBeNull();
    expect(MARKER_RE.exec("DONE. a sentence")).toBeNull();
    expect(MARKER_RE.exec("WAIT-LIST item")).toBeNull();
    expect(MARKER_RE.exec("TODO real task")?.[1]).toBe("TODO");
    expect(MARKER_RE.exec("DONE")?.[1]).toBe("DONE"); // bare marker at end-of-line
  });
});
