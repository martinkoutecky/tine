import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("I-12: feature file formats use tine_core::model::Format::from_path, as store page_parse does", () => {
  for (const name of ["conflicts", "pdf", "pages", "print", "parsed_text"]) {
    let source = readFileSync(`crates/tine-graph-features/src/${name}.rs`, "utf8");
    // The derived conflict queue is a child module of conflicts.rs; scan both.
    if (name === "conflicts") source += readFileSync("crates/tine-graph-features/src/conflicts/queue.rs", "utf8");
    // print.rs reads no file and decides no format: Store::page and DocBlock::is_org carry it.
    if (name !== "print") {
      expect(source, `I-12: ${name} must use Format::from_path, never a case-sensitive suffix twin`).toContain("Format::from_path(");
    }
    expect(source).not.toMatch(/ends_with\("\.org"\)/);
  }
});
