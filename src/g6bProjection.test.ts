import { describe, it } from "vitest";
import fs from "node:fs";
import { loadSingle, resetStore } from "./document/workingSet";
import { setRaw } from "./document";
import { pageToDto } from "./document/convert";
import type { BlockDto, PageDto } from "./types";

interface Entry { id: string; doc?: PageDto; cause?: string; span_lines?: number }

describe("G6b private graph frontend projection", () => {
  it.skipIf(!process.env.G6B_DUMP || !process.env.G6B_EDITS)("edits one leaf on every readable page through the app model", () => {
    const input = fs.readFileSync(process.env.G6B_DUMP!, "utf8").trimEnd().split("\n");
    const output: Entry[] = [];
    for (const line of input) {
      const entry = JSON.parse(line) as Entry;
      if (!entry.doc) { output.push(entry); continue; }
      resetStore();
      loadSingle(entry.doc);
      let leaf: BlockDto | undefined;
      let emptyLeaf: BlockDto | undefined;
      let propertyLeaf: BlockDto | undefined;
      const visit = (blocks: BlockDto[]) => {
        for (const block of blocks) {
          if (block.children.length) visit(block.children);
          else {
            const first = block.raw.split("\n")[0];
            if (!first) emptyLeaf ??= block;
            else if (/^[\p{L}\p{M}\p{N}_./-]+::/u.test(first)) propertyLeaf ??= block;
            else leaf ??= block;
          }
        }
      };
      visit(entry.doc.blocks);
      leaf ??= emptyLeaf;
      if (!leaf) { output.push({ id: entry.id, cause: propertyLeaf ? "property-only-leaf" : "no-leaf-block" }); continue; }
      const first = leaf.raw.split("\n")[0];
      setRaw(leaf.id, first + (first ? " " : "") + "__TINE_G6B_06D__" + leaf.raw.slice(first.length), { timetracking: false });
      const projected = pageToDto(entry.doc.name);
      output.push(projected ? { id: entry.id, doc: projected, span_lines: leaf.raw.split("\n").length }
        : { id: entry.id, cause: "projection-refused" });
    }
    resetStore();
    fs.writeFileSync(process.env.G6B_EDITS!, output.map((entry) => JSON.stringify(entry)).join("\n") + "\n");
  });
});
