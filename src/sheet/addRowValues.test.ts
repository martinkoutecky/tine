import { describe, expect, it } from "vitest";
import { editBlock } from "../render/parse";
import { facetsOf } from "../render/facets";
import { newBlockRawWithProperties } from "../document";

// tine#1047: the values a table's Add row writes on its new row, built in one
// pass. Each key is kept only when it reads back as an ordinary property.
const written = (keys: string[], format: "md" | "org") => {
  const raw = newBlockRawWithProperties(keys, "false", format);
  return { raw, props: raw ? facetsOf(raw, format).properties : [] };
};

describe("newBlockRawWithProperties", () => {
  it.each(["md", "org"] as const)("writes what one property edit per key writes (%s)", (format) => {
    const keys = ["prop", "shipped", "done"];
    const chained = keys.reduce((raw, key) => editBlock(raw, format, { kind: "property", key, value: "false" }), "\n");
    expect(written(keys, format).raw).toBe(chained);
  });

  it.each(["md", "org"] as const)("never writes the block's identity, in any case (%s)", (format) => {
    for (const id of ["id", "Id", "ID"]) {
      expect(written([id, "prop"], format).props).toEqual([["prop", "false"]]);
      expect(written(["prop", id], format).props).toEqual([["prop", "false"]]);
    }
    expect(written(["id"], format).raw).toBe("");
  });

  it.each(["md", "org"] as const)("writes case-insensitive duplicates once, under the first spelling (%s)", (format) => {
    expect(written(["Done", "done"], format).props).toEqual([["Done", "false"]]);
    expect(written(["done", "Done", "x"], format).props).toEqual([["done", "false"], ["x", "false"]]);
  });

  it("keeps the Org drawer whole when a key is `end`, first or not", () => {
    for (const keys of [["end", "a", "b"], ["a", "END", "b"], ["a", "b", "End"]]) {
      const { raw, props } = written(keys, "org");
      expect(raw).toBe("\n:PROPERTIES:\n:a: false\n:b: false\n:END:");
      expect(props).toEqual([["a", "false"], ["b", "false"]]);
    }
  });

  it("keeps `end` in Markdown, where it is an ordinary key", () => {
    expect(written(["end", "a"], "md").props).toEqual([["end", "false"], ["a", "false"]]);
  });

  it.each(["md", "org"] as const)("drops keys with a colon or a space and writes the others (%s)", (format) => {
    for (const keys of [["a:b", "x", "c d"], ["c d", "x", "a:b"]]) {
      expect(written(keys, format).props).toEqual([["x", "false"]]);
    }
    expect(written(["a:b", "c d"], format).raw).toBe("");
  });
});
