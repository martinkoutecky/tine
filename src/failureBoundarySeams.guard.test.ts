import { readFileSync } from "node:fs";
import path from "node:path";
import ts from "typescript";
import { describe, expect, it } from "vitest";

// Which regions must be able to fail alone.
//
// Solid throws the WHOLE pending effect queue away when a render throws
// (`runUpdates`, solid-js dist/solid.js:820 — `if (!wait) Effects = null;`) and
// then RETHROWS when no boundary is registered. Tine registered none, so one
// unreadable value blanked the entire window silently and did not recover:
// GH #490 found it in the conflict panel, GH #332 is a user who has been on an
// old release since August because of it.
//
// A boundary is therefore an architectural fact, not a decoration, and facts
// live in tests. Each entry below is a mount site whose failure must cost the
// user that region and nothing more. Adding a seam is welcome; removing one
// means arguing that a throw there should take the app with it.
const REQUIRED_SEAMS: Array<{ file: string; component: string }> = [
  { file: "src/App.tsx", component: "PaneContent" },
  { file: "src/App.tsx", component: "Sidebar" },
  { file: "src/components/Page.tsx", component: "LinkedReferences" },
  { file: "src/components/Page.tsx", component: "UnlinkedReferences" },
  { file: "src/components/Page.tsx", component: "PageConflictResolution" },
  { file: "src/components/RightSidebar.tsx", component: "LinkedReferences" },
  { file: "src/components/RightSidebar.tsx", component: "UnlinkedReferences" },
];

const BOUNDARY = "FailureBoundary";
const REPO_ROOT = path.resolve(__dirname, "..");

function parse(file: string, source: string): ts.SourceFile {
  return ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
}

function tagName(node: ts.Node): string | null {
  if (ts.isJsxSelfClosingElement(node)) return node.tagName.getText();
  if (ts.isJsxElement(node)) return node.openingElement.tagName.getText();
  return null;
}

/**
 * Mount sites of `component` in `source` that no enclosing element wraps in a
 * FailureBoundary. AST, not text: a comment or a doc string naming the
 * component is prose and must stay legal, and the first thing a text scan
 * catches is the comment explaining the rule.
 */
export function unboundedMountSites(file: string, source: string, component: string): number[] {
  const sourceFile = parse(file, source);
  const offending: number[] = [];
  const visit = (node: ts.Node) => {
    if (tagName(node) === component) {
      let guarded = false;
      for (let parent = node.parent; parent; parent = parent.parent) {
        if (tagName(parent) === BOUNDARY) {
          guarded = true;
          break;
        }
      }
      if (!guarded) {
        offending.push(sourceFile.getLineAndCharacterOfPosition(node.getStart(sourceFile)).line + 1);
      }
    }
    ts.forEachChild(node, visit);
  };
  ts.forEachChild(sourceFile, visit);
  return offending;
}

describe("failure-boundary seams (GH #490/#332)", () => {
  for (const seam of REQUIRED_SEAMS) {
    it(`wraps every <${seam.component}> in ${seam.file}`, () => {
      const source = readFileSync(path.join(REPO_ROOT, seam.file), "utf8");
      const mounts = unboundedMountSites(seam.file, source, seam.component);
      expect(
        mounts,
        `<${seam.component}> is mounted outside a <FailureBoundary> at ${seam.file}:${mounts.join(", ")}. `
          + "A throw there blanks the whole window silently (solid-js runUpdates discards the effect "
          + "queue and rethrows with no boundary registered). Wrap it, or argue in the packet why this "
          + "region may take the app down with it. Exemplar: src/components/Page.tsx, Unlinked References.",
      ).toEqual([]);
    });
  }

  it("is not vacuous: it reports an unwrapped mount", () => {
    expect(unboundedMountSites("x.tsx", "const a = () => <LinkedReferences name={n} />;", "LinkedReferences"))
      .toEqual([1]);
  });

  it("accepts a wrapped mount", () => {
    const wrapped = 'const a = () => <FailureBoundary region="R"><LinkedReferences name={n} /></FailureBoundary>;';
    expect(unboundedMountSites("x.tsx", wrapped, "LinkedReferences")).toEqual([]);
  });

  it("leaves prose alone: a comment naming the component is not a mount", () => {
    const prose = "// Rendering <LinkedReferences /> is what this file does.\nconst a = 1;\n";
    expect(unboundedMountSites("x.tsx", prose, "LinkedReferences")).toEqual([]);
  });
});
