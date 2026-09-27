import { describe, expect, it } from "vitest";
import fs from "node:fs";
import { loadSingle, resetStore } from "./document/workingSet";
import { setRaw } from "./document";
import { pageToDto } from "./document/convert";
import type { PageDto } from "./types";

describe("G6b synthetic fixture projection", () => {
  it("keeps the loaded sibling's trailing spaces while trimming the edited block", () => {
    const fixture = fs.readFileSync(new URL("../scripts/fixtures/g6b/graph/pages/Trailing.md", import.meta.url), "utf8");
    const lines = fixture.replace(/\n$/, "").split("\n");
    // The fixture's second block has trailing whitespace that must survive a
    // save of the first block. Use the file's raw lines as the loaded DTO.
    const dto: PageDto = {
      name: "Trailing", kind: "page", title: "Trailing", pre_block: null,
      blocks: lines.map((line, i) => ({ id: `g6b-${i}`, raw: line.slice(2), collapsed: false, children: [] })),
    };
    resetStore();
    loadSingle(dto);
    setRaw("g6b-0", dto.blocks[0].raw + " edited  ", { timetracking: false });
    const projected = pageToDto(dto.name)!;
    expect(projected.blocks[0].raw).toBe(dto.blocks[0].raw + " edited");
    expect(projected.blocks[1].raw).toBe(dto.blocks[1].raw);
    resetStore();
  });
});
