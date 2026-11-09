// I-2 (one bad input must not poison the session): the vendored lsdoc wasm is built with
// panic = "abort", so a Rust panic is an `unreachable` trap. scripts/build-wasm.mjs wraps
// EVERY export so a trap reinstantiates a fresh instance and surfaces as an ordinary Error
// carrying the panic message, instead of leaving later calls on a poisoned instance (the og
// "Unreachable code should not be executed (evaluating 'page_header_json')" report).
// Before the wrapper only parse_block_bundle_json / header_tokens_json recovered; ~20 doors,
// including page_header_json, did not. Exemplar: src/render/parseReinstantiate.test.tsx.
import { afterEach, beforeAll, describe, expect, it, onTestFinished, vi } from "vitest";
import { initParser } from "./parse";
import { editor_text_json, format_journal_date, last_panic, outline_apply_json, page_header_json, parse_block_json, search_query_json } from "./wasm/lsdoc_wasm.js";

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

  it("parses the bare-CR inputs that used to trap (lsdoc v0.5.8 owns them)", () => {
    // These were lsdoc v2 ownership gaps (`parse_format` panicked with a formatted message).
    // The hook's I-5 withholding of formatted messages is unit-tested in
    // crates/tine-core/src/wasm_panic_report.rs, since no formatted panic is reachable now.
    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    for (const raw of ["s::\r", "- s::\r", "- tags:: x\rid:: ::}}"]) {
      expect(() => parse_block_json(raw, false), JSON.stringify(raw)).not.toThrow();
      expect(() => page_header_json(raw), JSON.stringify(raw)).not.toThrow();
    }
    expect(logged).not.toHaveBeenCalled();
    expect(last_panic()).toBe("");
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
    // before any Rust runs: not a trap, so it propagates untouched, as the caller's own error.
    expect(() => format_journal_date(1n as unknown as number, 1, 1, "yyyy")).toThrow(TypeError);
    expect(JSON.parse(page_header_json("type:: book\n")).end).toBeGreaterThan(0);
  });

  it("an exception thrown from inside a wasm call cannot poison later calls (og-wasm-enter-trap)", () => {
    // `regions` without its arrays makes regions_from_js call Array.from(undefined): the TypeError
    // is thrown inside the wasm call and skips the Rust epilogues, leaking the frames it unwound
    // (about 0.9 KB of the 1 MiB shadow stack) and the externref slots they held. Without a fresh
    // instance the 1024th such call trapped ("table index is out of bounds"), and a deeper leak
    // ends with an ordinary Enter running off the stack ("Out of bounds memory access").
    const raw = "the last block of the third page";
    const enter = JSON.stringify({
      pages: [{ name: "p3", format: "md", pre_block: null, roots: ["a"], blocks: { a: { raw, children: [] } },
        header_root: null, read_only: false, frozen: false }],
      verb: { verb: "split", key: "a", raw_offset: raw.length, force_child: false, children_visible: false, keep_start_in_scope: false },
      keys: { prefix: "k-", next: 0 }, hide: { kind: "builtin" },
    });
    for (let i = 0; i < 1100; i++) {
      // The caller keeps its own error every time, never a later trap.
      expect(() => editor_text_json(raw, false, undefined, '{"kind":"builtin"}')).toThrow(TypeError);
    }
    const split = JSON.parse(outline_apply_json(enter));
    expect(split.ok.pages[0].upserts.map((u: { raw: string }) => u.raw)).toContain(raw);
  });

  it("an unwound exception or a trap costs exactly one fresh instance; an ordinary call or a Rust Err costs none", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    // Count the instances the glue builds. A fresh instance is the recovery's whole cost: it is
    // taken once per failed call, never on a call that went well.
    const Real = WebAssembly.Instance;
    let made = 0;
    WebAssembly.Instance = class extends Real { constructor(...args: ConstructorParameters<typeof Real>) { made++; super(...args); } } as typeof Real;
    onTestFinished(() => { WebAssembly.Instance = Real; });
    const raw = "the last block of the third page";
    for (let i = 0; i < 200; i++) page_header_json("type:: book\n");
    expect(made).toBe(0);
    // A Rust `Err` reaches JS as a string after the call returned: the instance is intact.
    expect(() => outline_apply_json("{not a request")).toThrow();
    expect(made).toBe(0);
    // One exception thrown from inside a call: one fresh instance, and the caller's own error.
    expect(() => editor_text_json(raw, false, undefined, '{"kind":"builtin"}')).toThrow(TypeError);
    expect(made).toBe(1);
    // A trap: one fresh instance (the panic message is read before it, then it is replaced once).
    expect(() => parse_block_json(TRAPPING, false)).toThrow(/lsdoc-wasm trap/);
    expect(made).toBe(2);
    // And the next ordinary calls cost none.
    for (let i = 0; i < 200; i++) page_header_json("type:: book\n");
    expect(made).toBe(2);
  });

  it("a trap names how the instance got there, without page text", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    let message = "";
    try { parse_block_json(TRAPPING, false); } catch (e) { message = String((e as Error).message); }
    // The failing door, the instance's calls before the trap and the argument shape: enough to
    // tell an input that is too deep from state left by earlier calls (I-5: no page text).
    expect(message).toMatch(/lsdoc-wasm trap in parse_block_json: .*\[instance: \d+ calls, \d+ resets, \d+ pages; args string:1102, boolean\]/);
    expect(message).not.toContain(">>>");
  });
});
