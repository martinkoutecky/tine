import { expect, it } from "vitest";
import blocks from "../crates/tine-core/tests/fixtures/block-facets.json";
import outline from "../crates/tine-outline/tests/fixtures/ops/m1.json";
import { loadBrowserCore } from "./browserCore";
import { initParser } from "./render/parse";
import * as parser from "./render/wasm/lsdoc_wasm.js";

it("the parser-only and browser core builds agree on the shared parser fixtures", async () => {
  const loaded = loadBrowserCore();
  expect(loadBrowserCore()).toBe(loaded);
  const [core] = await Promise.all([loaded, initParser()]);
  expect(core.BrowserGraph).toBeTypeOf("function");
  expect(Object.keys(parser).sort()).toEqual(Object.keys(core).filter((name) => name !== "BrowserGraph").sort());

  for (const { raw, format } of blocks) {
    const org = format === "org";
    expect(core.parse_block_bundle_json(raw, org, false), raw).toBe(parser.parse_block_bundle_json(raw, org, false));
    expect(core.parse_inline_json(raw, org), raw).toBe(parser.parse_inline_json(raw, org));
  }
  for (const { name, verb } of outline.cases) {
    const request = JSON.stringify(verb);
    expect(core.outline_reads_json(request), name).toBe(parser.outline_reads_json(request));
  }
});
