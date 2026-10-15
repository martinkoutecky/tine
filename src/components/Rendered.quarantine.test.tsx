import { beforeAll, expect, it, vi } from "vitest";
vi.mock("../render/wasm/lsdoc_wasm.js", async original => ({
  ...await original<object>(),
  parse_block_bundle_json: vi.fn(() => { throw new Error("persistent parser trap"); }),
  __tineReinstantiate: vi.fn(),
}));
import { initParser, isQuarantined, parseBlock, soleBlockMacro } from "../render/parse";
import { detectMacro } from "./Rendered";
beforeAll(initParser);
it.each(["md", "org"] as const)("keeps a quarantined option-bearing query literal in %s", format => {
  const raw = '{{query (task TODO) {:title "Quarantined"}}}';
  expect(isQuarantined(parseBlock(raw, format === "org"))).toBe(true);
  expect(soleBlockMacro(raw, format)).toBeNull();
  expect(detectMacro(raw, format)).toBeNull();
});
