import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("the bundled Guide explains safe capture retries and conflict recovery", () => {
  const guide = (page: string) => readFileSync(`crates/tine-core/src/templates/${page}.md`, "utf8");
  expect(guide("quick-capture")).toContain("without adding it twice");
  const recovery = guide("troubleshooting-recovery");
  for (const phrase of ["A failed delete keeps your unsaved text", "logseq/.tine-trash/conflicts/", "keep the external version"]) expect(recovery).toContain(phrase);
});
