import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { journalHasContent } from "./journalContent";

it("I-12 journal content matches the shared Rust fixture; exemplar crates/tine-store/src/model.rs doc_has_content", () => {
  const cases = JSON.parse(readFileSync("tests/fixtures/journal-content.json", "utf8"));
  for (const item of cases) expect(journalHasContent(item.blocks), item.name).toBe(item.hasContent);
});
