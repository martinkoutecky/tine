// R4 / I-20 (og-flow3 finding 2): a durable write's owner is the graph binding
// (store reset + backend binding generation), never the display epoch. A repaint
// (typography, journal title format, a rename of another page) bumps
// `graphEpoch` on the same graph; an owner that includes it retires a write
// whose bytes still land, dropping its success bookkeeping (base revision) or
// its failure report (dirty + toast). Exemplars: `bindingOwner` (src/owned.ts)
// and `bindingCurrent` (src/binding.ts).
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { bindingOwner, graphOwner, serializeDurable, serializeOwned, writeOwned } from "./owned";

const RULE = "R4/I-20: a write's owner must not include the display epoch; use bindingOwner/bindingCurrent (exemplar src/owned.ts bindingOwner)";

/** Files that may still call `stillBound` (binding + display epoch). Each is a
 *  display/navigation continuation whose refusal is visible and writes nothing. */
const DISPLAY_ONLY: Record<string, string> = {
  "src/binding.ts": "defines stillBound; graphScopedSignal reads popup targets",
  "src/owned.ts": "graphOwner: the display owner for render/read results",
  "src/inpageFind.ts": "find-in-page highlight/reveal; display only",
  "src/router.ts": "route/tab navigation continuations; display only",
  "src/deepLinkNavigation.ts": "deep-link navigation; display only",
};

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sources(path);
    return /\.(ts|tsx)$/.test(name) && !/\.test\.tsx?$/.test(name) ? [path] : [];
  });
}

export function displayEpochCallers(files: Record<string, string>): string[] {
  return Object.entries(files)
    .filter(([file, text]) => /\bstillBound\(/.test(text) && !(file in DISPLAY_ONLY))
    .map(([file]) => file);
}

describe("display epoch never owns a write (R4 / I-20)", () => {
  it("only display-only modules call stillBound", () => {
    const files = Object.fromEntries(sources("src").map((file) => [file, readFileSync(file, "utf8")]));
    expect(displayEpochCallers(files), RULE).toEqual([]);
  });

  it("flags a planted write-side stillBound", () => {
    expect(displayEpochCallers({ "src/planted.ts": "if (stillBound(binding)) void writeOwned(owner, backend().savePages([]));" })).toEqual(["src/planted.ts"]);
  });

  it("the type system refuses a display owner at every write boundary", () => {
    const work = () => Promise.resolve(1);
    // @ts-expect-error R4: writeOwned takes a WriteOwner, never a DisplayOwner
    void writeOwned(graphOwner(), work());
    // @ts-expect-error R4: serializeDurable takes a WriteOwner
    void serializeDurable({}, graphOwner(), work);
    // @ts-expect-error R4: serializeOwned orders writes, so it takes a WriteOwner
    void serializeOwned({}, graphOwner(), work);
    void writeOwned(bindingOwner(), work());
    void writeOwned(() => true, work());
    expect(RULE).toContain("bindingOwner");
  });
});
