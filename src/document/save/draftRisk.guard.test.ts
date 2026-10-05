// Guard (og storage.qnt guarantee B, mutant MS): ONLY A MATCHING-VERSION
// PUBLISHED REPLY RETIRES RISK. A page at risk keeps its crash-surviving draft
// until a Published reply covers the exact buffer version it holds now
// (`notePublished`), or the user replaced the buffer with disk bytes
// (`noteBufferOnDisk`), or the buffer left the working set (`forgetSaveState`).
// Every other way of ending a conflict or a save failure leaves the risk held.
// Threat: crash / power loss while newer typed text exists only in memory after
// an older save landed (conformance map GAP-1, GAP-2). Behaviour is proved in
// src/draftRisk.test.ts; this scan stops a new retirement site from appearing.
// Exemplar: the success branch of `doSave` (`notePublished(name, covered)`).
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("./engine.ts", import.meta.url), "utf8");

/** Top-level function name → its source text (up to the next top-level item). */
function functions(): Map<string, string> {
  const out = new Map<string, string>();
  const heads = [...source.matchAll(/^(?:export )?(?:async )?function (\w+)/gm)];
  heads.forEach((head, i) => {
    out.set(head[1], source.slice(head.index!, heads[i + 1]?.index ?? source.length));
  });
  return out;
}

function sitesOf(pattern: RegExp): string[] {
  const sites: string[] = [];
  for (const [name, body] of functions()) for (const _ of body.matchAll(pattern)) sites.push(name);
  return sites.sort();
}

const RULE = "only a matching-version Published reply retires risk (storage.qnt guarantee B, mutant MS). "
  + "Call notePublished(name, coveredVersion) with the bufferVersion captured at the save's snapshot, as doSave does.";

describe("draft-risk retirement front door", () => {
  it("risk leaves riskHeld only through the version check, a rename move, or the buffer leaving", () => {
    expect(sitesOf(/riskHeld\.(delete|clear)\(/g), RULE).toEqual(["forgetSaveState", "noteRisk", "rekeyPageSaveState", "resetSaveState"]);
    expect(functions().get("noteRisk"), RULE).toMatch(/publishedVersions\.get\(name\) === bufferVersion\(name\)\) riskHeld\.delete\(name\)/);
  });

  it("only a Published reply or a user's disk choice records a covered version", () => {
    expect(sitesOf(/publishedVersions\.set\(/g), RULE).toEqual(["noteBufferOnDisk", "notePublished", "rekeyPageSaveState"]);
    expect(sitesOf(/noteBufferOnDisk\(/g).filter((name) => name !== "noteBufferOnDisk"), RULE)
      .toEqual(["installLiveResolution", "resolveConflict"]);
  });

  it("every save success path reports its covered version", () => {
    // createPage, doSave and runGroup are the three Published replies.
    expect(sitesOf(/notePublished\(name, cover/g), RULE).toEqual(["createPage", "doSave", "runGroup"]);
    expect(sitesOf(/forgetSaveFailure\(/g).filter((name) => name !== "forgetSaveFailure"), RULE)
      .toEqual(["forgetSaveState", "installLiveResolution", "notePublished", "rekeyPageSaveState", "reportSaveFailure", "resetSaveState"]);
  });

  it("the draft keeper hears a page's risk from noteRisk alone (plus the rename hand-over)", () => {
    expect(sitesOf(/draftKeeper\?\.\(/g), RULE).toEqual(["noteRisk", "rekeyPageSaveState"]);
  });

  it("every user edit moves the buffer version", () => {
    expect(sitesOf(/bufferVersions\.set\(name, \+\+bufferClock\)/g), RULE).toEqual(["addDirty", "markDirty"]);
  });
});
