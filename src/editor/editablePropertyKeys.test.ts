// og 14 Q5 follow-up (G3 blocker): the properties panel must never write a key
// that ANY Tine property reader would not find again. The accepted/rejected
// lists are a fixture shared with the Rust readers' test
// (crates/tine-core/src/logbook.rs `editable_property_keys_fixture`), so a
// drift on either side fails a test instead of shipping an invisible property.
import { readFileSync } from "node:fs";
import { beforeAll, describe, expect, it } from "vitest";
import { acceptedPropertyLine, isEditablePropertyKey, pagePropertyEntries, splitProps } from "./properties";
import { initParser } from "../render/parse";
import { facetsOf } from "../render/facets";

const FIXTURE = "crates/tine-core/tests/fixtures/editable-property-keys.txt";
const entries = readFileSync(FIXTURE, "utf8").split("\n").filter((line) => line !== "" && !line.startsWith("#"));
const accepted = entries.filter((line) => line.startsWith("+")).map((line) => line.slice(1));
const rejected = entries.filter((line) => line.startsWith("-")).map((line) => line.slice(1));

beforeAll(async () => {
  await initParser();
});

describe("isEditablePropertyKey = the intersection of every Tine reader (shared fixture)", () => {
  it("accepts exactly the fixture's accepted keys and refuses its rejected keys", () => {
    expect(accepted.length).toBeGreaterThan(5);
    expect(accepted).toContain("klíč");
    for (const key of accepted) expect(isEditablePropertyKey(key), key).toBe(true);
    for (const key of rejected) expect(isEditablePropertyKey(key), JSON.stringify(key)).toBe(false);
  });

  it("every accepted key reads back through every TypeScript reader, both formats", () => {
    for (const key of accepted) {
      const lower = key.toLowerCase();
      expect(acceptedPropertyLine(`${key}:: v`)?.key, key).toBe(key);
      expect(pagePropertyEntries(`${key}:: v`, "md").map((e) => [e.key, e.value]), key).toEqual([[key, "v"]]);
      expect(pagePropertyEntries(`#+${key}: v`, "org").map((e) => [e.key, e.value]), key).toEqual([[lower, "v"]]);
      expect(pagePropertyEntries(`:PROPERTIES:\n:${key}: v\n:END:`, "org").map((e) => [e.key, e.value]), key).toEqual([[lower, "v"]]);
      expect(facetsOf(`title\n${key}:: v`, "md").properties, key).toEqual([[key, "v"]]);
      expect(facetsOf(`title\n:PROPERTIES:\n:${key}: v\n:END:`, "org").properties.map(([k, v]) => [k.toLowerCase(), v]), key).toEqual([[lower, "v"]]);
      // The Org block editor's drawer recognizer hides exactly this key.
      expect(splitProps(`title\n:PROPERTIES:\n:${key}: v\n:END:`, (k) => k === lower, "org").hidden, key).toBe(`:${key}: v`);
    }
  });

  it("over printable ASCII, a key is editable iff each character is in the shared page-header class", () => {
    // The Rust test pins that accepted keys pass parse_property_line and logbook.
    for (let code = 0x21; code <= 0x7e; code++) {
      const c = String.fromCharCode(code);
      expect(isEditablePropertyKey(`a${c}b`), c).toBe(/[A-Za-z0-9_./-]/.test(c));
    }
  });
});
