import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { extractRefs } from "./vendor/refs.mjs";

interface MldocApi {
  parseJson(input: string, config: string): string;
}

// mldoc is a js_of_ocaml IIFE rather than an ESM module. Vite's test transform
// can discard its global side effect, so execute the exact vendored bytes in the
// Node test realm just as a browser script tag would.
Function(readFileSync(new URL("./vendor/mldoc.js", import.meta.url), "utf8"))();
const mldoc = (globalThis as typeof globalThis & { Mldoc: MldocApi }).Mldoc;

function config(format: "md" | "org"): string {
  return JSON.stringify({
    toc: false,
    parse_outline_only: false,
    heading_number: false,
    keep_line_break: true,
    format: format === "org" ? "Org" : "Markdown",
    heading_to_list: false,
    export_md_remove_options: [],
  });
}

function ast(input: string, format: "md" | "org"): unknown {
  return JSON.parse(mldoc.parseJson(input, config(format))) as unknown;
}

function refs(input: string, format: "md" | "org" = "md") {
  const parsed = ast(input, format);
  return extractRefs(parsed, format);
}

describe("vendored mldoc reference oracle", () => {
  it.each([
    ["k:: [label](foo)", ["foo"]],
    ["k:: [file.ext](../assets/file.ext)", ["../assets/file.ext"]],
    ["k:: [[outer [[Inner]]]]", ["outer [[Inner]]"]],
    ["k:: [Some](file:../x.md)", []],
  ] as const)("matches OG property refs for %s", (input, page) => {
    expect(refs(input)).toEqual({ page: [...page], block: [] });
  });

  it("threads the format through for Org search-link semantics", () => {
    const parsed = ast("[[foo][Example]]", "org");
    expect(extractRefs(parsed, "org").page).toEqual(["foo"]);
    expect(extractRefs(parsed, "md").page).toEqual([]);
  });
});
