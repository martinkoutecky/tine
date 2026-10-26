import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { EDIT_KIND_VALUES } from "./editKind";
import { markDirty } from "./document";

it("pins the eight edit kinds to the shared fixture (OG-RULES Rule 8)", () => {
  const fixture = JSON.parse(readFileSync("edit-kinds.json", "utf8")) as string[];
  expect(new Set(EDIT_KIND_VALUES)).toEqual(new Set(fixture));
  expect(EDIT_KIND_VALUES.length, "a new edit kind needs an ADR arguing why the existing kinds cannot express it, plus Martin's approval (OG-RULES Rule 8); exemplar src/document/host/wiring.ts markDirty").toBe(8);
});

it("requires a kind at the dirty boundary", () => {
  if (false) {
    // @ts-expect-error OG-RULES Rule 8: a dirty page must declare an edit kind.
    markDirty("Page");
  }
});
