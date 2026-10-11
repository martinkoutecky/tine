// J6, second round (STEP3-DESIGN "Manager decisions on REVIEW-3b-P2b"): a
// version the host took from this window stays the window's until a TRUE
// publication answer. That is one client predicate, `HostClient.owes`, and
// every path that lets go of a loaded page consults it: the close
// (`maybeClose`), working-set eviction (`unsettledPages`), the recovery panel
// and close prompt (`unsavedDrafts`), and reload blocking (`hostBlocksReload`).
// A new path that decides on a page's unanswered work (`busy`) is a candidate
// let-go path: classify it here. Exemplar: `unsettledPages` in wiring.ts.

import { readFileSync } from "node:fs";
import ts from "typescript";
import { expect, it } from "vitest";

type Bodies = Map<string, string>;

/** Top-level functions and class methods of `file`, by name, with their source text. */
function bodies(file: string): Bodies {
  const source = ts.createSourceFile(file, readFileSync(file, "utf8"), ts.ScriptTarget.Latest, true);
  const found: Bodies = new Map();
  const visit = (node: ts.Node) => {
    if ((ts.isFunctionDeclaration(node) || ts.isMethodDeclaration(node)) && node.name && node.body)
      found.set(node.name.getText(source), node.body.getText(source));
    ts.forEachChild(node, visit);
  };
  visit(source);
  return found;
}

/** The let-go paths: each must consult the publication-debt predicate. */
const LET_GO = {
  "src/document/host/client.ts": { maybeClose: "this.owing(page)" },
  "src/document/host/wiring.ts": {
    unsettledPages: ".owes(name)",
    unsavedDrafts: ".owes(name)",
    hostBlocksReload: ".owes(name)",
    // The change signal those listings recompute on.
    hostChanged: ".owes(name)",
  },
} as const;

/** Functions that read `busy` but let nothing go. */
const NOT_LET_GO: Record<string, string> = {
  isSaving: "the unanswered-request state a view shows",
  hostHolds: "routes a page's disk changes to host mail; a page in debt stays open",
  landAliasDraft: "a precondition for writing into the alias owner",
};

/** Client calls that drop a page, each for a reason that ends the debt. */
const DROPS: Record<string, string> = {
  rebind: "a new session",
  retire: "the client's retirement",
  answer: "the host released the page (an Open whose read failed)",
  admitted: "an admitted close, sent only once the debt is discharged",
  maybeClose: "a page the host does not hold",
};

it("every let-go path consults the publication-debt predicate (J6, REVIEW-3b-P2b)", () => {
  const bad: string[] = [];
  for (const [file, paths] of Object.entries(LET_GO)) {
    const found = bodies(file);
    for (const [name, call] of Object.entries(paths)) {
      if (!found.get(name)?.includes(call))
        bad.push(`J6: ${file} ${name} lets go of a page without the publication-debt predicate (${call}); exemplar unsettledPages`);
    }
  }
  const close = bodies("src/document/host/client.ts").get("maybeClose") ?? "";
  if (close.indexOf("this.owing(page)") < 0 || close.indexOf("this.owing(page)") > close.indexOf("this.host.close("))
    bad.push("J6: maybeClose must check the publication debt before it sends the close");
  const wiring = bodies("src/document/host/wiring.ts");
  for (const [name, body] of wiring) {
    if (!/\.busy\(/.test(body) || name in LET_GO["src/document/host/wiring.ts"] || name in NOT_LET_GO) continue;
    bad.push(`J6: wiring.ts ${name} decides on a page's unanswered work: classify it in publicationDebt.guard.test.ts `
      + "(a let-go path consults .owes(name))");
  }
  for (const [name, body] of bodies("src/document/host/client.ts")) {
    if (name !== "drop" && body.includes("this.drop(") && !(name in DROPS))
      bad.push(`J6: client.ts ${name} drops a page: name the reason that ends its publication debt here`);
  }
  expect(bad).toEqual([]);
});
