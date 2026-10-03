// I-2 (one bad input must not poison the session): the vendored lsdoc wasm is built with
// panic = "abort", so a Rust panic is an `unreachable` trap. scripts/build-wasm.mjs wraps
// EVERY export so a trap reinstantiates a fresh instance and surfaces as an ordinary Error
// carrying the panic message, instead of leaving later calls on a poisoned instance (the og
// "Unreachable code should not be executed (evaluating 'page_header_json')" report).
// Before the wrapper only parse_block_bundle_json / header_tokens_json recovered; ~20 doors,
// including page_header_json, did not. Exemplar: src/render/parseReinstantiate.test.tsx.
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { initParser } from "./parse";
import { format_journal_date, last_panic, page_header_json, parse_block_json, search_query_json } from "./wasm/lsdoc_wasm.js";

// lsdoc refuses a quote staircase deeper than 1024 with a deliberate panic (admit_block_source)
// on every block door: a deterministic trap that does not depend on an lsdoc ownership gap.
const TRAPPING = ">".repeat(1100) + " x";

beforeAll(async () => {
  await initParser();
});
afterEach(() => vi.restoreAllMocks());

describe("a wasm trap is isolated to the call that caused it", () => {
  it("surfaces as an Error with the panic message and leaves the instance usable", () => {
    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    expect(() => parse_block_json(TRAPPING, false)).toThrow(/lsdoc-wasm trap in parse_block_json: [\s\S]*quote nesting/);
    // The hook logged the message before the abort, and the glue captured it.
    expect(logged.mock.calls.some(([m]) => String(m).includes("quote nesting"))).toBe(true);
    // I-5: only the location and a literal message, never page text.
    expect(logged.mock.calls.every(([m]) => !String(m).includes(">>>"))).toBe(true);
    // Other doors work on the fresh instance, including ones that never had their own recovery.
    expect(JSON.parse(page_header_json("type:: book\n")).entries[0].key).toBe("type");
    expect(JSON.parse(search_query_json("foo", true))).toBeTruthy();
    // The new instance starts with no panic recorded.
    expect(last_panic()).toBe("");
  });

  it("names the panic site but withholds formatted messages (I-5)", () => {
    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    // An lsdoc v2 ownership gap: `parse_format` panics with a formatted message. Minimal input.
    expect(() => parse_block_json("s::\r", false)).toThrow(/panic at \S*v2\/mod\.rs:\d+: <formatted message withheld>/);
    expect(logged).toHaveBeenCalledTimes(1);
  });

  it("survives more traps than a leaked shadow stack could (no cumulative poisoning)", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    // Without reinstantiation the instance broke ("memory access out of bounds") after ~1800
    // traps of a shallow panic; deeper panics leak faster.
    for (let i = 0; i < 2500; i++) expect(() => parse_block_json(TRAPPING, false)).toThrow(/lsdoc-wasm trap/);
    expect(JSON.parse(page_header_json("type:: book\n")).entries[0].key).toBe("type");
  });

  it("an ordinary exception is not mistaken for a trap", () => {
    // A BigInt where wasm-bindgen expects a u32 is rejected at the JS/wasm boundary with a TypeError,
    // before any Rust runs: not a trap, so it propagates untouched and nothing is reinstantiated.
    expect(() => format_journal_date(1n as unknown as number, 1, 1, "yyyy")).toThrow(TypeError);
    expect(JSON.parse(page_header_json("type:: book\n")).end).toBeGreaterThan(0);
  });
});
