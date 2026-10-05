// C5 B (blockParts first line + TabBar summary): both labels read the block as lsdoc does (I-12)
// instead of regexes over markup and `key:: value` text.
import { beforeAll, describe, expect, it } from "vitest";
import { initParser } from "../render/parse";
import { blockSummary } from "./TabBar";
import { blockFirstLine } from "./blockParts";

beforeAll(initParser);

describe("zoomed-tab summary", () => {
  it("shows link labels, page names, emphasis text, code text and the heading title", () => {
    expect(blockSummary("## **Bold** [site](http://x.test) [[Some Page]] `code`", "md", false)).toBe("Bold site Some Page code");
  });
  it("a literal-looking marker run inside a link label is not stripped as emphasis", () => {
    expect(blockSummary("[a_b_c](http://x.test) snake_case_name", "md", false)).toBe("a_b_c snake_case_name");
  });
  it("keeps a task marker and skips hidden id lines", () => {
    expect(blockSummary("id:: 6f1c0000-0000-4000-8000-000000000000\nTODO write [[Plan]]", "md", false)).toBe("TODO write Plan");
  });
});

describe("block-ref picker label", () => {
  it("skips an Org property drawer, whose lines no `key:: value` regex recognises", () => {
    expect(blockFirstLine(":PROPERTIES:\n:ID: abc\n:END:\nTitle line", "org")).toBe("Title line");
  });
  it("skips Markdown property lines", () => {
    expect(blockFirstLine("alias:: x\nText")).toBe("Text");
  });
});
