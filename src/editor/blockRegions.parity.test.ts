import { beforeAll, expect, it } from "vitest";
import fixtures from "../../crates/tine-core/tests/fixtures/block-regions.json";
import native from "../../crates/tine-core/tests/fixtures/block-regions-native.json";
import { initParser, blockRegions, parseBlock, editBlock } from "../render/parse";
beforeAll(() => initParser());
it("native and vendored wasm return identical raw byte regions", () => {
  expect(fixtures).toHaveLength(native.length);
  fixtures.forEach((f,i) => expect(blockRegions(f.raw,f.org ? "org" : "md"),`fixture ${i}`).toEqual(native[i]));
});
it("a warm render parse serves region operations without another parse", () => {
  const raw="Task\n```\nid:: literal\n```\nid:: real";
  const ast=parseBlock(raw,false);
  expect(editBlock(raw,"md",{kind:"strip_copy",template:false})).toBe("Task\n```\nid:: literal\n```");
  expect(parseBlock(raw,false)).toBe(ast);
});
