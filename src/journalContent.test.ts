import { expect, it } from "vitest";
import { journalHasContent } from "./journalContent";

const block = (raw: string, children: { raw: string; children: never[] }[] = []) => ({ raw, children });

it("OG-C5 L12-S1: any written text, whatever its shape, keeps the template away", () => {
  expect(journalHasContent([])).toBe(false);
  expect(journalHasContent([block(""), block(" \t\n", [block("")])])).toBe(false);
  expect(journalHasContent([block("memo:: keep this sentence")])).toBe(true); // Org heading prose
  expect(journalHasContent([block("id:: 6679f1c2-0000-4000-8000-000000000001")])).toBe(true);
  expect(journalHasContent([block("", [block("note:: kept")])])).toBe(true);
  expect(journalHasContent([block("#tag:: prose")])).toBe(true);
});
