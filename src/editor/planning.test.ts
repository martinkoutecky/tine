import { describe, it, expect, beforeAll } from "vitest";
import { initParser } from "../render/parse";
import { normalizePlanning } from "./planning";

// normalizePlanning gates on lsdoc (real Timestamp), so the wasm parser must be up.
beforeAll(async () => {
  await initParser();
});

describe("normalizePlanning (M1c: move SCHEDULED/DEADLINE to canonical position on exit)", () => {
  it("moves a planning line typed at the end to right after the first line", () => {
    const v = "TODO ship it\nsome note\nSCHEDULED: <2026-07-06 Mon>";
    expect(normalizePlanning(v, "md")).toBe("TODO ship it\nSCHEDULED: <2026-07-06 Mon>\nsome note");
  });

  it("orders SCHEDULED before DEADLINE", () => {
    const v = "TODO ship it\nDEADLINE: <2026-07-10 Fri>\nSCHEDULED: <2026-07-06 Mon>";
    expect(normalizePlanning(v, "md")).toBe(
      "TODO ship it\nSCHEDULED: <2026-07-06 Mon>\nDEADLINE: <2026-07-10 Fri>"
    );
  });

  it("is a no-op when planning is already in the canonical position", () => {
    const v = "TODO ship it\nSCHEDULED: <2026-07-06 Mon>\nbody";
    expect(normalizePlanning(v, "md")).toBe(v);
  });

  it("moves planning BEFORE a trailing user property line", () => {
    const v = "TODO ship it\nauthor:: martin\nSCHEDULED: <2026-07-06 Mon>";
    expect(normalizePlanning(v, "md")).toBe(
      "TODO ship it\nSCHEDULED: <2026-07-06 Mon>\nauthor:: martin"
    );
  });

  it("does NOT move a SCHEDULED inside a fenced code block (it is content)", () => {
    const v = "TODO ship it\n```\nSCHEDULED: <2026-07-06 Mon>\n```";
    // lsdoc parses the fenced line as code, not a Timestamp → no real planning → unchanged.
    expect(normalizePlanning(v, "md")).toBe(v);
  });

  it("does not treat a shorter run as closing a longer fence", () => {
    const v = [
      "Task",
      "SCHEDULED: <2026-07-11 Sat>",
      "````text",
      "```",
      "DEADLINE: <2026-07-12 Sun>",
      "````",
      "body",
    ].join("\n");
    expect(normalizePlanning(v, "md")).toBe(v);
  });

  it("does NOT move an inline-code DEADLINE (not a standalone Timestamp)", () => {
    const v = "see `DEADLINE: <2026-07-06 Mon>` here\nmore text";
    expect(normalizePlanning(v, "md")).toBe(v);
  });

  it("leaves a non-planning block untouched (cheap path)", () => {
    const v = "just some\nmultiline text";
    expect(normalizePlanning(v, "md")).toBe(v);
  });

  it("leaves a single-line block untouched", () => {
    expect(normalizePlanning("SCHEDULED: <2026-07-06 Mon>", "md")).toBe("SCHEDULED: <2026-07-06 Mon>");
  });
});
