import { beforeAll, describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { initParser } from "./render/parse";
import { clearSeededFacets, facetsOf } from "./render/facets";
import { MARKERS, OPEN_MARKERS, DONE_MARKERS, matchLeadingMarker, leadingMarker, taskCheckboxState } from "./markers";
import { cycleMarker, leadingMarker as leadingMarkerViaEditor, setMarker } from "./editor/marker";
import { setPriority } from "./editor/format";

describe("task markers (single source of truth)", () => {
  it("matches the backend set (crates/tine-core/src/doc.rs MARKERS) — keep in sync", () => {
    // Read the Rust source itself (I-12): a literal copied into this test could
    // not notice doc.rs changing. This set must equal lsdoc's recognizer
    // (lsdoc/src/parse.rs MARKERS, the mldoc/OG-faithful authority) — Tine
    // treats exactly what OG treats as a task.
    const source = readFileSync("crates/tine-core/src/doc.rs", "utf8");
    const block = /pub const MARKERS: &\[&str\] = &\[([^\]]*)\];/.exec(source)?.[1] ?? "";
    const rust = [...block.matchAll(/"([^"]+)"/g)].map((m) => m[1]);
    expect(rust.length).toBeGreaterThan(0);
    expect([...MARKERS].sort()).toEqual(rust.sort());
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

  it("finished markers match Rust so bare (task) and carry-over cannot drift", () => {
    const source = readFileSync("crates/tine-core/src/doc.rs", "utf8");
    const block = /pub const DONE_MARKERS: &\[&str\] = &\[([^\]]*)\];/.exec(source)?.[1];
    expect(block).toBeDefined();
    const rust = [...block!.matchAll(/"([^"]+)"/g)].map((m) => m[1]);
    expect([...DONE_MARKERS].sort()).toEqual(rust.sort());
    expect([...OPEN_MARKERS].sort()).toEqual(MARKERS.filter((m) => !rust.includes(m)).sort());
  });

  it("taskCheckboxState: DONE checked, open markers unchecked, canceled/none none (OG block-checkbox)", () => {
    expect(taskCheckboxState("DONE")).toBe(true);
    for (const m of ["TODO", "DOING", "NOW", "LATER", "WAITING", "WAIT", "STARTED", "IN-PROGRESS"]) {
      expect(taskCheckboxState(m)).toBe(false); // open task → unchecked box
    }
    expect(taskCheckboxState("CANCELED")).toBeNull(); // closed-but-not-done → no box (OG)
    expect(taskCheckboxState("CANCELLED")).toBeNull();
    expect(taskCheckboxState("NOT-A-MARKER")).toBeNull(); // Rust render_facets::task_checkbox_state, same table
    expect(taskCheckboxState(null)).toBeNull();
    expect(taskCheckboxState(undefined)).toBeNull();
  });

  it("the one recognizer anchors every marker as a whole word, prefix-safe (WAITING vs WAIT)", () => {
    for (const m of MARKERS) {
      expect(matchLeadingMarker(`${m} do the thing`)?.marker).toBe(m);
      expect(leadingMarker(`${m} do the thing`)).toBe(m);
    }
    // "WAITING" must not be read as "WAIT".
    expect(matchLeadingMarker("WAITING x")?.marker).toBe("WAITING");
    // A non-marker word isn't matched.
    expect(matchLeadingMarker("TODOLIST x")).toBeNull();
  });

  it("does NOT match a marker word followed by punctuation — audit C2 (carry overmatch)", () => {
    // `\b` matched these (lsdoc marks none); the carry `isOpenTask` would move non-task
    // prose. The marker must be followed by whitespace or end-of-line.
    expect(matchLeadingMarker("TODO: not a task")).toBeNull();
    expect(matchLeadingMarker("DONE. a sentence")).toBeNull();
    expect(matchLeadingMarker("WAIT-LIST item")).toBeNull();
    expect(matchLeadingMarker("TODO real task")?.marker).toBe("TODO");
    expect(matchLeadingMarker("DONE")?.marker).toBe("DONE"); // bare marker at end-of-input
  });

  // lsdoc parity (DUP-7, 2026-08-25 duplication audit). Every case is
  // byte-verified against the vendored lsdoc v0.5.5 wasm (`parse_block_json`,
  // the SAME one-block boundary the rendered checkbox derives from): leading
  // whitespace is skipped, then the marker needs a LITERAL ASCII space or must
  // be the entire rest of the input (`marker_eof`).
  describe("lsdoc marker parity (DUP-7)", () => {
    const cases: [string, string | null][] = [
      ["TODO x", "TODO"],
      ["TODO\tx", null], // a tab after the marker is NOT a marker (chip absent)
      ["TODO", "TODO"], // bare marker only at absolute end of input
      ["TODO\nbody line", null], // bare marker with a continuation line: NO marker
      ["TODO\n", null], // trailing newline is not end of input
      ["TODO \nbody", "TODO"], // marker with an empty (space-only) title
      ["\nTODO x", "TODO"], // the boundary trim skips leading newlines too
      ["  TODO x", "TODO"], // leading space skipped (trim_start)
      ["\tTODO x", "TODO"], // leading tab skipped
      ["\fTODO x", "TODO"], // form feed is an lsdoc parser space
      ["TODO  x", "TODO"],
      ["todo x", null], // case-sensitive
      ["DONE", "DONE"],
      ["WAITING x", "WAITING"], // prefix-safe: not WAIT
      ["TODOLIST x", null],
      ["TODO: not a task", null],
    ];
    it.each(cases)("matchLeadingMarker %j -> %s", (raw, want) => {
      expect(matchLeadingMarker(raw)?.marker ?? null).toBe(want);
      // leadingMarker is the same recognizer's name-only view, twice over.
      expect(leadingMarker(raw)).toBe(want);
      expect(leadingMarkerViaEditor(raw)).toBe(want);
    });

    // The table above is a claim about lsdoc; hold it against the wasm this
    // build actually ships (the renderer's own marker projection).
    beforeAll(() => initParser());
    it.each(cases)("vendored lsdoc agrees: %j -> %s", (raw, want) => {
      clearSeededFacets();
      expect(facetsOf(raw, "md").marker).toBe(want);
    });

    it("exposes splice-safe marker offsets (start/end) for editor splices", () => {
      expect(matchLeadingMarker("TODO x")).toEqual({ marker: "TODO", start: 0, end: 4 });
      expect(matchLeadingMarker("  TODO x")).toEqual({ marker: "TODO", start: 2, end: 6 });
      // Offsets of an indented marker point at the marker itself, so cycling
      // preserves the indent instead of prepending a second marker.
      expect(matchLeadingMarker("\n\nWAITING x")?.start).toBe(2);
      expect(matchLeadingMarker("\n\nWAITING x")?.end).toBe(9);
    });
  });

  // DUP-7 consumers splice at the recognized offsets: an indented or
  // blank-line-led task gets its marker replaced, never a second one prepended,
  // and a tab-separated word that renders no checkbox is not treated as a task.
  it("marker writers splice at the shared recognizer's offsets", () => {
    expect(cycleMarker("  TODO x", "todo").raw).toBe("  DOING x");
    expect(setMarker("\nTODO x", "DONE")).toBe("\nDONE x");
    expect(setMarker("TODO\tx", "DONE")).toBe("DONE TODO\tx");
    expect(setPriority("TODO\tx", "A")).toBe("[#A] TODO\tx");
  });
});
